//! Análise de uma gravação: separa as vozes e reconhece as já salvas.

use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::diarize::{self, DiarizeOptions, Span};
use super::embedding::{gather, Extractor};
use super::preproc;
use super::sherpa::SherpaPaths;
use super::store::VoiceStore;
use crate::audio::decode::{write_wav_16k, PROC_RATE};
use crate::paths::data_dir;

#[derive(Debug, Clone)]
pub struct AnalyzeOptions {
    pub num_speakers: Option<u32>,
    pub cluster_threshold: f32,
    /// Cosseno mínimo para dizer "é a voz salva X".
    pub voice_threshold: f32,
    /// Modo Presencial: redução de ruído + ganho automático antes de tudo.
    pub in_person: bool,
    /// Ajuste fino do pré-processamento do modo Presencial (padrão: os dois ligados).
    pub denoise: bool,
    pub agc: bool,
    /// Reagrupa as vozes por trecho (centralizado). Ajuda no canal único.
    pub refine: Option<super::refine::RefineOptions>,
}

impl Default for AnalyzeOptions {
    fn default() -> Self {
        Self { num_speakers: None, cluster_threshold: 0.90, voice_threshold: 0.5, in_person: false, denoise: false, agc: false, refine: Some(Default::default()) }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterInfo {
    pub cluster: i64,
    pub seconds: f64,
    pub embedding: Vec<f32>,
    pub voice_id: Option<i64>,
    pub name: Option<String>,
    pub score: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Analysis {
    pub spans: Vec<Span>,
    pub clusters: Vec<ClusterInfo>,
}

/// `pcm`: f32 mono 16 kHz da gravação inteira.
pub fn analyze(paths: &SherpaPaths, pcm: &[f32], opt: &AnalyzeOptions, store: Option<&VoiceStore>) -> Result<Analysis> {
    let work: Vec<f32> = if opt.in_person {
        let mut v = if opt.denoise { preproc::denoise_16k(pcm) } else { pcm.to_vec() };
        if opt.agc {
            preproc::agc(&mut v, 0.08);
        }
        v
    } else {
        pcm.to_vec()
    };
    let tmp = tmp_wav();
    write_wav_16k(&tmp, &work)?;
    let spans = diarize::run(paths, &tmp, DiarizeOptions { num_speakers: opt.num_speakers, threshold: opt.cluster_threshold });
    let _ = std::fs::remove_file(&tmp);
    let mut spans = spans?;

    let mut ex = Extractor::open(&paths.lib, &paths.embedding)?;
    if let Some(mut r) = opt.refine {
        r.num_speakers = r.num_speakers.or(opt.num_speakers.map(|n| n as usize));
        spans = super::refine::refine(&mut ex, &work, &spans, r)?;
    }
    let mut ids: Vec<i64> = spans.iter().map(|s| s.cluster).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut clusters = Vec::new();
    for c in ids {
        let ranges: Vec<(f64, f64)> = spans.iter().filter(|s| s.cluster == c).map(|s| (s.start, s.end)).collect();
        let seconds = ranges.iter().map(|(a, b)| b - a).sum();
        let audio = gather(&work, &ranges, 60.0);
        let Ok(embedding) = ex.embed(&audio) else { continue };
        let found = match store {
            Some(s) => s.identify(&embedding, opt.voice_threshold)?,
            None => None,
        };
        clusters.push(ClusterInfo {
            cluster: c,
            seconds,
            embedding,
            voice_id: found.as_ref().map(|(v, _)| v.id),
            name: found.as_ref().map(|(v, _)| v.name.clone()),
            score: found.as_ref().map(|(_, s)| *s),
        });
    }
    Ok(Analysis { spans, clusters })
}

/// Impressão de voz de uma leitura (cadastro): remove silêncio simples e tira a impressão.
pub fn enroll_embedding(paths: &SherpaPaths, pcm: &[f32]) -> Result<(Vec<f32>, f32)> {
    let voiced = trim_silence(pcm);
    let mut ex = Extractor::open(&paths.lib, &paths.embedding)?;
    let secs = voiced.len() as f32 / PROC_RATE as f32;
    Ok((ex.embed(&voiced)?, secs))
}

fn trim_silence(pcm: &[f32]) -> Vec<f32> {
    let win = 320;
    let peak = pcm.iter().fold(0f32, |m, v| m.max(v.abs())).max(1e-6);
    let mut out = Vec::with_capacity(pcm.len());
    for c in pcm.chunks(win) {
        let rms = (c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).sqrt();
        if rms > peak * 0.03 {
            out.extend_from_slice(c);
        }
    }
    out
}

fn tmp_wav() -> PathBuf {
    let d = data_dir().join("tmp");
    let _ = std::fs::create_dir_all(&d);
    d.join(format!("analise-{}.wav", std::process::id()))
}
