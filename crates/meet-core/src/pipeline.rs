//! Da gravação pronta ao resultado: transcrição (Groq) e separação de vozes (local) rodam ao mesmo
//! tempo; depois junta os dois, nomeia as vozes salvas, gera a ata e grava tudo no banco.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::audio::decode::{load_16k, write_opus_mono_16k, Channel};
use crate::clock::now_sp;
use crate::db::{Db, Meeting, Segment, Speaker, Stage};
use crate::groq::{self, Groq};
use crate::minutes::{self, transcript_text};
use crate::paths::meeting_dir;
use crate::transcribe;
use crate::voice::analyze::{analyze, identify_clusters, AnalyzeOptions};
use crate::voice::sherpa::SherpaPaths;
use crate::voice::store::VoiceStore;

#[derive(Debug, Clone)]
pub struct ProcessConfig {
    pub groq_key: String,
    pub chat_model: String,
    pub language: String,
    pub analyze: AnalyzeOptions,
}

impl ProcessConfig {
    pub fn new(groq_key: String) -> Self {
        Self { groq_key, chat_model: groq::CHAT_MODEL.into(), language: "pt".into(), analyze: AnalyzeOptions::default() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    Loading,
    /// Blocos de áudio já transcritos / total.
    Transcribing(usize, usize),
    Minutes(usize, usize),
    Saving,
}

/// Cria a reunião e a pasta dela; o áudio vai em `<pasta>/audio.opus`.
pub fn new_meeting(db: &Db, mode: &str, title: Option<&str>, num_speakers: Option<u32>) -> Result<Meeting> {
    let now = now_sp();
    let base = now.format("%Y%m%d-%H%M%S").to_string();
    let mut id = base.clone();
    let mut n = 1;
    while meeting_dir(&id).exists() {
        n += 1;
        id = format!("{base}-{n}");
    }
    let dir = meeting_dir(&id);
    std::fs::create_dir_all(&dir).with_context(|| format!("criar {}", dir.display()))?;
    let m = Meeting {
        title: title.map(str::to_string).unwrap_or_else(|| format!("Reunião de {}", now.format("%d/%m/%Y %H:%M"))),
        started_at: now.to_rfc3339_opts(chrono::SecondsFormat::Secs, false),
        duration_secs: 0.0,
        audio_path: Some(dir.join("audio.opus").to_string_lossy().into_owned()),
        mode: mode.into(),
        num_speakers,
        stage: Stage::Recording,
        error: None,
        id,
    };
    db.create_meeting(&m)?;
    Ok(m)
}

/// Traz um áudio de fora (`.opus`, `.wav`, `.m4a`, `.mp3`, `.flac`): vira a reunião pronta para processar.
pub fn import(db: &Db, src: &Path, mode: &str, title: Option<&str>, num_speakers: Option<u32>) -> Result<Meeting> {
    let default_title = src.file_stem().map(|s| s.to_string_lossy().into_owned());
    let m = new_meeting(db, mode, title.or(default_title.as_deref()), num_speakers)?;
    let dest = meeting_dir(&m.id).join("audio.opus");
    let imported = (|| -> Result<f64> {
        let pcm = load_16k(src, Channel::Mix)?;
        write_opus_mono_16k(&dest, &pcm, transcribe::OPUS_BITRATE)?;
        Ok(pcm.len() as f64 / crate::audio::decode::PROC_RATE as f64)
    })();
    match imported {
        Ok(secs) => {
            db.finish_recording(&m.id, secs, &dest.to_string_lossy())?;
            db.meeting(&m.id)
        }
        Err(e) => {
            let _ = db.delete_meeting(&m.id);
            let _ = std::fs::remove_dir_all(meeting_dir(&m.id));
            Err(e).with_context(|| format!("importar {}", src.display()))
        }
    }
}

/// Processa uma reunião gravada. Falha na transcrição ou nas vozes deixa a reunião em `Failed`
/// (o áudio fica; dá para tentar de novo). Falha só na ata mantém a transcrição.
pub fn process(db: &mut Db, id: &str, cfg: &ProcessConfig, voices: &VoiceStore, sherpa: &SherpaPaths, mut on_step: impl FnMut(Step)) -> Result<()> {
    db.set_stage(id, Stage::Processing, None)?;
    match run(db, id, cfg, voices, sherpa, &mut on_step) {
        Ok(minutes_error) => {
            db.set_stage(id, Stage::Done, minutes_error.as_deref())?;
            Ok(())
        }
        Err(e) => {
            let _ = db.set_stage(id, Stage::Failed, Some(&format!("{e:#}")));
            Err(e)
        }
    }
}

fn run(db: &mut Db, id: &str, cfg: &ProcessConfig, voices: &VoiceStore, sherpa: &SherpaPaths, on_step: &mut impl FnMut(Step)) -> Result<Option<String>> {
    let meeting = db.meeting(id)?;
    let Some(audio) = meeting.audio_path.as_deref().map(PathBuf::from).filter(|p| p.exists()) else {
        bail!("o áudio desta reunião não está mais no disco");
    };
    sherpa.check()?;
    on_step(Step::Loading);
    let pcm = load_16k(&audio, Channel::Mix)?;
    let mut opt = cfg.analyze.clone();
    opt.num_speakers = meeting.num_speakers.or(opt.num_speakers);
    let groq = Groq::new(&cfg.groq_key);

    // a Groq responde pela rede e a análise usa a CPU: uma não espera a outra
    let (stt, analysis) = std::thread::scope(|s| {
        let local = s.spawn(|| analyze(sherpa, &pcm, &opt, None));
        let remote = transcribe::transcribe(&groq, &pcm, &cfg.language, |d, t| on_step(Step::Transcribing(d, t)));
        (remote, local.join().unwrap_or_else(|_| bail!("a análise de vozes falhou de forma inesperada")))
    });
    drop(pcm);
    let (stt, mut analysis) = (stt.context("transcrição")?, analysis.context("separação de vozes")?);
    identify_clusters(&mut analysis.clusters, voices, opt.voice_threshold)?;

    let segments: Vec<Segment> = transcribe::attribute(&stt, &analysis.spans);
    let speakers: Vec<Speaker> = analysis
        .clusters
        .iter()
        .map(|c| Speaker { cluster: c.cluster, name: c.name.clone(), voice_id: c.voice_id, seconds: c.seconds, embedding: c.embedding.clone() })
        .collect();
    on_step(Step::Saving);
    db.replace_transcript(id, &segments, &speakers)?;

    Ok(make_minutes(db, id, &groq, &cfg.chat_model, &segments, &speakers, on_step).err().map(|e| format!("ata não gerada: {e:#}")))
}

fn make_minutes(db: &mut Db, id: &str, groq: &Groq, model: &str, segments: &[Segment], speakers: &[Speaker], on_step: &mut impl FnMut(Step)) -> Result<()> {
    let text = transcript_text(segments, speakers);
    let m = minutes::generate(groq, model, &text, |d, t| on_step(Step::Minutes(d, t)))?;
    db.save_minutes(id, &m)
}

/// Gera de novo só a ata (por exemplo, depois de corrigir nomes de falantes ou de a Groq ter falhado).
pub fn regenerate_minutes(db: &mut Db, id: &str, cfg: &ProcessConfig, mut on_step: impl FnMut(Step)) -> Result<()> {
    let (segments, speakers) = (db.segments(id)?, db.speakers(id)?);
    make_minutes(db, id, &Groq::new(&cfg.groq_key), &cfg.chat_model, &segments, &speakers, &mut on_step)?;
    // só limpa o aviso "ata não gerada"; numa reunião que falhou, o motivo da falha continua valendo
    let m = db.meeting(id)?;
    if m.stage == Stage::Done {
        db.set_stage(id, Stage::Done, None)?;
    }
    Ok(())
}

/// Dá nome a um falante da reunião e, se pedido, guarda a voz para reconhecê-lo nas próximas.
/// Se o nome já é de uma voz salva, reforça essa voz em vez de criar outra.
pub fn name_speaker(db: &Db, voices: &VoiceStore, id: &str, cluster: i64, name: &str, remember: bool) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        bail!("digite um nome");
    }
    let sp = db.speakers(id)?.into_iter().find(|s| s.cluster == cluster).with_context(|| format!("falante {cluster} não existe"))?;
    let mut voice_id = sp.voice_id;
    if remember {
        if sp.embedding.is_empty() {
            bail!("este falante falou pouco demais para guardar a voz");
        }
        let existing = voices.list()?.into_iter().find(|v| v.name.eq_ignore_ascii_case(name));
        voice_id = Some(match existing {
            Some(v) => {
                voices.reinforce(v.id, &sp.embedding, sp.seconds as f32)?;
                v.id
            }
            None => voices.add(name, &sp.embedding, sp.seconds as f32, false)?,
        });
    }
    db.rename_speaker(id, cluster, Some(name), voice_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Segment;

    fn with_data_dir<T>(name: &str, f: impl FnOnce(&Path) -> T) -> T {
        // as pastas vêm de variável de ambiente: um teste por vez mexe nela
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("rm-pipe-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("RENANMEET_DATA_DIR", &dir);
        let r = f(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        r
    }

    #[test]
    fn importa_m4a_vira_opus_e_reuniao_pronta() {
        with_data_dir("import", |_| {
            let db = Db::memory().unwrap();
            let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tom-440hz-2s.m4a");
            let m = import(&db, &src, "presencial", None, Some(3)).unwrap();
            assert_eq!((m.stage, m.mode.as_str(), m.num_speakers, m.title.as_str()), (Stage::Recorded, "presencial", Some(3), "tom-440hz-2s"));
            assert!((m.duration_secs - 2.3).abs() < 0.2);
            let audio = PathBuf::from(m.audio_path.unwrap());
            assert!(std::fs::read(&audio).unwrap().starts_with(b"OggS"));
            // arquivo ruim não deixa reunião nem pasta para trás
            let ruim = audio.with_file_name("lixo.m4a");
            std::fs::write(&ruim, b"nao e audio").unwrap();
            assert!(import(&db, &ruim, "online", None, None).is_err());
            assert_eq!(db.meetings().unwrap().len(), 1);
        });
    }

    #[test]
    fn nomear_falante_guarda_a_voz_e_reforca_em_vez_de_duplicar() {
        with_data_dir("nomear", |_| {
            let mut db = Db::memory().unwrap();
            let voices = VoiceStore::memory().unwrap();
            let m = new_meeting(&db, "online", Some("R"), None).unwrap();
            let sp = |c, e: Vec<f32>| Speaker { cluster: c, name: None, voice_id: None, seconds: 30.0, embedding: e };
            let seg = vec![Segment { start: 0.0, end: 1.0, text: "oi".into(), cluster: Some(0) }];
            db.replace_transcript(&m.id, &seg, &[sp(0, vec![1.0, 0.0]), sp(1, vec![])]).unwrap();
            name_speaker(&db, &voices, &m.id, 0, " Ana ", true).unwrap();
            name_speaker(&db, &voices, &m.id, 0, "ana", true).unwrap();
            assert_eq!(voices.list().unwrap().len(), 1, "mesmo nome reforça a voz");
            let s = db.speakers(&m.id).unwrap().into_iter().find(|s| s.cluster == 0).unwrap();
            assert_eq!((s.name.as_deref(), s.voice_id.is_some()), (Some("ana"), true));
            assert!(name_speaker(&db, &voices, &m.id, 1, "Caio", true).is_err(), "sem impressão não guarda voz");
            name_speaker(&db, &voices, &m.id, 1, "Caio", false).unwrap();
            assert_eq!(voices.list().unwrap().len(), 1);
            assert!(name_speaker(&db, &voices, &m.id, 0, "  ", false).is_err());
        });
    }
}
