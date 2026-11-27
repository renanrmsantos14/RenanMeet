//! Atualização pelo GitHub Releases. O release traz o executável de cada sistema, um `SHA256SUMS` e a assinatura
//! minisign desse arquivo; só se troca o executável se a assinatura bater com `PUBKEY` e o sha256 com o da lista.
//! A primeira linha assinada é `# versao X.Y.Z`: amarra a lista à tag e impede republicar um binário antigo sob uma tag nova.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Chave pública minisign do release (a secreta fica em `MINISIGN_SECRET_KEY` no GitHub e em `~/.config/RenanMeet-release`).
pub const PUBKEY: &str = "RWQjaUQibPXXM41YohkIg44yhaCeSdUJ3xV1TXJnFKEerweacX79Gs8r";
const API: &str = "https://api.github.com/repos/renanrmsantos14/RenanMeet";
const MAX_BIN: u64 = 200 * 1024 * 1024;

#[cfg(windows)]
pub const ASSET: &str = "renanmeet-windows-x86_64.exe";
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub const ASSET: &str = "renanmeet-linux-x86_64";
#[cfg(not(any(windows, all(target_os = "linux", target_arch = "x86_64"))))]
pub const ASSET: &str = "renanmeet-sem-suporte-neste-sistema";

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: String,
    pub notes: String,
    bin_url: String,
    sums_url: String,
    sig_url: String,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    body: Option<String>,
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

pub struct Updater {
    base: String,
    pubkey: String,
    agent: ureq::Agent,
}

impl Default for Updater {
    fn default() -> Self {
        Self::new()
    }
}

impl Updater {
    /// `RENANMEET_UPDATE_BASE` troca o endereço da API (testes); a chave de verificação continua fixa.
    pub fn new() -> Self {
        Self::with(&std::env::var("RENANMEET_UPDATE_BASE").unwrap_or_else(|_| API.into()), PUBKEY)
    }

    pub fn with(base: &str, pubkey: &str) -> Self {
        let agent = ureq::AgentBuilder::new()
            .user_agent(concat!("renanmeet/", env!("CARGO_PKG_VERSION")))
            .timeout_connect(Duration::from_secs(15))
            .timeout(Duration::from_secs(600))
            .build();
        Self { base: base.trim_end_matches('/').into(), pubkey: pubkey.into(), agent }
    }

    /// O último release, se for mais novo que `current`.
    pub fn latest(&self, current: &str) -> Result<Option<Release>> {
        let gh: GhRelease = self
            .agent
            .get(&format!("{}/releases/latest", self.base))
            .set("Accept", "application/vnd.github+json")
            .call()
            .context("consultar o último release")?
            .into_json()?;
        let version = gh.tag_name.trim_start_matches('v').to_string();
        if !is_newer(current, &version) {
            return Ok(None);
        }
        let url = |name: &str| {
            gh.assets.iter().find(|a| a.name == name).map(|a| a.browser_download_url.clone()).ok_or_else(|| anyhow!("o release v{version} não tem {name}"))
        };
        Ok(Some(Release {
            bin_url: url(ASSET)?,
            sums_url: url("SHA256SUMS")?,
            sig_url: url("SHA256SUMS.minisig")?,
            notes: gh.body.unwrap_or_default(),
            version,
        }))
    }

    /// Baixa o executável novo para `dir` e só o devolve se a assinatura e o sha256 conferirem.
    pub fn download(&self, rel: &Release, dir: &Path) -> Result<PathBuf> {
        let sums = self.text(&rel.sums_url)?;
        let sig = self.text(&rel.sig_url)?;
        let pk = minisign_verify::PublicKey::from_base64(&self.pubkey).map_err(|e| anyhow!("chave pública inválida: {e}"))?;
        let sig = minisign_verify::Signature::decode(&sig).map_err(|e| anyhow!("assinatura ilegível: {e}"))?;
        pk.verify(sums.as_bytes(), &sig, false).map_err(|_| anyhow!("a assinatura do release não confere; atualização recusada"))?;
        if !sums.lines().any(|l| l.trim() == format!("# versao {}", rel.version)) {
            bail!("a lista assinada não é da versão {}; atualização recusada", rel.version);
        }
        let expected = sums
            .lines()
            .filter_map(|l| l.split_once(char::is_whitespace))
            .find(|(_, name)| name.trim().trim_start_matches('*') == ASSET)
            .map(|(h, _)| h.to_ascii_lowercase())
            .ok_or_else(|| anyhow!("{ASSET} não está no SHA256SUMS"))?;

        fs::create_dir_all(dir)?;
        let out = dir.join(format!("renanmeet-{}.novo", rel.version));
        let r = self.fetch_to(&rel.bin_url, &out, &expected);
        if r.is_err() {
            let _ = fs::remove_file(&out);
        }
        r.map(|_| out)
    }

    fn text(&self, url: &str) -> Result<String> {
        let mut s = String::new();
        self.agent.get(url).call().context("baixar o release")?.into_reader().take(64 * 1024).read_to_string(&mut s)?;
        Ok(s)
    }

    fn fetch_to(&self, url: &str, out: &Path, expected: &str) -> Result<()> {
        let mut rd = self.agent.get(url).call().context("baixar o executável")?.into_reader().take(MAX_BIN + 1);
        let mut f = File::create(out)?;
        let (mut hash, mut buf, mut total) = (Sha256::new(), vec![0u8; 64 * 1024], 0u64);
        loop {
            let n = rd.read(&mut buf)?;
            if n == 0 {
                break;
            }
            total += n as u64;
            hash.update(&buf[..n]);
            f.write_all(&buf[..n])?;
        }
        if total > MAX_BIN {
            bail!("o executável baixado é grande demais");
        }
        if hex(&hash.finalize()) != expected {
            bail!("sha256 do executável não confere; atualização recusada");
        }
        Ok(())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Troca o executável em uso pelo novo e devolve o caminho dele (o antigo vira `.old` até a próxima abertura; no Windows não dá para apagá-lo em uso).
/// O novo é copiado ao lado do atual e só então entra por rename: uma queda no meio da cópia não deixa binário truncado.
pub fn apply(new_bin: &Path) -> Result<PathBuf> {
    let cur = std::env::current_exe()?;
    let (old, stage) = (cur.with_extension("old"), cur.with_extension("novo"));
    let r = swap(new_bin, &cur, &old, &stage);
    let _ = fs::remove_file(new_bin);
    if r.is_err() {
        let _ = fs::remove_file(&stage);
    }
    r.map(|_| cur)
}

fn swap(new_bin: &Path, cur: &Path, old: &Path, stage: &Path) -> Result<()> {
    fs::copy(new_bin, stage).context("gravar o executável novo")?;
    #[cfg(unix)]
    fs::set_permissions(stage, std::os::unix::fs::PermissionsExt::from_mode(0o755))?;
    let _ = fs::remove_file(old);
    fs::rename(cur, old).context("separar o executável atual")?;
    if let Err(e) = fs::rename(stage, cur) {
        let _ = fs::rename(old, cur);
        return Err(e).context("instalar o executável novo");
    }
    Ok(())
}

/// Apaga o executável da versão anterior, deixado por `apply`.
pub fn cleanup_old() {
    if let Ok(cur) = std::env::current_exe() {
        let _ = fs::remove_file(cur.with_extension("old"));
    }
}

/// `true` se `candidate` (ex.: "0.0.7") é maior que `current`, comparando número a número.
pub fn is_newer(current: &str, candidate: &str) -> bool {
    let parse = |v: &str| v.trim_start_matches('v').split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let (a, b) = (parse(current), parse(candidate));
    (0..a.len().max(b.len())).map(|i| (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0))).find(|(x, y)| x != y).is_some_and(|(x, y)| y > x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::net::TcpListener;

    #[test]
    fn compara_versoes_numero_a_numero() {
        assert!(is_newer("0.0.9", "0.0.10"));
        assert!(is_newer("0.9.0", "v1.0.0"));
        assert!(!is_newer("0.0.5", "0.0.5"));
        assert!(!is_newer("0.1.0", "0.0.99"));
    }

    /// Servidor mínimo: `files` recebe o endereço do próprio servidor e devolve (caminho, conteúdo).
    fn serve(files: impl FnOnce(&str) -> Vec<(&'static str, Vec<u8>)>) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let files = files(&base);
        std::thread::spawn(move || {
            for mut s in l.incoming().flatten() {
                let mut req = [0u8; 2048];
                let n = s.read(&mut req).unwrap_or(0);
                let path = String::from_utf8_lossy(&req[..n]).split_whitespace().nth(1).unwrap_or("/").to_string();
                let body = files.iter().find(|(p, _)| *p == path).map(|(_, b)| b.clone()).unwrap_or_default();
                let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = s.write_all(&body);
            }
        });
        base
    }

    /// Release local v9.9.9: a lista é de `bin`, mas o servidor entrega `servido`; `outra_chave` assina com a chave errada.
    fn release(bin: &[u8], servido: &[u8], outra_chave: bool) -> (String, String) {
        let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let outra = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let sums = format!("# versao 9.9.9\n{}  {ASSET}\n", hex(&Sha256::digest(bin)));
        let sk = if outra_chave { &outra.sk } else { &kp.sk };
        let sig = minisign::sign(None, sk, Cursor::new(sums.as_bytes()), None, None).unwrap().to_string();
        let base = serve(|base| {
            let json = format!(
                r#"{{"tag_name":"v9.9.9","body":"notas","assets":[{{"name":"{ASSET}","browser_download_url":"{base}/bin"}},{{"name":"SHA256SUMS","browser_download_url":"{base}/sums"}},{{"name":"SHA256SUMS.minisig","browser_download_url":"{base}/sig"}}]}}"#
            );
            vec![("/releases/latest", json.into_bytes()), ("/bin", servido.to_vec()), ("/sums", sums.into_bytes()), ("/sig", sig.into_bytes())]
        });
        (base, kp.pk.to_base64())
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rm-upd-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn troca_o_executavel_e_desfaz_se_falhar() {
        let d = tmp("swap");
        fs::create_dir_all(&d).unwrap();
        let (cur, old, stage, novo) = (d.join("app"), d.join("app.old"), d.join("app.novo"), d.join("baixado"));
        fs::write(&cur, b"antigo").unwrap();
        fs::write(&novo, b"novo").unwrap();
        swap(&novo, &cur, &old, &stage).unwrap();
        assert_eq!((fs::read(&cur).unwrap(), fs::read(&old).unwrap()), (b"novo".to_vec(), b"antigo".to_vec()));
        assert!(swap(&d.join("não-existe"), &cur, &old, &stage).is_err());
        assert_eq!(fs::read(&cur).unwrap(), b"novo", "falha ao copiar não pode mexer no atual");
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn baixa_confere_e_entrega_o_executavel_novo() {
        let bin = b"binario-novo".repeat(1000);
        let (base, pk) = release(&bin, &bin, false);
        let up = Updater::with(&base, &pk);
        assert_eq!(up.latest("9.9.9").unwrap(), None);
        let rel = up.latest("0.0.1").unwrap().expect("há versão nova");
        assert_eq!((rel.version.as_str(), rel.notes.as_str()), ("9.9.9", "notas"));
        let dir = tmp("ok");
        let p = up.download(&rel, &dir).unwrap();
        assert_eq!(fs::read(&p).unwrap(), bin);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn recusa_binario_adulterado_e_assinatura_de_outra_chave() {
        let bin = b"binario-novo".to_vec();
        let (base, pk) = release(&bin, b"binario-trocado", false);
        let up = Updater::with(&base, &pk);
        let rel = up.latest("0.0.1").unwrap().unwrap();
        let dir = tmp("adulterado");
        let e = up.download(&rel, &dir).unwrap_err().to_string();
        assert!(e.contains("sha256"), "{e}");
        assert!(fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0) == 0, "o arquivo ruim não pode ficar");

        let (base, pk) = release(&bin, &bin, true);
        let up = Updater::with(&base, &pk);
        let rel = up.latest("0.0.1").unwrap().unwrap();
        let e = up.download(&rel, &dir).unwrap_err().to_string();
        assert!(e.contains("assinatura"), "{e}");
        let _ = fs::remove_dir_all(dir);
    }
}
