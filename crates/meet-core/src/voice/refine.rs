//! Reagrupamento das vozes por trecho: tira a impressão de cada trecho de fala, subtrai a
//! média da gravação (remove o efeito comum de sala/microfone) e agrupa por ligação média.
//! Pensado para o canal único (presencial), onde o clustering padrão do Sherpa confunde vozes.

use anyhow::Result;

use super::diarize::Span;
use super::embedding::{Extractor, SAMPLE_RATE};

const MIN_ITEM: f64 = 0.8;
const MAX_ITEM: f64 = 6.0;

#[derive(Debug, Clone, Copy)]
pub struct RefineOptions {
    /// Número de participantes, se conhecido.
    pub num_speakers: Option<usize>,
    /// Sem número: junta grupos enquanto a distância média (1 − cosseno, após centralizar) ficar abaixo disso.
    pub distance_threshold: f32,
    /// Subtrai a impressão média da gravação.
    pub center: bool,
}

impl Default for RefineOptions {
    fn default() -> Self {
        Self { num_speakers: None, distance_threshold: 0.75, center: true }
    }
}

fn normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    v.iter_mut().for_each(|x| *x /= n);
}

fn dist(a: &[f32], b: &[f32]) -> f32 {
    1.0 - a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>()
}

/// Agrupamento aglomerativo por ligação média. Devolve o rótulo (0..k) de cada vetor.
pub fn agglomerate(vs: &[Vec<f32>], weights: &[f32], k: Option<usize>, threshold: f32) -> Vec<usize> {
    let n = vs.len();
    let mut groups: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    let linkage = |a: &[usize], b: &[usize]| -> f32 {
        let (mut s, mut w) = (0.0, 0.0);
        for &i in a {
            for &j in b {
                let ww = weights[i] * weights[j];
                s += dist(&vs[i], &vs[j]) * ww;
                w += ww;
            }
        }
        s / w.max(1e-9)
    };
    loop {
        if groups.len() <= 1 || k.is_some_and(|k| groups.len() <= k) {
            break;
        }
        let (mut bi, mut bj, mut bd) = (0, 1, f32::MAX);
        for i in 0..groups.len() {
            for j in i + 1..groups.len() {
                let d = linkage(&groups[i], &groups[j]);
                if d < bd {
                    (bi, bj, bd) = (i, j, d);
                }
            }
        }
        if k.is_none() && bd > threshold {
            break;
        }
        let g = groups.remove(bj);
        groups[bi].extend(g);
    }
    let mut lab = vec![0; n];
    for (c, g) in groups.iter().enumerate() {
        for &i in g {
            lab[i] = c;
        }
    }
    lab
}

pub fn refine(ex: &mut Extractor, pcm: &[f32], spans: &[Span], opt: RefineOptions) -> Result<Vec<Span>> {
    // itens: trechos de até 6 s
    let mut items: Vec<(f64, f64)> = Vec::new();
    for s in spans {
        let mut a = s.start;
        while a < s.end {
            let b = (a + MAX_ITEM).min(s.end);
            // evita sobra minúscula no fim
            let b = if s.end - b < MIN_ITEM { s.end } else { b };
            items.push((a, b));
            a = b;
        }
    }
    let sr = SAMPLE_RATE as f64;
    let mut embedded: Vec<(usize, Vec<f32>)> = Vec::new();
    for (i, &(a, b)) in items.iter().enumerate() {
        if b - a < MIN_ITEM {
            continue;
        }
        let (from, to) = ((a * sr) as usize, ((b * sr) as usize).min(pcm.len()));
        if from >= to {
            continue;
        }
        if let Ok(mut e) = ex.embed(&pcm[from..to]) {
            normalize(&mut e);
            embedded.push((i, e));
        }
    }
    if embedded.len() < 2 {
        return Ok(spans.to_vec());
    }
    if opt.center {
        let dim = embedded[0].1.len();
        let mut mean = vec![0f32; dim];
        for (_, e) in &embedded {
            mean.iter_mut().zip(e).for_each(|(m, x)| *m += x / embedded.len() as f32);
        }
        for (_, e) in embedded.iter_mut() {
            e.iter_mut().zip(&mean).for_each(|(x, m)| *x -= m);
            normalize(e);
        }
    }
    let vs: Vec<Vec<f32>> = embedded.iter().map(|(_, e)| e.clone()).collect();
    let w: Vec<f32> = embedded.iter().map(|(i, _)| (items[*i].1 - items[*i].0) as f32).collect();
    let lab = agglomerate(&vs, &w, opt.num_speakers, opt.distance_threshold);
    // rótulo de cada item: o do embutido; itens curtos herdam o do vizinho mais próximo no tempo
    let mut label: Vec<Option<usize>> = vec![None; items.len()];
    for ((i, _), l) in embedded.iter().zip(&lab) {
        label[*i] = Some(*l);
    }
    let known: Vec<usize> = (0..items.len()).filter(|i| label[*i].is_some()).collect();
    let mut out: Vec<Span> = Vec::new();
    for i in 0..items.len() {
        let l = label[i].or_else(|| {
            let mid = (items[i].0 + items[i].1) / 2.0;
            known.iter().min_by(|&&x, &&y| {
                let dx = ((items[x].0 + items[x].1) / 2.0 - mid).abs();
                let dy = ((items[y].0 + items[y].1) / 2.0 - mid).abs();
                dx.total_cmp(&dy)
            }).and_then(|&x| label[x])
        });
        let Some(l) = l else { continue };
        // junta com o anterior se mesmo rótulo e contíguo
        if let Some(last) = out.last_mut() {
            if last.cluster == l as i64 && items[i].0 - last.end < 0.6 {
                last.end = items[i].1;
                continue;
            }
        }
        out.push(Span { start: items[i].0, end: items[i].1, cluster: l as i64 });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agrupa_dois_grupos() {
        let mut vs = vec![vec![1.0, 0.1], vec![0.9, 0.0], vec![0.0, 1.0], vec![0.1, 0.9]];
        vs.iter_mut().for_each(|v| normalize(v));
        let l = agglomerate(&vs, &[1.0; 4], Some(2), 0.5);
        assert_eq!(l[0], l[1]);
        assert_eq!(l[2], l[3]);
        assert_ne!(l[0], l[2]);
    }

    #[test]
    fn limiar_separa_sem_k() {
        let mut vs = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        vs.iter_mut().for_each(|v| normalize(v));
        let l = agglomerate(&vs, &[1.0; 2], None, 0.5);
        assert_ne!(l[0], l[1]);
    }
}
