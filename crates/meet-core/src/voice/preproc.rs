//! Pré-processamento do modo Presencial (um microfone para todos): redução de ruído leve
//! (RNNoise, via nnnoiseless) e ganho automático, antes da diarização.

use nnnoiseless::DenoiseState;

use crate::audio::resample::resample;

/// Reduz ruído de sala. Entrada/saída: f32 mono 16 kHz.
pub fn denoise_16k(pcm: &[f32]) -> Vec<f32> {
    let up: Vec<f32> = resample(pcm, 16_000, 48_000).into_iter().map(|v| v * 32768.0).collect();
    let mut st = DenoiseState::new();
    let mut out = Vec::with_capacity(up.len() + DenoiseState::FRAME_SIZE);
    let mut frame = [0f32; DenoiseState::FRAME_SIZE];
    let mut res = [0f32; DenoiseState::FRAME_SIZE];
    for chunk in up.chunks(DenoiseState::FRAME_SIZE) {
        frame[..chunk.len()].copy_from_slice(chunk);
        frame[chunk.len()..].fill(0.0);
        st.process_frame(&mut res, &frame);
        out.extend_from_slice(&res);
    }
    out.truncate(up.len());
    let down = resample(&out.iter().map(|v| v / 32768.0).collect::<Vec<_>>(), 48_000, 16_000);
    let mut down = down;
    down.resize(pcm.len(), 0.0);
    down
}

/// Ganho automático suave: leva a fala ao nível-alvo sem amplificar silêncio/ruído de fundo.
pub fn agc(pcm: &mut [f32], target_rms: f32) {
    let win = 320; // 20 ms a 16 kHz
    let (mut gain, mut env) = (1.0f32, 0.0f32);
    for chunk in pcm.chunks_mut(win) {
        let rms = (chunk.iter().map(|v| v * v).sum::<f32>() / chunk.len() as f32).sqrt();
        // envelope com subida rápida e queda lenta
        env = if rms > env { 0.5 * env + 0.5 * rms } else { 0.98 * env + 0.02 * rms };
        if env > 0.004 {
            let want = (target_rms / env).clamp(0.5, 8.0);
            gain += (want - gain) * if want < gain { 0.3 } else { 0.02 };
        }
        for v in chunk.iter_mut() {
            *v = (*v * gain).clamp(-1.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tom(f: f32, amp: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| amp * (2.0 * std::f32::consts::PI * f * i as f32 / 16_000.0).sin()).collect()
    }

    #[test]
    fn denoise_mantem_tamanho() {
        let x = tom(300.0, 0.3, 16_000 * 2);
        assert_eq!(denoise_16k(&x).len(), x.len());
    }

    #[test]
    fn agc_sobe_voz_baixa() {
        let mut x = tom(300.0, 0.02, 16_000 * 3);
        agc(&mut x, 0.08);
        let rms = (x[x.len() - 8000..].iter().map(|v| v * v).sum::<f32>() / 8000.0).sqrt();
        assert!(rms > 0.04, "rms {rms}");
    }
}
