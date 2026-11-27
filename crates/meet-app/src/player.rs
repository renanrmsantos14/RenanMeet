//! Player: o áudio vira PCM cru (16 kHz, mono, 16 bits) em disco e o sistema o toca a partir de qualquer ponto.
//! Nada fica na RAM: tocar de um ponto é abrir o arquivo no deslocamento certo.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use meet_core::audio::decode::{load_16k, Channel, PROC_RATE};

const BYTES_PER_SEC: u64 = PROC_RATE as u64 * 2;

pub struct Player {
    raw: Option<(PathBuf, f64)>,
    child: Option<Child>,
    started: Instant,
    offset: f64,
    pub duration: f64,
}

impl Player {
    pub fn new() -> Self {
        Self { raw: None, child: None, started: Instant::now(), offset: 0.0, duration: 0.0 }
    }

    /// Prepara o arquivo cru da reunião (na primeira vez que se toca).
    pub fn load(&mut self, audio: &Path, raw_path: PathBuf) -> Result<()> {
        self.stop();
        if self.raw.as_ref().map(|(p, _)| p) == Some(&raw_path) {
            return Ok(());
        }
        let pcm = load_16k(audio, Channel::Mix)?;
        let mut w = BufWriter::new(File::create(&raw_path).context("criar áudio temporário")?);
        for s in &pcm {
            w.write_all(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())?;
        }
        w.flush()?;
        self.duration = pcm.len() as f64 / PROC_RATE as f64;
        self.raw = Some((raw_path, self.duration));
        self.offset = 0.0;
        Ok(())
    }

    pub fn unload(&mut self) {
        self.stop();
        if let Some((p, _)) = self.raw.take() {
            let _ = std::fs::remove_file(p);
        }
        self.duration = 0.0;
        self.offset = 0.0;
    }

    pub fn playing(&mut self) -> bool {
        match self.child.as_mut().map(|c| c.try_wait()) {
            Some(Ok(None)) => true,
            Some(_) => {
                self.child = None;
                // acabou sozinho: volta ao fim
                self.offset = self.duration;
                false
            }
            None => false,
        }
    }

    pub fn position(&mut self) -> f64 {
        if self.playing() {
            (self.offset + self.started.elapsed().as_secs_f64()).min(self.duration)
        } else {
            self.offset.min(self.duration)
        }
    }

    pub fn play_from(&mut self, secs: f64) -> Result<()> {
        let (path, dur) = self.raw.clone().ok_or_else(|| anyhow!("áudio não carregado"))?;
        self.stop();
        let secs = if secs >= dur - 0.2 { 0.0 } else { secs.max(0.0) };
        let mut f = File::open(&path)?;
        f.seek(SeekFrom::Start((secs * BYTES_PER_SEC as f64) as u64 & !1))?;
        self.child = Some(spawn_player(f)?);
        self.offset = secs;
        self.started = Instant::now();
        Ok(())
    }

    pub fn pause(&mut self) {
        let p = self.position();
        self.stop();
        self.offset = p;
    }

    pub fn seek(&mut self, secs: f64) -> Result<()> {
        if self.playing() {
            self.play_from(secs)
        } else {
            self.offset = secs.clamp(0.0, self.duration);
            Ok(())
        }
    }

    fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.unload();
    }
}

#[cfg(target_os = "linux")]
fn spawn_player(f: File) -> Result<Child> {
    Command::new("paplay")
        .args(["--raw", "--rate=16000", "--channels=1", "--format=s16le"])
        .stdin(Stdio::from(f))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("tocar o áudio (paplay não encontrado)")
}

#[cfg(not(target_os = "linux"))]
fn spawn_player(_f: File) -> Result<Child> {
    Err(anyhow!("reprodução ainda não implementada neste sistema"))
}
