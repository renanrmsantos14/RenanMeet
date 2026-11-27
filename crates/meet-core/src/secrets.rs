//! Chave da Groq no cofre do sistema (Secret Service no Linux, Credential Manager no Windows).
//! Nunca vai para o banco, para arquivo nem para log.

use anyhow::{Context, Result};

const SERVICE: &str = "RenanMeet";
const ACCOUNT: &str = "groq-api-key";
/// Só para testes e automação: tem precedência sobre o cofre.
pub const ENV_KEY: &str = "RENANMEET_GROQ_KEY";

fn entry() -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, ACCOUNT).context("abrir o cofre de senhas do sistema")
}

pub fn groq_key() -> Result<Option<String>> {
    if let Some(k) = std::env::var(ENV_KEY).ok().filter(|k| !k.trim().is_empty()) {
        return Ok(Some(k));
    }
    match entry()?.get_password() {
        Ok(k) if k.trim().is_empty() => Ok(None),
        Ok(k) => Ok(Some(k)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e).context("ler a chave da Groq no cofre do sistema"),
    }
}

pub fn set_groq_key(key: &str) -> Result<()> {
    if key.trim().is_empty() {
        anyhow::bail!("a chave está vazia");
    }
    entry()?.set_password(key.trim()).context("guardar a chave da Groq no cofre do sistema")
}

pub fn clear_groq_key() -> Result<()> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e).context("apagar a chave da Groq do cofre do sistema"),
    }
}

/// Máscara para mostrar na tela: `gsk_…ab12`.
pub fn mask(key: &str) -> String {
    let k = key.trim();
    let n = k.chars().count();
    if n <= 8 {
        "••••".into()
    } else {
        format!("gsk_…{}", k.chars().skip(n - 4).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mascara_nao_vaza_a_chave() {
        let m = mask("gsk_abcdefghijklmnop1234");
        assert_eq!(m, "gsk_…1234");
        assert_eq!(mask("curta"), "••••");
    }
}
