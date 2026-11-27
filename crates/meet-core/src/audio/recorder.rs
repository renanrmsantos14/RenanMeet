//! Gravador: lê quadros de 20 ms do microfone (e do áudio do sistema, no modo online),
//! intercala (L = microfone, R = sistema) e escreve Ogg Opus direto em disco.
//!
//! Gasto em gravação: um encoder Opus e dois subprocessos `parec`; nenhum buffer cresce.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use super::capture::{open_mic, open_system, peak, FrameSource, Recv};
use super::opus::{OpusFileWriter, FRAME};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Reunião online: microfone + áudio do sistema (arquivo estéreo: L mic, R sistema).
    Online,
    /// Reunião presencial: só o microfone (arquivo mono). Todos falam no mesmo mic.
    InPerson,
}

#[derive(Debug, Clone)]
pub struct RecorderConfig {
    pub mode: Mode,
    pub out: PathBuf,
    pub mic_device: Option<String>,
    pub system_device: Option<String>,
}

#[derive(Default)]
pub struct Levels {
    /// Pico do último quadro, em milésimos (0..=1000).
    pub mic: AtomicU32,
    pub system: AtomicU32,
    pub frames: AtomicU64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingStats {
    pub seconds: f64,
    pub bytes: u64,
    pub channels: usize,
    /// Quadros em que uma fonte não entregou nada a tempo (preenchidos com silêncio).
    pub gaps_mic: u64,
    pub gaps_system: u64,
}

pub struct Recorder {
    stop: Arc<AtomicBool>,
    levels: Arc<Levels>,
    join: Option<JoinHandle<Result<RecordingStats>>>,
    pub started: Instant,
}

impl Recorder {
    pub fn start(cfg: RecorderConfig) -> Result<Self> {
        let mut mic = open_mic(cfg.mic_device.as_deref())?;
        let sys = if cfg.mode == Mode::Online {
            match open_system(cfg.system_device.as_deref()) {
                Ok(s) => Some(s),
                Err(e) => {
                    mic.stop();
                    return Err(e.context("áudio do sistema indisponível"));
                }
            }
        } else {
            None
        };
        if let Some(dir) = cfg.out.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let channels = if sys.is_some() { 2 } else { 1 };
        let bitrate = if channels == 2 { 32_000 } else { 24_000 };
        let writer = OpusFileWriter::create(&cfg.out, channels, bitrate)?;
        let stop = Arc::new(AtomicBool::new(false));
        let levels = Arc::new(Levels::default());
        let (s2, l2) = (stop.clone(), levels.clone());
        let join = std::thread::Builder::new()
            .name("recorder".into())
            .spawn(move || run(mic, sys, writer, s2, l2))?;
        Ok(Self {
            stop,
            levels,
            join: Some(join),
            started: Instant::now(),
        })
    }

    pub fn levels(&self) -> &Levels {
        &self.levels
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn stop(mut self) -> Result<RecordingStats> {
        self.stop.store(true, Ordering::SeqCst);
        self.join
            .take()
            .ok_or_else(|| anyhow!("já parado"))?
            .join()
            .map_err(|_| anyhow!("thread do gravador falhou"))?
    }
}

fn run(
    mut mic: Box<dyn FrameSource>,
    mut sys: Option<Box<dyn FrameSource>>,
    mut writer: OpusFileWriter,
    stop: Arc<AtomicBool>,
    levels: Arc<Levels>,
) -> Result<RecordingStats> {
    let silent = vec![0i16; FRAME];
    let (mut gaps_mic, mut gaps_sys) = (0u64, 0u64);
    let (mut mic_open, mut sys_open) = (true, sys.is_some());
    let mut stopping = false;
    let mut buf: Vec<i16> = Vec::with_capacity(FRAME * 2);
    while mic_open || sys_open {
        if !stopping && stop.load(Ordering::Relaxed) {
            stopping = true;
            mic.stop();
            if let Some(s) = sys.as_mut() {
                s.stop();
            }
        }
        // O microfone dá o ritmo; o sistema entra com tolerância curta.
        let m = if mic_open {
            match mic.recv(Duration::from_millis(100)) {
                Recv::Frame(f) => Some(f),
                Recv::Timeout => {
                    gaps_mic += 1;
                    None
                }
                Recv::Closed => {
                    mic_open = false;
                    None
                }
            }
        } else {
            None
        };
        let s = if sys_open {
            match sys.as_mut().unwrap().recv(Duration::from_millis(if mic_open { 30 } else { 100 })) {
                Recv::Frame(f) => Some(f),
                Recv::Timeout => {
                    gaps_sys += 1;
                    None
                }
                Recv::Closed => {
                    sys_open = false;
                    None
                }
            }
        } else {
            None
        };
        if m.is_none() && s.is_none() && !(mic_open || sys_open) {
            break;
        }
        if m.is_none() && s.is_none() && stopping {
            continue;
        }
        let mf = m.as_deref().unwrap_or(&silent);
        levels.mic.store((peak(mf) * 1000.0) as u32, Ordering::Relaxed);
        buf.clear();
        if sys.is_some() {
            let sf = s.as_deref().unwrap_or(&silent);
            levels.system.store((peak(sf) * 1000.0) as u32, Ordering::Relaxed);
            for i in 0..FRAME {
                buf.push(mf[i]);
                buf.push(sf[i]);
            }
        } else {
            buf.extend_from_slice(mf);
        }
        writer.write(&buf)?;
        levels.frames.fetch_add(1, Ordering::Relaxed);
    }
    let channels = writer.channels();
    let seconds = writer.seconds();
    let bytes = writer.finish()?;
    Ok(RecordingStats {
        seconds,
        bytes,
        channels,
        gaps_mic,
        gaps_system: gaps_sys,
    })
}
