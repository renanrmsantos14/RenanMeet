//! Leitura de áudio para processamento: tudo vira PCM f32 mono a 16 kHz.

use std::path::Path;

use anyhow::{bail, Context, Result};

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

/// `.m4a` (AAC, o que o gravador do Android gera), `.mp3`, `.flac` e `.aac`: decodifica e vira mono 16 kHz.
pub fn load_symphonia_16k(path: &Path) -> Result<Vec<f32>> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::errors::Error;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let mut hint = Hint::new();
    if let Some(e) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(e);
    }
    let mss = MediaSourceStream::new(Box::new(std::fs::File::open(path)?), Default::default());
    let mut format = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .context("formato de áudio não reconhecido")?
        .format;
    // .mp4 de videoconferência traz vídeo e áudio: a faixa de áudio é a que declara taxa de amostragem
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL && t.codec_params.sample_rate.is_some())
        .or_else(|| format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL))
        .context("arquivo sem faixa de áudio")?;
    let (track_id, params) = (track.id, track.codec_params.clone());
    let mut decoder = symphonia::default::get_codecs().make(&params, &DecoderOptions::default()).context("codec de áudio não suportado")?;
    let mut mono: Vec<f32> = Vec::new();
    let mut rate = params.sample_rate.unwrap_or(PROC_RATE);
    let mut buf: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::ResetRequired) => break,
            Err(e) => return Err(e).context("ler o áudio"),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                let spec = *decoded.spec();
                rate = spec.rate;
                let ch = spec.channels.count().max(1);
                // o buffer acompanha o maior quadro visto (copiar para um buffer menor dá pânico)
                if buf.as_ref().map_or(true, |b| b.capacity() < decoded.capacity() * ch) {
                    buf = Some(SampleBuffer::new(decoded.capacity() as u64, spec));
                }
                let sb = buf.as_mut().expect("buffer criado acima");
                sb.copy_interleaved_ref(decoded);
                mono.extend(sb.samples().chunks_exact(ch).map(|f| f.iter().sum::<f32>() / ch as f32));
            }
            Err(Error::DecodeError(_)) => continue, // quadro corrompido: pula
            Err(e) => return Err(e).context("decodificar o áudio"),
        }
    }
    if mono.is_empty() {
        bail!("o arquivo não tem áudio legível");
    }
    Ok(super::resample::resample(&mono, rate, PROC_RATE))
}

/// Escolhe o leitor pela extensão.
pub fn load_16k(path: &Path, ch: Channel) -> Result<Vec<f32>> {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("opus") | Some("ogg") => load_opus_16k(path, ch),
        Some("wav") => load_wav_16k(path),
        Some("m4a") | Some("mp4") | Some("aac") | Some("mp3") | Some("flac") => load_symphonia_16k(path),
        other => bail!("formato não suportado ainda: {other:?}"),
    }
}

/// Grava PCM mono 16 kHz como Ogg Opus mono (o Opus trabalha a 48 kHz: reamostra antes de codificar).
pub fn write_opus_mono_16k(path: &Path, pcm: &[f32], bitrate: i32) -> Result<()> {
    let up = super::resample::resample(pcm, PROC_RATE, RATE);
    let mut w = super::opus::OpusFileWriter::create(path, 1, bitrate)?;
    let i16s: Vec<i16> = up.iter().map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i16).collect();
    w.write(&i16s)?;
    w.finish()?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m4a_aac_vira_mono_16k_com_a_duracao_e_a_frequencia_certas() {
        let f = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tom-440hz-2s.m4a");
        let pcm = load_16k(&f, Channel::Mix).unwrap();
        let secs = pcm.len() as f64 / PROC_RATE as f64;
        assert!((secs - 2.3).abs() < 0.2, "duração {secs}");
        // 440 Hz → ~880 cruzamentos de zero por segundo
        let mid = &pcm[PROC_RATE as usize / 2..PROC_RATE as usize * 3 / 2];
        let cruz = mid.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        assert!((430..450).contains(&cruz), "{cruz} ciclos em 1 s");
        assert!(load_16k(Path::new("x.xyz"), Channel::Mix).is_err());
    }

    #[test]
    fn opus_mono_ida_e_volta_mantem_a_duracao() {
        let pcm: Vec<f32> = (0..16000 * 3).map(|i| ((i as f32) * 0.2).sin() * 0.4).collect();
        let f = std::env::temp_dir().join(format!("rm-rt-{}.opus", std::process::id()));
        write_opus_mono_16k(&f, &pcm, 32_000).unwrap();
        let back = load_opus_16k(&f, Channel::Mix).unwrap();
        let _ = std::fs::remove_file(&f);
        assert!((back.len() as f64 - pcm.len() as f64).abs() < 1600.0);
    }
}

