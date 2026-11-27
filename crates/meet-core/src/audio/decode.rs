//! Leitura de áudio para processamento: tudo vira PCM f32 mono a 16 kHz.

use std::path::Path;

use anyhow::{bail, Result};

use super::opus::{decode_file, RATE};
use super::resample::Resampler;

pub const PROC_RATE: u32 = 16_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// Média dos canais (o que a diarização usa).
    Mix,
    Left,
    Right,
}

/// Decodifica Ogg Opus em fluxo e já entrega a 16 kHz mono (sem guardar 48 kHz).
pub fn load_opus_16k(path: &Path, ch: Channel) -> Result<Vec<f32>> {
    let mut rs = Resampler::new(RATE, PROC_RATE);
    let mut out: Vec<f32> = Vec::new();
    let mut mono: Vec<f32> = Vec::new();
    decode_file(path, |block, channels| {
        mono.clear();
        if channels == 1 {
            mono.extend_from_slice(block);
        } else {
            for fr in block.chunks_exact(2) {
                mono.push(match ch {
                    Channel::Mix => (fr[0] + fr[1]) * 0.5,
                    Channel::Left => fr[0],
                    Channel::Right => fr[1],
                });
            }
        }
        rs.push(&mono, &mut out);
    })?;
    Ok(out)
}

pub fn load_wav_16k(path: &Path) -> Result<Vec<f32>> {
    let mut r = hound::WavReader::open(path)?;
    let spec = r.spec();
    let ch = spec.channels as usize;
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>()
                .map(|s| s.map(|v| v as f32 / max))
                .collect::<Result<_, _>>()?
        }
    };
    let mono: Vec<f32> = if ch == 1 {
        samples
    } else {
        samples.chunks_exact(ch).map(|f| f.iter().sum::<f32>() / ch as f32).collect()
    };
    Ok(super::resample::resample(&mono, spec.sample_rate, PROC_RATE))
}

/// Escolhe o leitor pela extensão. (`.m4a`/`.mp3`/`.flac` entram no bloco de importação.)
pub fn load_16k(path: &Path, ch: Channel) -> Result<Vec<f32>> {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("opus") | Some("ogg") => load_opus_16k(path, ch),
        Some("wav") => load_wav_16k(path),
        other => bail!("formato não suportado ainda: {other:?}"),
    }
}

pub fn write_wav_16k(path: &Path, pcm: &[f32]) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: PROC_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec)?;
    for s in pcm {
        w.write_sample((s.clamp(-1.0, 1.0) * 32767.0) as i16)?;
    }
    w.finalize()?;
    Ok(())
}
