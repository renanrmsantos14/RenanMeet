//! Chave de assinatura das atualizações (minisign).
//!   gerar <pasta>    cria release.key (secreta, 0600) e imprime a chave pública para colar em `update::PUBKEY`
//!   verificar <arquivo>  confere <arquivo>.minisig com a chave pública embutida (`update::PUBKEY`): pega secret trocado antes de publicar
//!   assinar <arquivo> assina com a chave secreta de `MINISIGN_SECRET_KEY` (conteúdo do release.key) e grava <arquivo>.minisig

use std::io::Cursor;

use anyhow::{bail, Context, Result};
use minisign::{KeyPair, SecretKeyBox};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["gerar", dir] => {
            let kp = KeyPair::generate_unencrypted_keypair()?;
            std::fs::create_dir_all(dir)?;
            let path = std::path::Path::new(dir).join("release.key");
            std::fs::write(&path, kp.sk.to_box(None)?.to_string())?;
            #[cfg(unix)]
            std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
            println!("{}", kp.pk.to_base64());
        }
        ["assinar", file] => {
            let secret = std::env::var("MINISIGN_SECRET_KEY").context("MINISIGN_SECRET_KEY ausente")?;
            let sk = SecretKeyBox::from_string(&secret)?.into_unencrypted_secret_key()?;
            let sig = minisign::sign(None, &sk, Cursor::new(std::fs::read(file)?), None, None)?;
            std::fs::write(format!("{file}.minisig"), sig.to_string())?;
        }
        ["verificar", file] => {
            let pk = minisign_verify::PublicKey::from_base64(meet_core::update::PUBKEY).map_err(|e| anyhow::anyhow!("{e}"))?;
            let sig = minisign_verify::Signature::from_file(format!("{file}.minisig")).map_err(|e| anyhow::anyhow!("{e}"))?;
            pk.verify(&std::fs::read(file)?, &sig, false).map_err(|_| anyhow::anyhow!("a assinatura NÃO confere com a chave embutida no app"))?;
            println!("assinatura confere com a chave embutida");
        }
        _ => bail!("uso: chave_release gerar <pasta> | assinar <arquivo> | verificar <arquivo>"),
    }
    Ok(())
}
