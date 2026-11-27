//! Vozes: diarização (quem falou quando), impressão de voz (embedding) e vozes salvas.
//! Modelos pequenos do Sherpa-ONNX (segmentação pyannote 3.0 + CAM++), usados só depois
//! da gravação e descarregados ao terminar o processo de análise.

pub mod analyze;
pub mod diarize;
pub mod embedding;
pub mod preproc;
pub mod refine;
pub mod sherpa;
pub mod store;

/// Similaridade de cosseno entre duas impressões de voz.
pub fn cosine(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let (mut dot, mut na, mut nb) = (0f64, 0f64, 0f64);
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (*x as f64, *y as f64);
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let d = na.sqrt() * nb.sqrt();
    (d > 0.0).then(|| (dot / d) as f32)
}

/// Média ponderada de impressões (cada uma com seu peso, em segundos de fala ou nº de amostras).
pub fn weighted_mean(items: &[(&[f32], f32)]) -> Option<Vec<f32>> {
    let dim = items.first()?.0.len();
    let total: f32 = items.iter().map(|(_, w)| *w).sum();
    if total <= 0.0 || items.iter().any(|(e, _)| e.len() != dim) {
        return None;
    }
    let mut out = vec![0f32; dim];
    for (e, w) in items {
        // normaliza antes de somar: o peso é de quantidade de fala, não de magnitude
        let n = e.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-9);
        for (o, v) in out.iter_mut().zip(*e) {
            *o += v / n * w / total;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosseno() {
        assert_eq!(cosine(&[1.0, 0.0], &[1.0, 0.0]), Some(1.0));
        assert_eq!(cosine(&[1.0, 0.0], &[0.0, 1.0]), Some(0.0));
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), None);
    }

    #[test]
    fn media_ponderada() {
        let m = weighted_mean(&[(&[1.0, 0.0], 3.0), (&[0.0, 1.0], 1.0)]).unwrap();
        assert!((m[0] - 0.75).abs() < 1e-6 && (m[1] - 0.25).abs() < 1e-6);
    }
}
