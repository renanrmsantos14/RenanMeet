//! Ferramenta de linha de comando para desenvolvimento e testes do núcleo.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Result};
use meet_core::audio::decode::{load_16k, write_wav_16k, Channel};
use meet_core::audio::recorder::{Mode, Recorder, RecorderConfig};
use meet_core::db::{Db, Stage};
use meet_core::export::{markdown, send_to_wiki, wiki_dir, Full};
use meet_core::groq::Groq;
use meet_core::paths::{db_path, models_dir};
use meet_core::pipeline::{self, ProcessConfig, Step};
use meet_core::{models, secrets};
use meet_core::voice::analyze::{analyze, enroll_embedding, AnalyzeOptions, Preproc};
use meet_core::voice::sherpa::SherpaPaths;
use meet_core::voice::store::VoiceStore;

fn flag(args: &[String], nome: &str) -> Option<String> {
    args.iter().position(|a| a == nome).and_then(|i| args.get(i + 1).cloned())
}
fn has(args: &[String], nome: &str) -> bool {
    args.iter().any(|a| a == nome)
}

fn groq_key() -> Result<String> {
    secrets::groq_key()?.ok_or_else(|| anyhow::anyhow!("sem chave da Groq: rode `meetcli key set` (a chave é lida da entrada padrão)"))
}

/// Argumento posicional obrigatório: mensagem de uso em vez de pânico por índice.
fn arg<'a>(args: &'a [String], i: usize, uso: &str) -> Result<&'a str> {
    args.get(i).map(String::as_str).ok_or_else(|| anyhow::anyhow!("uso: meetcli {uso}"))
}

fn show_step(step: Step) {
    match step {
        Step::Loading => eprintln!("carregando áudio"),
        Step::Transcribing(d, t) => eprintln!("transcrevendo {d}/{t}"),
        Step::Minutes(d, t) => eprintln!("ata {d}/{t}"),
        Step::Saving => eprintln!("salvando"),
    }
}

fn with_full<T>(db: &Db, id: &str, f: impl FnOnce(&Full) -> Result<T>) -> Result<T> {
    let (meeting, speakers, segments, minutes) = (db.meeting(id)?, db.speakers(id)?, db.segments(id)?, db.minutes(id)?);
    f(&Full { meeting: &meeting, speakers: &speakers, segments: &segments, minutes: minutes.as_ref() })
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("record") => {
            // meetcli record <online|presencial> <segundos> <saida.opus>
            let mode = match args.get(1).map(String::as_str) {
                Some("online") => Mode::Online,
                Some("presencial") => Mode::InPerson,
                _ => bail!("uso: meetcli record <online|presencial> <segundos> <saida.opus>"),
            };
            let secs: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);
            let out = PathBuf::from(args.get(3).cloned().unwrap_or_else(|| "saida.opus".into()));
            let rec = Recorder::start(RecorderConfig {
                mode,
                out,
                mic_device: None,
                system_device: None,
            })?;
            for _ in 0..secs {
                std::thread::sleep(Duration::from_secs(1));
                let l = rec.levels();
                eprintln!(
                    "mic {:>4}‰  sistema {:>4}‰  quadros {}",
                    l.mic.load(std::sync::atomic::Ordering::Relaxed),
                    l.system.load(std::sync::atomic::Ordering::Relaxed),
                    l.frames.load(std::sync::atomic::Ordering::Relaxed)
                );
            }
            let st = rec.stop()?;
            println!("{}", serde_json::to_string_pretty(&st)?);
        }
        Some("wav") => {
            // meetcli wav <entrada> <saida.wav> [mix|esq|dir]  → 16 kHz mono
            let ch = match args.get(3).map(String::as_str) {
                Some("esq") => Channel::Left,
                Some("dir") => Channel::Right,
                _ => Channel::Mix,
            };
            let pcm = load_16k(&PathBuf::from(&args[1]), ch)?;
            write_wav_16k(&PathBuf::from(&args[2]), &pcm)?;
            println!("{} amostras ({:.1} s)", pcm.len(), pcm.len() as f64 / 16000.0);
        }
        Some("diarize") => {
            // meetcli diarize <audio> [--speakers N] [--denoise] [--agc] [--sem-refinar] [--dist X] [--threshold X] [--voz-limiar X] [--saida arq.json]
            let pcm = load_16k(&PathBuf::from(&args[1]), Channel::Mix)?;
            let mut opt = AnalyzeOptions::default();
            opt.num_speakers = flag(&args, "--speakers").and_then(|v| v.parse().ok());
            if let Some(v) = flag(&args, "--threshold").and_then(|v| v.parse().ok()) {
                opt.cluster_threshold = v;
            }
            if let Some(v) = flag(&args, "--voz-limiar").and_then(|v| v.parse().ok()) {
                opt.voice_threshold = v;
            }
            if has(&args, "--sem-refinar") {
                opt.refine_distance = None;
            } else if let Some(v) = flag(&args, "--dist").and_then(|v| v.parse().ok()) {
                opt.refine_distance = Some(v);
            }
            if has(&args, "--denoise") || has(&args, "--agc") {
                opt.preproc = Some(Preproc { denoise: has(&args, "--denoise"), agc: has(&args, "--agc") });
            }
            let store = VoiceStore::open(&db_path())?;
            let t0 = std::time::Instant::now();
            let a = analyze(&SherpaPaths::locate()?, &pcm, &opt, Some(&store))?;
            eprintln!("análise: {:.1} s para {:.0} s de áudio", t0.elapsed().as_secs_f64(), pcm.len() as f64 / 16000.0);
            let nome = |c: i64| {
                a.clusters.iter().find(|x| x.cluster == c).and_then(|x| x.name.clone()).unwrap_or_else(|| format!("Falante {}", c + 1))
            };
            let hip: Vec<_> = a.spans.iter().map(|s| serde_json::json!({"start": s.start, "end": s.end, "speaker": nome(s.cluster)})).collect();
            let out = serde_json::to_string_pretty(&hip)?;
            match flag(&args, "--saida") {
                Some(f) => std::fs::write(f, out)?,
                None => println!("{out}"),
            }
            // --nomear "0=Ana,1=Caio": salva a voz desses clusters para reconhecer depois
            if let Some(m) = flag(&args, "--nomear") {
                for par in m.split(',') {
                    if let Some((c, n)) = par.split_once('=') {
                        let c: i64 = c.trim().parse()?;
                        if let Some(info) = a.clusters.iter().find(|x| x.cluster == c) {
                            let id = store.add(n.trim(), &info.embedding, info.seconds as f32, false)?;
                            eprintln!("voz {id} salva: {} ({:.0} s)", n.trim(), info.seconds);
                        }
                    }
                }
            }
            for c in &a.clusters {
                eprintln!("cluster {} · {:.0} s · voz {:?} · cosseno {:?}", c.cluster, c.seconds, c.name, c.score);
            }
        }
        Some("enroll") => {
            // meetcli enroll <audio> <nome> [--dono]
            let pcm = load_16k(&PathBuf::from(&args[1]), Channel::Mix)?;
            let (emb, secs) = enroll_embedding(&SherpaPaths::locate()?, &pcm)?;
            let id = VoiceStore::open(&db_path())?.add(&args[2], &emb, secs, has(&args, "--dono"))?;
            println!("voz {id} salva: {} ({secs:.0} s de fala, {} dimensões)", args[2], emb.len());
        }
        Some("voices") => {
            for v in VoiceStore::open(&db_path())?.list()? {
                println!("{}\t{}\t{:.0} s{}", v.id, v.name, v.seconds, if v.is_owner { "\tdono" } else { "" });
            }
        }
        Some("models") => {
            // meetcli models  → baixa o que falta (com hash conferido)
            let root = models_dir();
            let mut last = String::new();
            models::ensure(&root, |label, done, total| {
                let pct = (done * 100).checked_div(total).unwrap_or(0);
                let line = format!("{label}: {pct}%");
                if line != last {
                    eprintln!("{line}");
                    last = line;
                }
            })?;
            println!("modelos prontos em {}", root.display());
        }
        Some("key") => match args.get(1).map(String::as_str) {
            Some("set") => {
                let mut key = String::new();
                std::io::stdin().read_line(&mut key)?;
                secrets::set_groq_key(key.trim())?;
                println!("chave guardada ({})", secrets::mask(key.trim()));
            }
            Some("check") => {
                Groq::new(&groq_key()?).check_key()?;
                println!("chave da Groq válida");
            }
            Some("clear") => secrets::clear_groq_key()?,
            _ => bail!("uso: meetcli key <set|check|clear>"),
        },
        Some("import") => {
            // meetcli import <audio> [--presencial] [--participantes N] [--titulo T]
            let db = Db::open(&db_path())?;
            let mode = if has(&args, "--presencial") { "presencial" } else { "online" };
            let n = flag(&args, "--participantes").and_then(|v| v.parse().ok());
            let m = pipeline::import(&db, &PathBuf::from(arg(&args, 1, "import <audio> [--presencial] [--participantes N] [--titulo T]")?), mode, flag(&args, "--titulo").as_deref(), n)?;
            println!("{}\t{}\t{:.0} s", m.id, m.title, m.duration_secs);
        }
        Some("process") => {
            // meetcli process <id>  (também refaz uma reunião que falhou)
            let mut db = Db::open(&db_path())?;
            let cfg = ProcessConfig::new(groq_key()?);
            let id = arg(&args, 1, "process <id>")?;
            pipeline::process(&mut db, id, &cfg, &VoiceStore::open(&db_path())?, &SherpaPaths::locate()?, show_step)?;
            let m = db.meeting(id)?;
            println!("{} · {}{}", m.id, m.stage.as_str(), m.error.map(|e| format!(" · {e}")).unwrap_or_default());
        }
        Some("minutes") => {
            // meetcli minutes <id>  → gera a ata de novo
            let mut db = Db::open(&db_path())?;
            pipeline::regenerate_minutes(&mut db, arg(&args, 1, "minutes <id>")?, &ProcessConfig::new(groq_key()?), show_step)?;
            println!("ata atualizada");
        }
        Some("name") => {
            // meetcli name <id> <n do falante (Falante 1 = 1)> <nome> [--guardar]
            let db = Db::open(&db_path())?;
            let uso = "name <id> <n do falante> <nome> [--guardar]";
            let n: i64 = arg(&args, 2, uso)?.parse()?;
            pipeline::name_speaker(&db, &VoiceStore::open(&db_path())?, arg(&args, 1, uso)?, n - 1, arg(&args, 3, uso)?, has(&args, "--guardar"))?;
        }
        Some("list") => {
            for m in Db::open(&db_path())?.meetings()? {
                println!("{}\t{}\t{}\t{:.0} s", m.id, m.stage.as_str(), m.title, m.duration_secs);
            }
        }
        Some("search") => {
            for h in Db::open(&db_path())?.search(&args[1..].join(" "), 20)? {
                println!("{}\t{}\t{}", h.meeting_id, h.title, h.snippet);
            }
        }
        Some("export") => {
            // meetcli export <id> [saida.md]
            let md = with_full(&Db::open(&db_path())?, arg(&args, 1, "export <id> [saida.md]")?, |f| Ok(markdown(f)))?;
            match args.get(2) {
                Some(out) => std::fs::write(out, md)?,
                None => print!("{md}"),
            }
        }
        Some("wiki") => {
            // meetcli wiki <id>  → nota (decisões e tarefas) na inbox da wiki
            let db = Db::open(&db_path())?;
            let id = arg(&args, 1, "wiki <id>")?;
            if db.meeting(id)?.stage != Stage::Done {
                bail!("a reunião ainda não foi processada");
            }
            let (path, warn) = with_full(&db, id, |f| send_to_wiki(f, &wiki_dir()))?;
            println!("{}", path.display());
            if let Some(w) = warn {
                eprintln!("aviso: {w}");
            }
        }
        _ => bail!("comandos: record, wav, diarize, enroll, voices, models, key, import, process, minutes, name, list, search, export, wiki"),
    }
    Ok(())
}
