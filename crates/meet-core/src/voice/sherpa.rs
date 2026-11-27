//! Onde ficam os arquivos do Sherpa-ONNX e dos modelos de voz (baixados na 1ª execução).

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use crate::paths::models_dir;

pub const EMBEDDING_FILE: &str = "3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx";

#[cfg(windows)]
const EXE: &str = "bin/sherpa-onnx-offline-speaker-diarization.exe";
#[cfg(not(windows))]
const EXE: &str = "bin/sherpa-onnx-offline-speaker-diarization";
#[cfg(windows)]
const LIB: &str = "lib/sherpa-onnx-c-api.dll";
#[cfg(not(windows))]
const LIB: &str = "lib/libsherpa-onnx-c-api.so";

#[derive(Debug, Clone)]
pub struct SherpaPaths {
    pub exe: PathBuf,
    pub lib: PathBuf,
    pub segmentation: PathBuf,
    pub embedding: PathBuf,
}

impl SherpaPaths {
    pub fn in_dir(root: &Path) -> Self {
        Self {
            exe: root.join("sherpa").join(EXE),
            lib: root.join("sherpa").join(LIB),
            segmentation: root.join("segmentation/model.onnx"),
            embedding: root.join("embedding").join(EMBEDDING_FILE),
        }
    }

    pub fn locate() -> Result<Self> {
        let p = Self::in_dir(&models_dir());
        p.check()?;
        Ok(p)
    }

    pub fn check(&self) -> Result<()> {
        for (nome, f) in [
            ("executável de diarização", &self.exe),
            ("biblioteca de voz", &self.lib),
            ("modelo de segmentação", &self.segmentation),
            ("modelo de impressão de voz", &self.embedding),
        ] {
            if !f.exists() {
                bail!("{nome} ausente: {}", f.display());
            }
        }
        Ok(())
    }
}
