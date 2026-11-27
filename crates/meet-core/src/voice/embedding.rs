//! Impressão de voz (embedding) pela biblioteca C do Sherpa-ONNX, carregada em tempo de
//! execução. O modelo fica na memória só enquanto o `Extractor` existir.

use std::ffi::{c_char, c_void, CString};
use std::path::Path;

use anyhow::{anyhow, bail, Result};
use libloading::Library;

pub const SAMPLE_RATE: i32 = 16_000;

#[repr(C)]
struct Config {
    model: *const c_char,
    num_threads: i32,
    debug: i32,
    provider: *const c_char,
}

type Create = unsafe extern "C" fn(*const Config) -> *const c_void;
type Destroy = unsafe extern "C" fn(*const c_void);
type Dim = unsafe extern "C" fn(*const c_void) -> i32;
type NewStream = unsafe extern "C" fn(*const c_void) -> *const c_void;
type Accept = unsafe extern "C" fn(*const c_void, i32, *const f32, i32);
type Finish = unsafe extern "C" fn(*const c_void);
type Ready = unsafe extern "C" fn(*const c_void, *const c_void) -> i32;
type Compute = unsafe extern "C" fn(*const c_void, *const c_void) -> *const f32;
type FreeEmb = unsafe extern "C" fn(*const f32);

pub struct Extractor {
    handle: *const c_void,
    destroy: Destroy,
    dim: Dim,
    new_stream: NewStream,
    destroy_stream: Destroy,
    accept: Accept,
    finish: Finish,
    ready: Ready,
    compute: Compute,
    free_emb: FreeEmb,
    // Mantida por último: a biblioteca precisa viver mais que os ponteiros acima.
    _lib: Library,
    _model: CString,
}

impl Extractor {
    pub fn open(lib_path: &Path, model_path: &Path) -> Result<Self> {
        // Carrega antes o onnxruntime que acompanha o Sherpa (evita pegar outro do sistema).
        #[cfg(windows)]
        let _ort = lib_path
            .parent()
            .map(|d| d.join("onnxruntime.dll"))
            .filter(|p| p.exists())
            .map(|p| unsafe { Library::new(p) })
            .transpose()?;
        let lib = unsafe { Library::new(lib_path) }
            .map_err(|e| anyhow!("não foi possível carregar a biblioteca de voz: {e}"))?;
        unsafe {
            macro_rules! sym {
                ($n:literal, $t:ty) => {
                    *lib.get::<$t>(concat!($n, "\0").as_bytes()).map_err(|e| anyhow!("API de voz incompleta ({}): {e}", $n))?
                };
            }
            let create: Create = sym!("SherpaOnnxCreateSpeakerEmbeddingExtractor", Create);
            let model = CString::new(model_path.to_string_lossy().as_bytes())?;
            let provider = CString::new("cpu")?;
            let cfg = Config { model: model.as_ptr(), num_threads: 2, debug: 0, provider: provider.as_ptr() };
            let handle = create(&cfg);
            if handle.is_null() {
                bail!("modelo de impressão de voz inválido: {}", model_path.display());
            }
            Ok(Self {
                handle,
                destroy: sym!("SherpaOnnxDestroySpeakerEmbeddingExtractor", Destroy),
                dim: sym!("SherpaOnnxSpeakerEmbeddingExtractorDim", Dim),
                new_stream: sym!("SherpaOnnxSpeakerEmbeddingExtractorCreateStream", NewStream),
                destroy_stream: sym!("SherpaOnnxDestroyOnlineStream", Destroy),
                accept: sym!("SherpaOnnxOnlineStreamAcceptWaveform", Accept),
                finish: sym!("SherpaOnnxOnlineStreamInputFinished", Finish),
                ready: sym!("SherpaOnnxSpeakerEmbeddingExtractorIsReady", Ready),
                compute: sym!("SherpaOnnxSpeakerEmbeddingExtractorComputeEmbedding", Compute),
                free_emb: sym!("SherpaOnnxSpeakerEmbeddingExtractorDestroyEmbedding", FreeEmb),
                _lib: lib,
                _model: model,
            })
        }
    }

    /// Impressão de voz de um trecho (PCM f32 mono 16 kHz, ≥ 1 s de fala).
    pub fn embed(&mut self, samples: &[f32]) -> Result<Vec<f32>> {
        if samples.len() < SAMPLE_RATE as usize {
            bail!("a amostra precisa ter pelo menos 1 segundo de fala");
        }
        unsafe {
            let st = (self.new_stream)(self.handle);
            if st.is_null() {
                bail!("não foi possível criar o fluxo de voz");
            }
            (self.accept)(st, SAMPLE_RATE, samples.as_ptr(), samples.len() as i32);
            (self.finish)(st);
            if (self.ready)(self.handle, st) == 0 {
                (self.destroy_stream)(st);
                bail!("sem fala suficiente para a impressão de voz");
            }
            let p = (self.compute)(self.handle, st);
            let d = (self.dim)(self.handle);
            if p.is_null() || d <= 0 {
                (self.destroy_stream)(st);
                bail!("o modelo não gerou impressão de voz");
            }
            let v = std::slice::from_raw_parts(p, d as usize).to_vec();
            (self.free_emb)(p);
            (self.destroy_stream)(st);
            Ok(v)
        }
    }
}

impl Drop for Extractor {
    fn drop(&mut self) {
        unsafe { (self.destroy)(self.handle) }
    }
}

/// Junta até `max_secs` de fala dos trechos dados, priorizando os mais longos (melhor impressão).
pub fn gather(samples: &[f32], ranges: &[(f64, f64)], max_secs: f64) -> Vec<f32> {
    let sr = SAMPLE_RATE as f64;
    let mut r: Vec<(f64, f64)> = ranges.iter().copied().filter(|(a, b)| b - a >= 0.8).collect();
    if r.is_empty() {
        r = ranges.to_vec();
    }
    r.sort_by(|a, b| (b.1 - b.0).total_cmp(&(a.1 - a.0)));
    let cap = (max_secs * sr) as usize;
    let mut out = Vec::new();
    for (a, b) in r {
        let from = ((a.max(0.0)) * sr) as usize;
        let to = (((b.max(0.0)) * sr) as usize).min(samples.len());
        if from >= to {
            continue;
        }
        let take = (to - from).min(cap - out.len());
        out.extend_from_slice(&samples[from..from + take]);
        if out.len() >= cap {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junta_priorizando_os_mais_longos_e_limita() {
        let s = vec![1.0f32; 16000 * 100];
        let g = gather(&s, &[(0.0, 2.0), (10.0, 50.0), (60.0, 90.0)], 60.0);
        assert_eq!(g.len(), 60 * 16000);
    }
}
