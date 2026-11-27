//! Ferramenta de linha de comando para desenvolvimento e testes do núcleo.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Result};
use meet_core::audio::decode::{load_16k, write_wav_16k, Channel};
use meet_core::audio::recorder::{Mode, Recorder, RecorderConfig};

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
        _ => bail!("comandos: record, wav"),
    }
    Ok(())
}
