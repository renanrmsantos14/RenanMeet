//! Análise de uma gravação: separa as vozes e reconhece as já salvas.

use std::borrow::Cow;
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

/// A análise roda depois da gravação, com a CPU livre: usa mais núcleos que a gravação usaria.
const ANALYSIS_THREADS: i32 = 4;

#[derive(Debug, Clone)]
pub struct AnalyzeOptions {
    pub num_speakers: Option<u32>,
    pub cluster_threshold: f32,
    /// Cosseno mínimo para dizer "é a voz salva X".
    pub voice_threshold: f32,
    /// Pré-processamento opcional (RNNoise e/ou ganho). Desligado: piorou o DER (docs/decisoes.md, D4).
    pub preproc: Option<Preproc>,
    /// Reagrupa as vozes por trecho: distância média máxima para juntar (sem nº de falantes). `None` usa só o Sherpa.
    pub refine_distance: Option<f32>,
}

#[derive(Debug, Clone, Copy)]
pub struct Preproc {
    pub denoise: bool,
    pub agc: bool,
}

impl Default for AnalyzeOptions {
    fn default() -> Self {
        Self { num_speakers: None, cluster_threshold: 0.90, voice_threshold: 0.5, preproc: None, refine_distance: Some(0.75) }
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
    let work: Cow<[f32]> = match opt.preproc {
        Some(p) => {
            let mut v = if p.denoise { preproc::denoise_16k(pcm) } else { pcm.to_vec() };
            if p.agc {
                preproc::agc(&mut v, 0.08);
            }
            Cow::Owned(v)
        }
        None => Cow::Borrowed(pcm),
    };
    let t0 = std::time::Instant::now();
    let tmp = TmpWav(tmp_wav());
    write_wav_16k(&tmp.0, &work)?;
    let mut spans = diarize::run(paths, &tmp.0, DiarizeOptions { num_speakers: opt.num_speakers, threshold: opt.cluster_threshold })?;
    drop(tmp);
    eprintln!("  segmentação (Sherpa): {:.1} s", t0.elapsed().as_secs_f64());
    let t1 = std::time::Instant::now();

    let mut ex = Extractor::open(&paths.lib, &paths.embedding, ANALYSIS_THREADS)?;
    if let Some(distance) = opt.refine_distance {
        spans = super::refine::refine(&mut ex, &work, &spans, opt.num_speakers.filter(|n| *n >= 1).map(|n| n as usize), distance)?;
    }
    eprintln!("  reagrupamento: {:.1} s", t1.elapsed().as_secs_f64());
    let mut ids: Vec<i64> = spans.iter().map(|s| s.cluster).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut clusters = Vec::new();
    for c in ids {
        let ranges: Vec<(f64, f64)> = spans.iter().filter(|s| s.cluster == c).map(|s| (s.start, s.end)).collect();
        let seconds = ranges.iter().map(|(a, b)| b - a).sum();
        let audio = gather(&work, &ranges, 60.0);
        // trecho curto demais para tirar a impressão: o falante continua na lista, só sem nome
        let embedding = ex.embed(&audio).unwrap_or_default();
        clusters.push(ClusterInfo { cluster: c, seconds, embedding, voice_id: None, name: None, score: None });
    }
    if let Some(s) = store {
        identify_clusters(&mut clusters, s, opt.voice_threshold)?;
    }
    Ok(Analysis { spans, clusters })
}

/// Compara a impressão de cada falante com as vozes salvas e dá o nome de quem passa do limiar.
pub fn identify_clusters(clusters: &mut [ClusterInfo], store: &VoiceStore, threshold: f32) -> Result<()> {
    for c in clusters.iter_mut().filter(|c| !c.embedding.is_empty()) {
        if let Some((v, score)) = store.identify(&c.embedding, threshold)? {
            (c.voice_id, c.name, c.score) = (Some(v.id), Some(v.name), Some(score));
        }
    }
    unique_voices(clusters);
    Ok(())
}

/// Uma voz salva só pode nomear um falante da gravação: em caso de empate, fica quem tem o maior cosseno.
fn unique_voices(clusters: &mut [ClusterInfo]) {
    let scores: Vec<(usize, i64, f32)> =
        clusters.iter().enumerate().filter_map(|(i, c)| Some((i, c.voice_id?, c.score?))).collect();
    for &(i, voice, score) in &scores {
        if scores.iter().any(|&(j, v, s)| j != i && v == voice && (s > score || (s == score && j < i))) {
            let c = &mut clusters[i];
            (c.voice_id, c.name, c.score) = (None, None, None);
        }
    }
}

/// Impressão de voz de uma leitura (cadastro): remove silêncio simples e tira a impressão.
pub fn enroll_embedding(paths: &SherpaPaths, pcm: &[f32]) -> Result<(Vec<f32>, f32)> {
    let voiced = trim_silence(pcm);
    let mut ex = Extractor::open(&paths.lib, &paths.embedding, 2)?;
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

/// Apaga o WAV temporário em qualquer saída (inclusive erro no meio da escrita).
struct TmpWav(PathBuf);

impl Drop for TmpWav {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn tmp_wav() -> PathBuf {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let d = data_dir().join("tmp");
    let _ = std::fs::create_dir_all(&d);
    d.join(format!("analise-{}-{}.wav", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cluster(c: i64, voice: Option<i64>, score: Option<f32>) -> ClusterInfo {
        ClusterInfo { cluster: c, seconds: 1.0, embedding: vec![], voice_id: voice, name: voice.map(|v| format!("v{v}")), score }
    }

    #[test]
    fn voz_salva_nomeia_so_um_falante() {
        let mut cs = vec![cluster(0, Some(1), Some(0.6)), cluster(1, Some(1), Some(0.8)), cluster(2, Some(2), Some(0.7)), cluster(3, None, None)];
        unique_voices(&mut cs);
        assert!(cs[0].voice_id.is_none() && cs[0].name.is_none());
        assert_eq!(cs[1].voice_id, Some(1));
        assert_eq!(cs[2].voice_id, Some(2));
    }
}
