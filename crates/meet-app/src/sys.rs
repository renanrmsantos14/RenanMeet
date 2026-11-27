//! Integrações com o sistema: seletor de arquivo, área de transferência e abrir com o computador.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{anyhow, Result};
#[cfg(windows)]
use anyhow::bail;

pub fn pick_audio() -> Result<Option<PathBuf>> {
    #[cfg(target_os = "linux")]
    let out = Command::new("zenity")
        .args(["--file-selection", "--title=Importar áudio", "--file-filter=Áudio | *.opus *.ogg *.wav *.m4a *.mp3 *.flac *.aac *.mp4"])
        .output()
        .map_err(|_| anyhow!("instale o zenity para escolher arquivos"))?;
    #[cfg(windows)]
    let out = Command::new("powershell")
        .args(["-NoProfile", "-Command", "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; Add-Type -AssemblyName System.Windows.Forms; $d=New-Object System.Windows.Forms.OpenFileDialog; $d.Filter='Áudio|*.opus;*.ogg;*.wav;*.m4a;*.mp3;*.flac;*.aac;*.mp4'; if($d.ShowDialog() -eq 'OK'){$d.FileName}"])
        .output()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok((!s.is_empty()).then(|| PathBuf::from(s)))
}

pub fn copy_text(text: &str) -> Result<()> {
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(text.to_string()))
        .map_err(|e| anyhow!("copiar: {e}"))
}

#[cfg(target_os = "linux")]
fn autostart_file() -> PathBuf {
    dirs::config_dir().unwrap_or_default().join("autostart").join("renanmeet.desktop")
}

pub fn autostart_enabled() -> bool {
    #[cfg(target_os = "linux")]
    return autostart_file().exists();
    #[cfg(windows)]
    return Command::new("reg").args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "RenanMeet"]).output().map(|o| o.status.success()).unwrap_or(false);
}

pub fn set_autostart(on: bool) -> Result<()> {
    let exe = std::env::current_exe()?;
    #[cfg(target_os = "linux")]
    {
        let f = autostart_file();
        if on {
            std::fs::create_dir_all(f.parent().unwrap())?;
            std::fs::write(&f, format!("[Desktop Entry]\nType=Application\nName=RenanMeet\nExec=\"{}\" --minimizado\nX-GNOME-Autostart-enabled=true\n", exe.display()))?;
        } else if f.exists() {
            std::fs::remove_file(f)?;
        }
    }
    #[cfg(windows)]
    {
        let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
        let st = if on {
            Command::new("reg").args(["add", key, "/v", "RenanMeet", "/t", "REG_SZ", "/d", &format!("\"{}\" --minimizado", exe.display()), "/f"]).status()?
        } else {
            Command::new("reg").args(["delete", key, "/v", "RenanMeet", "/f"]).status()?
        };
        if !st.success() {
            bail!("não consegui mexer na inicialização do Windows");
        }
    }
    Ok(())
}
