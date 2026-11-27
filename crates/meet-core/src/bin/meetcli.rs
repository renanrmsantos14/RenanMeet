//! Ferramenta de linha de comando para desenvolvimento e testes do núcleo.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Result};
use meet_core::audio::decode::{load_16k, write_wav_16k, Channel};
use meet_core::audio::recorder::{Mode, Recorder, RecorderConfig};
use meet_core::paths::db_path;
use meet_core::voice::analyze::{analyze, enroll_embedding, AnalyzeOptions, Preproc};
use meet_core::voice::sherpa::SherpaPaths;
use meet_core::voice::store::VoiceStore;

fn flag(args: &[String], nome: &str) -> Option<String> {
    args.iter().position(|a| a == nome).and_then(|i| args.get(i + 1).cloned())
}
fn has(args: &[String], nome: &str) -> bool {
    args.iter().any(|a| a == nome)
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
        _ => bail!("comandos: record, wav, diarize, enroll, voices"),
    }
    Ok(())
}
