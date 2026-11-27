//! Modelos e binários do Sherpa-ONNX: baixados na primeira execução, com SHA-256 conferido,
//! para fora do repositório e do instalador (`models_dir`).

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::voice::sherpa::EMBEDDING_FILE;

const SHERPA_RELEASE: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download";

#[derive(Clone, Copy)]
pub enum Kind {
    /// Arquivo único gravado em `dest`.
    File,
    /// `.tar.bz2` extraído em `dest` sem o 1º nível de pastas; só entram os arquivos aceitos por `keep`.
    Tar { keep: fn(&str) -> bool },
}

pub struct Asset {
    pub id: &'static str,
    pub label: &'static str,
    pub url: String,
    pub sha256: &'static str,
    pub bytes: u64,
    /// Pasta (Tar) ou arquivo (File), relativa a `models_dir`.
    pub dest: String,
    /// Se existir, o item está instalado.
    pub marker: String,
    pub kind: Kind,
}

#[cfg(windows)]
mod platform {
    pub const URL: &str = "v1.13.7/sherpa-onnx-v1.13.7-win-x64-shared-MD-Release-no-tts.tar.bz2";
    pub const SHA: &str = "269d078c31cb176cb7c2c87952e9a8b30b19541df95445aaaa961c91a0760159";
    pub const BYTES: u64 = 18_752_734;
    pub const MARKER: &str = "sherpa/lib/sherpa-onnx-c-api.dll";
    pub fn keep(rel: &str) -> bool {
        (rel.starts_with("bin/") && (rel.ends_with(".dll") || rel.ends_with("speaker-diarization.exe")))
            || (rel.starts_with("lib/") && rel.ends_with(".dll"))
    }
}

#[cfg(not(windows))]
mod platform {
    pub const URL: &str = "v1.13.7/sherpa-onnx-v1.13.7-linux-x64-shared-no-tts.tar.bz2";
    pub const SHA: &str = "e5abe50fae5e25ad6b70bc74b51984ccea77df2571f211833b572fcc0d1c3bef";
    pub const BYTES: u64 = 24_522_109;
    pub const MARKER: &str = "sherpa/lib/libsherpa-onnx-c-api.so";
    pub fn keep(rel: &str) -> bool {
        rel.ends_with("bin/sherpa-onnx-offline-speaker-diarization") || (rel.starts_with("lib/") && rel.contains(".so"))
    }
}

fn keep_segmentation(rel: &str) -> bool {
    rel == "model.onnx"
}

pub fn catalog() -> Vec<Asset> {
    vec![
        Asset {
            id: "sherpa",
            label: "Motor de voz (Sherpa-ONNX)",
            url: format!("{SHERPA_RELEASE}/{}", platform::URL),
            sha256: platform::SHA,
            bytes: platform::BYTES,
            dest: "sherpa".into(),
            marker: platform::MARKER.into(),
            kind: Kind::Tar { keep: platform::keep },
        },
        Asset {
            id: "segmentation",
            label: "Separação de falas (pyannote 3.0)",
            url: format!("{SHERPA_RELEASE}/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2"),
            sha256: "24615ee884c897d9d2ba09bb4d30da6bb1b15e685065962db5b02e76e4996488",
            bytes: 6_958_444,
            dest: "segmentation".into(),
            marker: "segmentation/model.onnx".into(),
            kind: Kind::Tar { keep: keep_segmentation },
        },
        Asset {
            id: "embedding",
            label: "Impressão de voz (CAM++)",
            url: format!("{SHERPA_RELEASE}/speaker-recongition-models/{EMBEDDING_FILE}"),
            sha256: "53cab20ba8f2eb2f6e2d89ecc18e88ef3222169b61b4419cbc13666744a4574b",
            bytes: 28_281_165,
            dest: format!("embedding/{EMBEDDING_FILE}"),
            marker: format!("embedding/{EMBEDDING_FILE}"),
            kind: Kind::File,
        },
    ]
}

pub fn missing(root: &Path) -> Vec<Asset> {
    catalog().into_iter().filter(|a| !root.join(&a.marker).exists()).collect()
}

/// Baixa e instala o que falta. `on_progress(rótulo, bytes_baixados, bytes_totais)`.
pub fn ensure(root: &Path, mut on_progress: impl FnMut(&str, u64, u64)) -> Result<()> {
    for a in missing(root) {
        install(root, &a, &mut |done, total| on_progress(a.label, done, total)).with_context(|| format!("instalar {}", a.label))?;
    }
    Ok(())
}

pub fn install(root: &Path, a: &Asset, on_progress: &mut dyn FnMut(u64, u64)) -> Result<()> {
    fs::create_dir_all(root)?;
    let part = root.join(format!(".{}.part", a.id));
    let sha = download(&a.url, &part, a.bytes, on_progress);
    let result = sha.and_then(|sha| {
        if !sha.eq_ignore_ascii_case(a.sha256) {
            bail!("SHA-256 diferente do esperado ({}…): arquivo corrompido ou trocado", &sha[..12]);
        }
        place(root, a, &part)
    });
    let _ = fs::remove_file(&part);
    result
}

/// Baixa para `dest` e devolve o SHA-256 do que chegou.
fn download(url: &str, dest: &Path, hint: u64, on_progress: &mut dyn FnMut(u64, u64)) -> Result<String> {
    let agent = ureq::AgentBuilder::new()
        .user_agent(concat!("renanmeet/", env!("CARGO_PKG_VERSION")))
        .timeout_connect(std::time::Duration::from_secs(20))
        .timeout_read(std::time::Duration::from_secs(60))
        .build();
    let resp = agent.get(url).call().with_context(|| format!("baixar {url}"))?;
    let total = resp.header("content-length").and_then(|v| v.parse().ok()).unwrap_or(hint);
    let mut r = resp.into_reader();
    let mut out = File::create(dest)?;
    let (mut h, mut done, mut buf) = (Sha256::new(), 0u64, vec![0u8; 256 * 1024]);
    loop {
        let n = r.read(&mut buf).context("conexão interrompida durante o download")?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        out.write_all(&buf[..n])?;
        done += n as u64;
        on_progress(done, total);
    }
    out.flush()?;
    Ok(hex(&h.finalize()))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn place(root: &Path, a: &Asset, archive: &Path) -> Result<()> {
    let dest = root.join(&a.dest);
    match a.kind {
        Kind::File => {
            fs::create_dir_all(dest.parent().unwrap())?;
            fs::rename(archive, &dest).or_else(|_| fs::copy(archive, &dest).map(|_| ()))?;
        }
        Kind::Tar { keep } => {
            // extrai numa pasta vizinha e troca de uma vez: instalação interrompida não deixa pasta pela metade
            let tmp = root.join(format!(".{}.dir", a.id));
            let _ = fs::remove_dir_all(&tmp);
            extract(archive, &tmp, keep)?;
            let _ = fs::remove_dir_all(&dest);
            fs::rename(&tmp, &dest)?;
        }
    }
    Ok(())
}

fn extract(archive: &Path, dest: &Path, keep: fn(&str) -> bool) -> Result<()> {
    let mut tar = tar::Archive::new(bzip2::read::BzDecoder::new(File::open(archive)?));
    let mut kept = 0;
    for entry in tar.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path()?.into_owned();
        // sem o 1º nível; recusa `..` e caminho absoluto
        let rel: PathBuf = path.components().skip(1).collect();
        if rel.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
            continue;
        }
        let rel_s = rel.to_string_lossy().replace('\\', "/");
        if !keep(&rel_s) {
            continue;
        }
        let out = dest.join(&rel);
        fs::create_dir_all(out.parent().unwrap())?;
        entry.unpack(&out)?;
        kept += 1;
    }
    if kept == 0 {
        bail!("o pacote não trouxe nenhum dos arquivos esperados");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testserver::{serve, Reply};

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rm-models-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn tar_bz2(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut tar_bytes = Vec::new();
        {
            let mut b = tar::Builder::new(&mut tar_bytes);
            for (name, data) in files {
                let mut h = tar::Header::new_gnu();
                h.set_size(data.len() as u64);
                h.set_mode(0o644);
                b.append_data(&mut h, name, *data).unwrap();
            }
            b.finish().unwrap();
        }
        let mut enc = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
        enc.write_all(&tar_bytes).unwrap();
        enc.finish().unwrap()
    }

    fn asset(url: String, sha: &'static str, kind: Kind, dest: &str) -> Asset {
        Asset { id: "t", label: "teste", url, sha256: sha, bytes: 0, dest: dest.into(), marker: dest.into(), kind }
    }

    fn sha_of(b: &[u8]) -> &'static str {
        Box::leak(hex(&Sha256::digest(b)).into_boxed_str())
    }

    #[test]
    fn instala_arquivo_com_hash_certo_e_recusa_hash_errado() {
        let data = b"conteudo do modelo".to_vec();
        let (base, srv) = serve(vec![Reply::ok(data.clone()), Reply::ok(data.clone())]);
        let root = tmp("file");
        let ok = asset(format!("{base}/m.onnx"), sha_of(&data), Kind::File, "embedding/m.onnx");
        let mut progresso = 0;
        install(&root, &ok, &mut |d, _| progresso = d).unwrap();
        assert_eq!(fs::read(root.join("embedding/m.onnx")).unwrap(), data);
        assert_eq!(progresso, data.len() as u64);

        let ruim = asset(format!("{base}/m.onnx"), sha_of(b"outro"), Kind::File, "embedding/ruim.onnx");
        let e = install(&root, &ruim, &mut |_, _| {}).unwrap_err().to_string();
        assert!(e.contains("SHA-256"), "{e}");
        assert!(!root.join("embedding/ruim.onnx").exists() && !root.join(".t.part").exists());
        srv.join().unwrap();
    }

    #[test]
    fn extrai_so_o_que_interessa_sem_o_primeiro_nivel() {
        let tgz = tar_bz2(&[("pacote/model.onnx", b"modelo"), ("pacote/run.sh", b"x"), ("pacote/sub/model.onnx", b"nao")]);
        let (base, srv) = serve(vec![Reply::ok(tgz.clone())]);
        let root = tmp("tar");
        let a = asset(format!("{base}/p.tar.bz2"), sha_of(&tgz), Kind::Tar { keep: keep_segmentation }, "segmentation");
        install(&root, &a, &mut |_, _| {}).unwrap();
        assert_eq!(fs::read(root.join("segmentation/model.onnx")).unwrap(), b"modelo");
        assert!(!root.join("segmentation/run.sh").exists() && !root.join("segmentation/sub").exists());
        srv.join().unwrap();
    }

    #[test]
    fn pacote_sem_os_arquivos_esperados_falha_e_nao_deixa_pasta() {
        let tgz = tar_bz2(&[("pacote/leia.txt", b"x")]);
        let (base, srv) = serve(vec![Reply::ok(tgz.clone())]);
        let root = tmp("vazio");
        let a = asset(format!("{base}/p.tar.bz2"), sha_of(&tgz), Kind::Tar { keep: keep_segmentation }, "segmentation");
        assert!(install(&root, &a, &mut |_, _| {}).is_err());
        assert!(!root.join("segmentation").exists());
        srv.join().unwrap();
    }

    #[test]
    fn catalogo_aponta_para_o_que_o_sherpa_paths_espera() {
        let root = tmp("cat");
        let m = missing(&root);
        assert_eq!(m.len(), 3);
        let p = crate::voice::sherpa::SherpaPaths::in_dir(&root);
        assert!(root.join(&m[0].marker) == p.lib && root.join(&m[1].marker) == p.segmentation && root.join(&m[2].marker) == p.embedding);
        assert!(m.iter().all(|a| a.sha256.len() == 64 && a.url.starts_with("https://")));
    }
}
