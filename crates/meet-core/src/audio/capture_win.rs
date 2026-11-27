//! Captura no Windows (WASAPI via cpal). Microfone = dispositivo de entrada padrão;
//! áudio do sistema = loopback do dispositivo de saída padrão (cpal abre um stream de
//! entrada num dispositivo de saída, o que no WASAPI vira loopback).
//!
//! Ainda sem teste neste repositório (o desenvolvimento é num desktop Linux).

use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};

use crate::audio::resample::Resampler;
use super::{FrameSource, Recv};
use crate::audio::opus::{FRAME, RATE};

pub struct CpalSource {
    stream: Option<Stream>,
    rx: Receiver<Vec<i16>>,
}

// `cpal::Stream` não é Send; só é criado/descartado na thread que grava.
unsafe impl Send for CpalSource {}

fn open(loopback: bool, device_name: Option<&str>) -> Result<CpalSource> {
    let host = cpal::default_host();
    let device = match (loopback, device_name) {
        (true, None) => host.default_output_device(),
        (false, None) => host.default_input_device(),
        (true, Some(n)) => host
            .output_devices()?
            .find(|d| d.name().map(|x| x == n).unwrap_or(false)),
        (false, Some(n)) => host
            .input_devices()?
            .find(|d| d.name().map(|x| x == n).unwrap_or(false)),
    }
    .ok_or_else(|| anyhow!("dispositivo de áudio não encontrado"))?;

    let cfg = if loopback {
        device.default_output_config()?
    } else {
        device.default_input_config()?
    };
    let channels = cfg.channels() as usize;
    let rate = cfg.sample_rate().0;
    let fmt = cfg.sample_format();
    let (tx, rx): (SyncSender<Vec<i16>>, _) = sync_channel(400);

    let mut resampler = if rate != RATE { Some(Resampler::new(rate, RATE)) } else { None };
    let mut acc: Vec<f32> = Vec::with_capacity(FRAME * 4);
    let mut out_tmp: Vec<f32> = Vec::new();
    let mut handle = move |mono: &[f32]| {
        match resampler.as_mut() {
            Some(r) => {
                out_tmp.clear();
                r.push(mono, &mut out_tmp);
                acc.extend_from_slice(&out_tmp);
            }
            None => acc.extend_from_slice(mono),
        }
        while acc.len() >= FRAME {
            let frame: Vec<i16> = acc[..FRAME]
                .iter()
                .map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i16)
                .collect();
            acc.drain(..FRAME);
            match tx.try_send(frame) {
                Ok(()) | Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => return,
            }
        }
    };
    let err_fn = |e| eprintln!("erro no stream de áudio: {e}");
    let sc: cpal::StreamConfig = cfg.clone().into();
    let stream = match fmt {
        SampleFormat::F32 => device.build_input_stream(
            &sc,
            move |d: &[f32], _| {
                let mono: Vec<f32> = d.chunks(channels).map(|c| c.iter().sum::<f32>() / channels as f32).collect();
                handle(&mono);
            },
            err_fn,
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            &sc,
            move |d: &[i16], _| {
                let mono: Vec<f32> = d
                    .chunks(channels)
                    .map(|c| c.iter().map(|s| *s as f32 / 32768.0).sum::<f32>() / channels as f32)
                    .collect();
                handle(&mono);
            },
            err_fn,
            None,
        ),
        other => return Err(anyhow!("formato de amostra não suportado: {other:?}")),
    }
    .context("abrindo stream WASAPI")?;
    stream.play()?;
    Ok(CpalSource {
        stream: Some(stream),
        rx,
    })
}

impl FrameSource for CpalSource {
    fn recv(&mut self, timeout: Duration) -> Recv {
        match self.rx.recv_timeout(timeout) {
            Ok(f) => Recv::Frame(f),
            Err(RecvTimeoutError::Timeout) => {
                if self.stream.is_none() {
                    Recv::Closed
                } else {
                    Recv::Timeout
                }
            }
            Err(RecvTimeoutError::Disconnected) => Recv::Closed,
        }
    }

    fn stop(&mut self) {
        self.stream = None; // descarta o stream; o canal fecha e `recv` devolve Closed
    }
}

pub fn open_mic(device: Option<&str>) -> Result<Box<dyn FrameSource>> {
    Ok(Box::new(open(false, device)?))
}

pub fn open_system(device: Option<&str>) -> Result<Box<dyn FrameSource>> {
    Ok(Box::new(open(true, device)?))
}
