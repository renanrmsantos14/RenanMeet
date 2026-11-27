//! Reagrupamento das vozes por trecho: tira a impressão de cada trecho de fala, subtrai a
//! média da gravação (remove o efeito comum de sala/microfone) e agrupa por ligação média.
//! É o único clustering de verdade da análise; o Sherpa só entrega os trechos de fala.

use anyhow::Result;

use super::diarize::Span;
use super::embedding::{Extractor, SAMPLE_RATE};

const MIN_ITEM: f64 = 0.8;
const MAX_ITEM: f64 = 6.0;

fn normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    v.iter_mut().for_each(|x| *x /= n);
}

/// Agrupamento aglomerativo por ligação média ponderada, com matriz de distâncias atualizada
/// a cada fusão (O(n²) por passo). Devolve o rótulo (0..k) de cada vetor (já normalizados).
pub fn agglomerate(vs: &[Vec<f32>], weights: &[f32], k: Option<usize>, threshold: f32) -> Vec<usize> {
    let n = vs.len();
    let mut d = vec![0f32; n * n];
    for i in 0..n {
        for j in i + 1..n {
            let x = 1.0 - vs[i].iter().zip(&vs[j]).map(|(a, b)| a * b).sum::<f32>();
            d[i * n + j] = x;
            d[j * n + i] = x;
        }
    }
    let mut w: Vec<f32> = weights.to_vec();
    let mut alive: Vec<usize> = (0..n).collect();
    let mut members: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    while alive.len() > 1 && !k.is_some_and(|k| alive.len() <= k) {
        let (mut bi, mut bj, mut bd) = (0, 0, f32::MAX);
        for (x, &i) in alive.iter().enumerate() {
            for &j in &alive[x + 1..] {
                if d[i * n + j] < bd {
                    (bi, bj, bd) = (i, j, d[i * n + j]);
                }
            }
        }
        if k.is_none() && bd > threshold {
            break;
        }
        // a distância média ponderada ao novo grupo é a média ponderada pelos pesos dos dois grupos
        let (wa, wb) = (w[bi], w[bj]);
        for &c in &alive {
            if c != bi && c != bj {
                let v = (wa * d[bi * n + c] + wb * d[bj * n + c]) / (wa + wb);
                d[bi * n + c] = v;
                d[c * n + bi] = v;
            }
        }
        w[bi] = wa + wb;
        let moved = std::mem::take(&mut members[bj]);
        members[bi].extend(moved);
        alive.retain(|&c| c != bj);
    }
    let mut lab = vec![0; n];
    for (l, &g) in alive.iter().enumerate() {
        for &i in &members[g] {
            lab[i] = l;
        }
    }
    lab
}

/// `num_speakers`: se conhecido; senão junta enquanto a distância média (1 − cosseno, centralizado) ficar abaixo de `distance`.
pub fn refine(ex: &mut Extractor, pcm: &[f32], spans: &[Span], num_speakers: Option<usize>, distance: f32) -> Result<Vec<Span>> {
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
    let mut mean = vec![0f32; embedded[0].1.len()];
    for (_, e) in &embedded {
        mean.iter_mut().zip(e).for_each(|(m, x)| *m += x / embedded.len() as f32);
    }
    for (_, e) in embedded.iter_mut() {
        e.iter_mut().zip(&mean).for_each(|(x, m)| *x -= m);
        normalize(e);
    }
    let (idx, vs): (Vec<usize>, Vec<Vec<f32>>) = embedded.into_iter().unzip();
    let w: Vec<f32> = idx.iter().map(|&i| (items[i].1 - items[i].0) as f32).collect();
    let lab = agglomerate(&vs, &w, num_speakers, distance);
    // rótulo de cada item: o do embutido; itens curtos herdam o do vizinho mais próximo no tempo
    let mut label: Vec<Option<usize>> = vec![None; items.len()];
    for (&i, &l) in idx.iter().zip(&lab) {
        label[i] = Some(l);
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
                last.end = last.end.max(items[i].1);
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
