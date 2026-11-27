//! Separação de vozes: roda o `sherpa-onnx-offline-speaker-diarization` como subprocesso
//! (isola o onnxruntime e devolve a memória ao sistema quando termina).

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use super::sherpa::SherpaPaths;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Span {
    pub start: f64,
    pub end: f64,
    pub cluster: i64,
}

#[derive(Debug, Clone, Copy)]
pub struct DiarizeOptions {
    /// Número de participantes, se o usuário informou; senão decide pelo limiar.
    pub num_speakers: Option<u32>,
    pub threshold: f32,
}

impl Default for DiarizeOptions {
    fn default() -> Self {
        Self { num_speakers: None, threshold: 0.90 }
    }
}

/// `wav` deve ser WAV mono 16 kHz.
pub fn run(paths: &SherpaPaths, wav: &Path, opt: DiarizeOptions) -> Result<Vec<Span>> {
    paths.check()?;
    let mut cmd = Command::new(&paths.exe);
    cmd.arg(format!("--segmentation.pyannote-model={}", paths.segmentation.display()))
        .arg(format!("--embedding.model={}", paths.embedding.display()))
        .arg("--segmentation.num-threads=2")
        .arg("--embedding.num-threads=2")
        .arg("--min-duration-on=0.3")
        .arg("--min-duration-off=0.5");
    match opt.num_speakers {
        Some(n) if n >= 1 => cmd.arg(format!("--clustering.num-clusters={n}")),
        _ => cmd.arg(format!("--clustering.cluster-threshold={}", opt.threshold)),
    };
    cmd.arg(wav);
    #[cfg(windows)]
    if let Some(bin) = paths.exe.parent() {
        let mut dirs = vec![bin.to_path_buf()];
        if let Some(root) = bin.parent() {
            dirs.push(root.join("lib"));
        }
        let old = std::env::var_os("PATH").unwrap_or_default();
        dirs.extend(std::env::split_paths(&old));
        cmd.env("PATH", std::env::join_paths(dirs)?);
    }
    let out = cmd.output().context("falha ao executar a diarização")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("diarização falhou ({}): {}", out.status, err.lines().last().unwrap_or("").trim());
    }
    let text = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let spans = parse_output(&text);
    if spans.is_empty() {
        bail!("a diarização não encontrou fala");
    }
    Ok(spans)
}

/// Linhas `INICIO -- FIM speaker_NN`.
pub fn parse_output(text: &str) -> Vec<Span> {
    let mut spans: Vec<Span> = text
        .lines()
        .filter_map(|l| {
            let (times, spk) = l.trim().rsplit_once(' ')?;
            let cluster = spk.strip_prefix("speaker_")?.parse().ok()?;
            let (a, b) = times.split_once("--")?;
            Some(Span { start: a.trim().parse().ok()?, end: b.trim().parse().ok()?, cluster })
        })
        .collect();
    spans.sort_by(|a, b| a.start.total_cmp(&b.start));
    spans
}

/// Para cada trecho `(ini, fim)` da transcrição, o cluster de maior sobreposição (mín. `min_overlap` s).
pub fn assign_clusters(transcript: &[(f64, f64)], voice: &[Span], min_overlap: f64) -> Vec<Option<i64>> {
    transcript
        .iter()
        .map(|&(ts, te)| {
            let mut best: Option<(i64, f64)> = None;
            for v in voice {
                let ov = (te.min(v.end) - ts.max(v.start)).max(0.0);
                if ov > 0.0 && best.map_or(true, |(_, b)| ov > b) {
                    best = Some((v.cluster, ov));
                }
            }
            best.filter(|&(_, ov)| ov >= min_overlap).map(|(c, _)| c)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parseia_saida() {
        let s = parse_output("progress 100%\n0.031 -- 2.765 speaker_00\n2.765 -- 7.810 speaker_01\n");
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].cluster, 1);
        assert!((s[1].start - 2.765).abs() < 1e-9);
    }

    #[test]
    fn atribui_por_sobreposicao() {
        let v = vec![Span { start: 0.0, end: 5.0, cluster: 0 }, Span { start: 4.5, end: 10.0, cluster: 1 }];
        assert_eq!(assign_clusters(&[(0.0, 4.0), (4.0, 9.0), (20.0, 21.0)], &v, 0.2), vec![Some(0), Some(1), None]);
    }
}
