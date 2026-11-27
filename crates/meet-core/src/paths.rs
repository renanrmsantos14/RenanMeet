//! Pastas do app. Tudo pode ser redirecionado por variável de ambiente (testes).

use std::path::PathBuf;

pub const APP_DIR: &str = "RenanMeet";

/// Dados do usuário: banco, reuniões, config. `RENANMEET_DATA_DIR` sobrescreve.
pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("RENANMEET_DATA_DIR") {
        return PathBuf::from(p);
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_DIR)
}

/// Modelos baixados na primeira execução. `RENANMEET_MODELS_DIR` sobrescreve.
pub fn models_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("RENANMEET_MODELS_DIR") {
        return PathBuf::from(p);
    }
    data_dir().join("models")
}

pub fn meetings_dir() -> PathBuf {
    data_dir().join("reunioes")
}

pub fn meeting_dir(id: &str) -> PathBuf {
    meetings_dir().join(id)
}

pub fn db_path() -> PathBuf {
    data_dir().join("renanmeet.db")
}
