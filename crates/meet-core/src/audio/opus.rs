//! Ogg Opus: escrita incremental (a gravação sobrevive a uma queda) e leitura.
//!
//! O encoder/decoder é o `unsafe-libopus` (libopus traduzido para Rust), então não
//! há biblioteca nativa para instalar nem para empacotar no Windows.

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use ogg::{PacketReader, PacketWriteEndInfo, PacketWriter};
use unsafe_libopus::{
    opus_decode, opus_decoder_create, opus_decoder_destroy, opus_encode, opus_encoder_create,
    opus_encoder_ctl, opus_encoder_destroy, OpusDecoder, OpusEncoder, OPUS_APPLICATION_VOIP,
    OPUS_GET_LOOKAHEAD_REQUEST, OPUS_OK, OPUS_SET_BITRATE_REQUEST, OPUS_SET_COMPLEXITY_REQUEST,
    OPUS_SET_SIGNAL_REQUEST, OPUS_SET_VBR_REQUEST, OPUS_SIGNAL_VOICE,
};

/// Taxa de gravação: 48 kHz é a taxa nativa do Opus (sem reamostrar na captura).
pub const RATE: u32 = 48_000;
/// 20 ms por quadro.
pub const FRAME: usize = 960;
const PAGE_EVERY: usize = 50; // fecha uma página por segundo: no máx. 1 s perdido

pub struct OpusFileWriter {
    enc: *mut OpusEncoder,
    pw: PacketWriter<'static, BufWriter<File>>,
    serial: u32,
    channels: usize,
    pre_skip: u64,
    samples: u64,
    packets_in_page: usize,
    scratch: Vec<u8>,
    pending: Vec<i16>,
    bytes: u64,
}

// O encoder só é usado pela thread que escreve.
unsafe impl Send for OpusFileWriter {}

impl OpusFileWriter {
    pub fn create(path: &Path, channels: usize, bitrate: i32) -> Result<Self> {
        if channels != 1 && channels != 2 {
            bail!("só mono ou estéreo");
        }
        let file = File::create(path).with_context(|| format!("criando {}", path.display()))?;
        let mut err = 0i32;
        let enc = unsafe {
            opus_encoder_create(RATE as i32, channels as i32, OPUS_APPLICATION_VOIP, &mut err)
        };
        if enc.is_null() || err != OPUS_OK {
            bail!("opus_encoder_create falhou ({err})");
        }
        let mut lookahead = 0i32;
        unsafe {
            opus_encoder_ctl!(enc, OPUS_SET_BITRATE_REQUEST, bitrate);
            opus_encoder_ctl!(enc, OPUS_SET_VBR_REQUEST, 1i32);
            opus_encoder_ctl!(enc, OPUS_SET_COMPLEXITY_REQUEST, 5i32);
            opus_encoder_ctl!(enc, OPUS_SET_SIGNAL_REQUEST, OPUS_SIGNAL_VOICE);
            opus_encoder_ctl!(enc, OPUS_GET_LOOKAHEAD_REQUEST, &mut lookahead);
        }
        let serial = 0x52_4D_45_54; // "RMET"
        let mut pw = PacketWriter::new(BufWriter::new(file));
        let pre_skip = lookahead.max(0) as u64;

        let mut head = Vec::with_capacity(19);
        head.extend_from_slice(b"OpusHead");
        head.push(1);
        head.push(channels as u8);
        head.extend_from_slice(&(pre_skip as u16).to_le_bytes());
        head.extend_from_slice(&RATE.to_le_bytes());
        head.extend_from_slice(&0i16.to_le_bytes());
        head.push(0);
        pw.write_packet(head, serial, PacketWriteEndInfo::EndPage, 0)?;

        let vendor = b"RenanMeet";
        let mut tags = Vec::new();
        tags.extend_from_slice(b"OpusTags");
        tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        tags.extend_from_slice(vendor);
        tags.extend_from_slice(&0u32.to_le_bytes());
        pw.write_packet(tags, serial, PacketWriteEndInfo::EndPage, 0)?;

        Ok(Self {
            enc,
            pw,
            serial,
            channels,
            pre_skip,
            samples: 0,
            packets_in_page: 0,
            scratch: vec![0u8; 4000],
            pending: Vec::new(),
            bytes: 0,
        })
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Segundos de áudio já entregues ao arquivo.
    pub fn seconds(&self) -> f64 {
        self.samples as f64 / RATE as f64
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// `interleaved`: PCM i16 intercalado, múltiplo de `FRAME * channels` (o resto é guardado).
    pub fn write(&mut self, interleaved: &[i16]) -> Result<()> {
        self.pending.extend_from_slice(interleaved);
        let n = FRAME * self.channels;
        let mut off = 0;
        while self.pending.len() - off >= n {
            let frame: Vec<i16> = self.pending[off..off + n].to_vec();
            self.encode_frame(&frame, false, FRAME as u64)?;
            off += n;
        }
        self.pending.drain(..off);
        Ok(())
    }

    fn encode_frame(&mut self, frame: &[i16], last: bool, real_samples: u64) -> Result<()> {
        let len = unsafe {
            opus_encode(
                self.enc,
                frame.as_ptr(),
                FRAME as i32,
                self.scratch.as_mut_ptr(),
                self.scratch.len() as i32,
            )
        };
        if len < 0 {
            bail!("opus_encode falhou ({len})");
        }
        self.samples += real_samples;
        let granule = self.pre_skip + self.samples;
        self.packets_in_page += 1;
        self.bytes += len as u64;
        let end = if last {
            PacketWriteEndInfo::EndStream
        } else if self.packets_in_page >= PAGE_EVERY {
            self.packets_in_page = 0;
            PacketWriteEndInfo::EndPage
        } else {
            PacketWriteEndInfo::NormalPacket
        };
        self.pw
            .write_packet(self.scratch[..len as usize].to_vec(), self.serial, end, granule)?;
        Ok(())
    }

    /// Fecha o arquivo (completa o último quadro com silêncio e marca o fim do stream).
    pub fn finish(mut self) -> Result<u64> {
        let n = FRAME * self.channels;
        let real = (self.pending.len() / self.channels) as u64;
        let mut frame = std::mem::take(&mut self.pending);
        frame.resize(n, 0);
        self.encode_frame(&frame, true, real)?;
        self.pw.inner_mut().flush()?;
        Ok(self.bytes)
    }
}

impl Drop for OpusFileWriter {
    fn drop(&mut self) {
        unsafe { opus_encoder_destroy(self.enc) };
    }
}

pub struct OpusInfo {
    pub channels: usize,
    pub seconds: f64,
}

/// Decodifica Ogg Opus em blocos; `sink` recebe PCM f32 intercalado a 48 kHz.
pub fn decode_file(path: &Path, mut sink: impl FnMut(&[f32], usize)) -> Result<OpusInfo> {
    let f = File::open(path).with_context(|| format!("abrindo {}", path.display()))?;
    let mut rd = PacketReader::new(BufReader::new(f));
    let head = rd
        .read_packet()?
        .ok_or_else(|| anyhow!("arquivo vazio"))?
        .data;
    if head.len() < 19 || &head[..8] != b"OpusHead" {
        bail!("não é Ogg Opus");
    }
    let channels = head[9] as usize;
    let pre_skip = u16::from_le_bytes([head[10], head[11]]) as usize;
    if channels != 1 && channels != 2 {
        bail!("canais não suportados: {channels}");
    }
    let mut err = 0i32;
    let dec: *mut OpusDecoder = unsafe { opus_decoder_create(RATE as i32, channels as i32, &mut err) };
    if dec.is_null() || err != OPUS_OK {
        bail!("opus_decoder_create falhou ({err})");
    }
    let mut pcm = vec![0f32; 5760 * channels];
    let mut tmp = vec![0i16; 5760 * channels];
    let mut skip = pre_skip;
    let mut total: u64 = 0;
    let mut tags_seen = false;
    let mut last_granule: Option<u64> = None;
    let res: Result<()> = (|| {
        while let Some(p) = rd.read_packet()? {
            if !tags_seen {
                tags_seen = true;
                if p.data.starts_with(b"OpusTags") {
                    continue;
                }
            }
            if p.absgp_page() != u64::MAX {
                last_granule = Some(p.absgp_page());
            }
            let n = unsafe {
                opus_decode(
                    dec,
                    p.data.as_ptr(),
                    p.data.len() as i32,
                    tmp.as_mut_ptr(),
                    5760,
                    0,
                )
            };
            if n < 0 {
                continue; // pacote corrompido: segue
            }
            let n = n as usize;
            for (o, s) in pcm.iter_mut().zip(&tmp[..n * channels]) {
                *o = *s as f32 / 32768.0;
            }
            let mut start = 0;
            if skip > 0 {
                let s = skip.min(n);
                skip -= s;
                start = s;
            }
            if start < n {
                sink(&pcm[start * channels..n * channels], channels);
                total += (n - start) as u64;
            }
        }
        Ok(())
    })();
    unsafe { opus_decoder_destroy(dec) };
    res?;
    let _ = last_granule;
    Ok(OpusInfo {
        channels,
        seconds: total as f64 / RATE as f64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grava_e_le_estereo() {
        let dir = std::env::temp_dir().join(format!("rm-opus-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.opus");
        let mut w = OpusFileWriter::create(&path, 2, 32_000).unwrap();
        // 2,5 s: L = 440 Hz, R = 880 Hz
        let n = (RATE as f64 * 2.5) as usize;
        let mut pcm = Vec::with_capacity(n * 2);
        for i in 0..n {
            let t = i as f64 / RATE as f64;
            pcm.push(((t * 440.0 * std::f64::consts::TAU).sin() * 8000.0) as i16);
            pcm.push(((t * 880.0 * std::f64::consts::TAU).sin() * 8000.0) as i16);
        }
        for ch in pcm.chunks(3000) {
            w.write(ch).unwrap();
        }
        let bytes = w.finish().unwrap();
        assert!(bytes > 0);
        let mut got = 0usize;
        let info = decode_file(&path, |b, c| {
            assert_eq!(c, 2);
            got += b.len() / 2;
        })
        .unwrap();
        assert_eq!(info.channels, 2);
        assert!((got as i64 - n as i64).abs() <= FRAME as i64, "got {got} n {n}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
