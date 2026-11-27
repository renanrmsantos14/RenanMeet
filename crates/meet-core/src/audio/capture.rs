//! Fontes de captura: entregam quadros de 20 ms, mono, i16, 48 kHz.
//!
//! - Linux: `parec` (PulseAudio/PipeWire-pulse) em subprocesso. Não exige bibliotecas
//!   de desenvolvimento, o servidor de som reamostra, e o processo morre com o app.
//! - Windows: WASAPI via `cpal` (microfone) e loopback (áudio do sistema) — ver `capture_win.rs`.

use std::time::Duration;


pub enum Recv {
    Frame(Vec<i16>),
    Timeout,
    Closed,
}

pub trait FrameSource: Send {
    fn recv(&mut self, timeout: Duration) -> Recv;
    /// Pede para parar; `recv` devolve o que sobrou e depois `Closed`.
    fn stop(&mut self);
}

pub const DEFAULT_MIC: &str = "@DEFAULT_SOURCE@";
pub const DEFAULT_SYSTEM: &str = "@DEFAULT_MONITOR@";

pub fn peak(frame: &[i16]) -> f32 {
    frame.iter().map(|s| (*s as i32).unsigned_abs()).max().unwrap_or(0) as f32 / 32768.0
}

#[cfg(target_os = "linux")]
pub mod linux {
    use std::io::Read;
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
    use std::thread::JoinHandle;

    use anyhow::{bail, Context, Result};
    use crate::audio::opus::FRAME;

    use super::*;

    pub struct ParecSource {
        child: Child,
        rx: Receiver<Vec<i16>>,
        reader: Option<JoinHandle<()>>,
    }

    fn spawn(bin: &str, device: &str) -> std::io::Result<Child> {
        Command::new(bin)
            .args([
                "--raw",
                "--format=s16le",
                "--rate=48000",
                "--channels=1",
                "--latency-msec=20",
                "--client-name=RenanMeet",
                &format!("--device={device}"),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
    }

    impl ParecSource {
        pub fn open(device: &str) -> Result<Self> {
            let mut child = match spawn("parec", device) {
                Ok(c) => c,
                Err(_) => spawn("parecord", device)
                    .context("não achei `parec` (instale pulseaudio-utils / pipewire-pulse)")?,
            };
            let mut out = child.stdout.take().context("parec sem stdout")?;
            let (tx, rx): (SyncSender<Vec<i16>>, _) = sync_channel(400); // 8 s de folga
            let reader = std::thread::Builder::new()
                .name("parec-reader".into())
                .spawn(move || {
                    let mut buf = vec![0u8; FRAME * 2];
                    while out.read_exact(&mut buf).is_ok() {
                        let frame: Vec<i16> = buf
                            .chunks_exact(2)
                            .map(|b| i16::from_le_bytes([b[0], b[1]]))
                            .collect();
                        match tx.try_send(frame) {
                            Ok(()) => {}
                            Err(TrySendError::Full(_)) => {} // consumidor travado: descarta
                            Err(TrySendError::Disconnected(_)) => break,
                        }
                    }
                })?;
            // Se o parec morrer logo (dispositivo inexistente), avisa cedo.
            std::thread::sleep(Duration::from_millis(120));
            if let Ok(Some(status)) = child.try_wait() {
                bail!("parec saiu ({status}) — dispositivo `{device}` inválido?");
            }
            Ok(Self {
                child,
                rx,
                reader: Some(reader),
            })
        }
    }

    impl FrameSource for ParecSource {
        fn recv(&mut self, timeout: Duration) -> Recv {
            match self.rx.recv_timeout(timeout) {
                Ok(f) => Recv::Frame(f),
                Err(RecvTimeoutError::Timeout) => Recv::Timeout,
                Err(RecvTimeoutError::Disconnected) => Recv::Closed,
            }
        }

        fn stop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    impl Drop for ParecSource {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            if let Some(h) = self.reader.take() {
                let _ = h.join();
            }
        }
    }
}

#[cfg(target_os = "linux")]
pub fn open_mic(device: Option<&str>) -> anyhow::Result<Box<dyn FrameSource>> {
    Ok(Box::new(linux::ParecSource::open(device.unwrap_or(DEFAULT_MIC))?))
}

#[cfg(target_os = "linux")]
pub fn open_system(device: Option<&str>) -> anyhow::Result<Box<dyn FrameSource>> {
    Ok(Box::new(linux::ParecSource::open(device.unwrap_or(DEFAULT_SYSTEM))?))
}

#[cfg(windows)]
#[path = "capture_win.rs"]
mod win;
#[cfg(windows)]
pub use win::{open_mic, open_system};

#[cfg(not(any(target_os = "linux", windows)))]
pub fn open_mic(_: Option<&str>) -> anyhow::Result<Box<dyn FrameSource>> {
    anyhow::bail!("captura não suportada nesta plataforma")
}
#[cfg(not(any(target_os = "linux", windows)))]
pub fn open_system(_: Option<&str>) -> anyhow::Result<Box<dyn FrameSource>> {
    anyhow::bail!("captura não suportada nesta plataforma")
}
