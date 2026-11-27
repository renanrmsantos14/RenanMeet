//! Transcrição em blocos pela Groq e junção com a separação de vozes.

use anyhow::Result;

use crate::audio::decode::{write_opus_mono_16k, PROC_RATE};
use crate::db::Segment;
use crate::groq::{Groq, SttSegment};
use crate::voice::diarize::{assign_clusters, Span};

/// Um bloco de ≤10 min em Opus mono 32 kb/s fica em ~2,4 MB, bem abaixo do limite de 25 MB da Groq.
pub const CHUNK_SECS: f64 = 600.0;
/// O corte procura o trecho mais silencioso nos últimos segundos do bloco, para não partir uma palavra.
const CUT_SEARCH_SECS: f64 = 30.0;
pub const OPUS_BITRATE: i32 = 32_000;

/// Intervalos `[ini, fim)` em amostras. Cada bloco termina no ponto mais quieto da janela final.
pub fn chunk_ranges(pcm: &[f32], chunk_secs: f64, search_secs: f64) -> Vec<(usize, usize)> {
    let rate = PROC_RATE as f64;
    let max = (chunk_secs * rate) as usize;
    let search = (search_secs * rate) as usize;
    let mut out = Vec::new();
    let mut start = 0;
    while start < pcm.len() {
        if pcm.len() - start <= max {
            out.push((start, pcm.len()));
            break;
        }
        let end = quietest(pcm, start + max - search.min(max / 2), start + max);
        out.push((start, end));
        start = end;
    }
    out
}

/// Posição (em amostras) do menor RMS em janelas de 100 ms dentro de `[from, to)`.
fn quietest(pcm: &[f32], from: usize, to: usize) -> usize {
    let win = (PROC_RATE / 10) as usize;
    let mut best = (f32::MAX, to);
    let mut i = from;
    while i + win <= to {
        let e: f32 = pcm[i..i + win].iter().map(|v| v * v).sum();
        if e < best.0 {
            best = (e, i + win / 2);
        }
        i += win;
    }
    best.1
}

fn encode_opus(pcm: &[f32]) -> Result<Vec<u8>> {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = std::env::temp_dir().join(format!("chunk-{}-{n}.opus", std::process::id()));
    let done = write_opus_mono_16k(&tmp, pcm, OPUS_BITRATE);
    let bytes = done.and_then(|()| Ok(std::fs::read(&tmp)?));
    let _ = std::fs::remove_file(&tmp);
    bytes
}

/// Transcreve a gravação inteira (PCM mono 16 kHz) em blocos, com tempos relativos ao início.
/// `on_progress(feito, total)` conta blocos.
pub fn transcribe(groq: &Groq, pcm: &[f32], language: &str, mut on_progress: impl FnMut(usize, usize)) -> Result<Vec<SttSegment>> {
    let ranges = chunk_ranges(pcm, CHUNK_SECS, CUT_SEARCH_SECS);
    let mut all = Vec::new();
    for (i, &(a, b)) in ranges.iter().enumerate() {
        on_progress(i, ranges.len());
        let offset = a as f64 / PROC_RATE as f64;
        let audio = encode_opus(&pcm[a..b])?;
        for mut s in groq.transcribe(&audio, "bloco.opus", language, None)? {
            s.start += offset;
            s.end += offset;
            all.push(s);
        }
    }
    on_progress(ranges.len(), ranges.len());
    Ok(dedupe_hallucinations(all))
}

/// O Whisper repete a mesma frase quando o áudio é silêncio ou ruído: descarta repetições seguidas (≥3).
fn dedupe_hallucinations(segs: Vec<SttSegment>) -> Vec<SttSegment> {
    let mut out: Vec<SttSegment> = Vec::with_capacity(segs.len());
    let mut run = 0;
    for s in segs {
        if out.last().is_some_and(|p| p.text.eq_ignore_ascii_case(&s.text)) {
            run += 1;
            if run >= 2 {
                continue;
            }
        } else {
            run = 0;
        }
        out.push(s);
    }
    out
}

/// Cola o falante em cada trecho do Whisper pela maior sobreposição de tempo. Trecho sem
/// sobreposição suficiente herda o falante do trecho de voz mais próximo (até `NEAR_SECS`).
pub fn attribute(segs: &[SttSegment], voice: &[Span]) -> Vec<Segment> {
    const MIN_OVERLAP: f64 = 0.2;
    const NEAR_SECS: f64 = 1.5;
    let ranges: Vec<(f64, f64)> = segs.iter().map(|s| (s.start, s.end)).collect();
    let direct = assign_clusters(&ranges, voice, MIN_OVERLAP);
    segs.iter()
        .zip(direct)
        .map(|(s, c)| {
            let cluster = c.or_else(|| {
                voice
                    .iter()
                    .map(|v| ((v.start - s.end).max(s.start - v.end).max(0.0), v.cluster))
                    .filter(|(d, _)| *d <= NEAR_SECS)
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .map(|(_, c)| c)
            });
            Segment { start: s.start, end: s.end, text: s.text.clone(), cluster }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stt(start: f64, end: f64, text: &str) -> SttSegment {
        SttSegment { start, end, text: text.into() }
    }

    #[test]
    fn corta_no_silencio_perto_do_fim_do_bloco() {
        let r = PROC_RATE as usize;
        // 50 s de "fala" com um buraco silencioso em 18–19 s; blocos de 20 s, busca nos últimos 5 s
        let mut pcm: Vec<f32> = (0..50 * r).map(|i| ((i as f32) * 0.05).sin() * 0.5).collect();
        pcm[18 * r..19 * r].iter_mut().for_each(|v| *v = 0.0);
        let rg = chunk_ranges(&pcm, 20.0, 5.0);
        assert_eq!(rg.len(), 3);
        assert!(rg[0].1 > 18 * r && rg[0].1 < 19 * r, "cortou em {}", rg[0].1 as f64 / r as f64);
        assert_eq!(rg[0].1, rg[1].0);
        assert_eq!(rg.last().unwrap().1, pcm.len());
        assert!(rg.iter().all(|(a, b)| b - a <= 20 * r));
    }

    #[test]
    fn gravacao_curta_vira_um_bloco() {
        assert_eq!(chunk_ranges(&vec![0.1; 16000 * 60], 600.0, 30.0), vec![(0, 16000 * 60)]);
        assert!(chunk_ranges(&[], 600.0, 30.0).is_empty());
    }

    #[test]
    fn bloco_codifica_em_opus_pequeno_e_com_a_duracao_certa() {
        let pcm: Vec<f32> = (0..16000 * 5).map(|i| ((i as f32) * 0.1).sin() * 0.3).collect();
        let bytes = encode_opus(&pcm).unwrap();
        assert!(bytes.starts_with(b"OggS") && bytes.len() < 40_000);
        // volta pelo decodificador: 5 s de áudio têm que continuar com 5 s (não 1,7 s por taxa errada)
        let f = std::env::temp_dir().join(format!("rm-chunk-{}.opus", std::process::id()));
        std::fs::write(&f, &bytes).unwrap();
        let volta = crate::audio::decode::load_opus_16k(&f, crate::audio::decode::Channel::Mix).unwrap();
        let _ = std::fs::remove_file(&f);
        assert!((volta.len() as f64 / 16000.0 - 5.0).abs() < 0.1, "duração {}", volta.len() as f64 / 16000.0);
    }

    #[test]
    fn atribui_por_sobreposicao_e_herda_do_vizinho() {
        let voice = vec![Span { start: 0.0, end: 5.0, cluster: 0 }, Span { start: 5.0, end: 10.0, cluster: 1 }];
        let segs = [stt(0.5, 4.0, "a"), stt(5.2, 9.0, "b"), stt(10.5, 11.0, "c"), stt(30.0, 31.0, "d")];
        let r = attribute(&segs, &voice);
        assert_eq!(r.iter().map(|s| s.cluster).collect::<Vec<_>>(), vec![Some(0), Some(1), Some(1), None]);
    }

    #[test]
    fn corta_repeticao_de_alucinacao() {
        let segs: Vec<_> = (0..6).map(|i| stt(i as f64, i as f64 + 1.0, "Obrigado.")).chain([stt(7.0, 8.0, "Fim.")]).collect();
        let r = dedupe_hallucinations(segs);
        assert_eq!(r.len(), 3);
        assert_eq!(r.last().unwrap().text, "Fim.");
    }
}
