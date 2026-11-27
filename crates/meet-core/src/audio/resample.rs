//! Reamostragem polifásica (sinc com janela de Kaiser), sem dependências.
//! Serve tanto para 48 kHz → 16 kHz (razão 3) quanto para arquivos importados
//! (44,1 kHz → 16 kHz etc.).

use std::f64::consts::PI;

const BASE_HALF_TAPS: usize = 16; // meia-largura do filtro, em amostras de entrada, para razão ≤ 1

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let q = x * x / 4.0;
    for k in 1..40 {
        term *= q / (k as f64 * k as f64);
        sum += term;
        if term < 1e-12 * sum {
            break;
        }
    }
    sum
}

/// Reamostrador em fluxo: `push` aceita blocos de qualquer tamanho.
pub struct Resampler {
    up: usize,
    down: usize,
    /// `up` fases × `2*HALF_TAPS` coeficientes.
    table: Vec<f32>,
    half: usize,
    buf: Vec<f32>,
    /// Posição (em amostras de entrada × up) da próxima saída, relativa a `buf[0]`.
    pos: usize,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        let g = gcd(from, to);
        let up = (to / g) as usize;
        let down = (from / g) as usize;
        // ao decimar, o filtro precisa ser proporcionalmente mais largo
        let half = BASE_HALF_TAPS * ((down + up - 1) / up).max(1);
        let n = 2 * half;
        // corte relativo à taxa de entrada interpolada (up*from)
        let cutoff = 0.5 * (1.0f64).min(up as f64 / down as f64) * 0.97;
        let beta = 8.0;
        let norm = bessel_i0(beta);
        let mut table = vec![0f32; up * n];
        for p in 0..up {
            let mut sum = 0.0;
            let mut tmp = vec![0f64; n];
            for (k, t) in tmp.iter_mut().enumerate() {
                // posição do tap relativa ao ponto de saída, em amostras de entrada
                let x = (k as f64 - (half as f64 - 1.0)) - p as f64 / up as f64;
                let w = {
                    let r = x / half as f64;
                    if r.abs() >= 1.0 {
                        0.0
                    } else {
                        bessel_i0(beta * (1.0 - r * r).sqrt()) / norm
                    }
                };
                let s = if x.abs() < 1e-12 {
                    2.0 * cutoff
                } else {
                    (2.0 * PI * cutoff * x).sin() / (PI * x)
                };
                *t = s * w;
                sum += *t;
            }
            // ganho DC unitário por fase
            for (k, t) in tmp.iter().enumerate() {
                table[p * n + k] = (*t / sum) as f32;
            }
        }
        Self {
            up,
            down,
            table,
            half,
            buf: vec![0.0; half - 1],
            pos: 0,
        }
    }

    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        self.buf.extend_from_slice(input);
        let n = 2 * self.half;
        loop {
            // saída k está em (pos / up) amostras de entrada; fase = pos % up
            let base = self.pos / self.up;
            let phase = self.pos % self.up;
            if base + n > self.buf.len() {
                break;
            }
            let coefs = &self.table[phase * n..(phase + 1) * n];
            let win = &self.buf[base..base + n];
            let mut acc = 0f32;
            for (c, s) in coefs.iter().zip(win) {
                acc += c * s;
            }
            out.push(acc);
            self.pos += self.down;
        }
        // descarta o que já não é mais necessário
        let drop = self.pos / self.up;
        if drop > 0 {
            self.buf.drain(..drop);
            self.pos -= drop * self.up;
        }
    }
}

/// Atalho para vetores inteiros.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to {
        return input.to_vec();
    }
    let mut r = Resampler::new(from, to);
    let mut out = Vec::with_capacity(input.len() * to as usize / from as usize + 64);
    r.push(input, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f64, rate: u32, secs: f64) -> Vec<f32> {
        (0..(rate as f64 * secs) as usize)
            .map(|i| (2.0 * PI * freq * i as f64 / rate as f64).sin() as f32 * 0.5)
            .collect()
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
    }

    #[test]
    fn mantem_tom_de_1khz_48k_para_16k() {
        let x = sine(1000.0, 48_000, 1.0);
        let y = resample(&x, 48_000, 16_000);
        assert!((y.len() as i64 - 16_000).abs() < 40);
        let r = rms(&y[200..y.len() - 200]);
        assert!((r - 0.5 / 2f64.sqrt()).abs() < 0.01, "rms {r}");
    }

    #[test]
    fn atenua_acima_de_nyquist() {
        let x = sine(10_000.0, 48_000, 1.0); // > 8 kHz: deve sumir
        let y = resample(&x, 48_000, 16_000);
        assert!(rms(&y[200..y.len() - 200]) < 0.01);
    }

    #[test]
    fn razao_nao_inteira_44k1_para_16k() {
        let x = sine(440.0, 44_100, 1.0);
        let y = resample(&x, 44_100, 16_000);
        assert!((y.len() as i64 - 16_000).abs() < 40);
        let r = rms(&y[200..y.len() - 200]);
        assert!((r - 0.5 / 2f64.sqrt()).abs() < 0.01, "rms {r}");
    }
}
