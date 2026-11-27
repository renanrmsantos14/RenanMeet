//! Exportação: Markdown da reunião e nota para a inbox da wiki.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::clock::{br_datetime, now_sp};
use crate::db::{Meeting, Minutes, Segment, Speaker};
use crate::minutes::transcript_text;

pub struct Full<'a> {
    pub meeting: &'a Meeting,
    pub speakers: &'a [Speaker],
    pub segments: &'a [Segment],
    pub minutes: Option<&'a Minutes>,
}

fn duration_text(secs: f64) -> String {
    let m = (secs / 60.0).round() as u64;
    if m >= 60 {
        format!("{}h{:02}", m / 60, m % 60)
    } else {
        format!("{m} min")
    }
}

fn list(out: &mut String, title: &str, items: &[String]) {
    if !items.is_empty() {
        out.push_str(&format!("\n## {title}\n\n"));
        for i in items {
            out.push_str(&format!("- {i}\n"));
        }
    }
}

fn task_line(t: &crate::db::Task) -> String {
    let mut s = t.text.clone();
    if let Some(a) = &t.assignee {
        s.push_str(&format!(" — {a}"));
    }
    if let Some(d) = &t.due {
        s.push_str(&format!(" (até {d})"));
    }
    s
}

pub fn markdown(f: &Full) -> String {
    let m = f.meeting;
    let names: Vec<String> = f.speakers.iter().map(Speaker::label).collect();
    let mut out = format!("# {}\n\n**Data:** {} · **Duração:** {}", m.title, br_datetime(&m.started_at), duration_text(m.duration_secs));
    if !names.is_empty() {
        out.push_str(&format!(" · **Participantes:** {}", names.join(", ")));
    }
    out.push('\n');
    if let Some(min) = f.minutes {
        out.push_str(&format!("\n## Resumo\n\n{}\n", min.summary));
        list(&mut out, "Decisões", &min.decisions);
        if !min.tasks.is_empty() {
            out.push_str("\n## Tarefas\n\n");
            for t in &min.tasks {
                out.push_str(&format!("- [{}] {}\n", if t.done { "x" } else { " " }, task_line(t)));
            }
        }
        list(&mut out, "Pontos em aberto", &min.open_points);
    }
    if !f.segments.is_empty() {
        out.push_str("\n## Transcrição\n\n");
        out.push_str(&transcript_text(f.segments, f.speakers).replace('\n', "\n\n"));
        out.push('\n');
    }
    out
}

pub fn wiki_dir() -> PathBuf {
    std::env::var_os("RENANMEET_WIKI_DIR").map(PathBuf::from).unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join("wiki"))
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.to_lowercase().chars() {
        let c = match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'ê' | 'è' => 'e',
            'í' | 'ì' => 'i',
            'ó' | 'ô' | 'õ' | 'ò' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let t: String = out.trim_end_matches('-').chars().take(40).collect();
    if t.is_empty() { "reuniao".into() } else { t }
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "pc".into())
}

/// Texto da nota (formato da skill `/wiki`): só fatos curtos — decisões e tarefas, nunca a transcrição.
pub fn wiki_note(f: &Full) -> Result<String> {
    let Some(min) = f.minutes else { bail!("gere a ata antes de mandar para a wiki") };
    let m = f.meeting;
    let mut lines = vec![format!("- Reunião \"{}\" em {} ({}).", m.title, br_datetime(&m.started_at), duration_text(m.duration_secs))];
    lines.extend(min.decisions.iter().take(3).map(|d| format!("- Decisão: {d}")));
    lines.extend(min.tasks.iter().take(4).map(|t| format!("- Tarefa: {}", task_line(t))));
    Ok(format!(
        "---\ndata: {}\norigem: RenanMeet @ {}\nassunto: Reunião — {}\n---\n{}\n",
        now_sp().format("%Y-%m-%d %H:%M"),
        hostname(),
        m.title.replace('\n', " "),
        lines.join("\n")
    ))
}

/// Grava a nota em `<wiki>/inbox` e sincroniza com o git. Devolve o arquivo e, se o git falhou, o aviso
/// (a nota fica criada mesmo assim).
pub fn send_to_wiki(f: &Full, wiki: &Path) -> Result<(PathBuf, Option<String>)> {
    if !wiki.join("inbox").is_dir() {
        bail!("a wiki não está em {} — rode /fleet para clonar", wiki.display());
    }
    let note = wiki_note(f)?;
    let stem = format!("{}-{}", now_sp().format("%Y-%m-%d-%H%M"), slug(&f.meeting.title));
    // mesmo título no mesmo minuto não pode sobrescrever a nota anterior
    let (mut name, mut n) = (format!("{stem}.md"), 1);
    while wiki.join("inbox").join(&name).exists() {
        n += 1;
        name = format!("{stem}-{n}.md");
    }
    let path = wiki.join("inbox").join(&name);
    std::fs::write(&path, note).with_context(|| format!("gravar {}", path.display()))?;
    let msg = format!("Inbox: reunião {}", f.meeting.title.replace('\n', " "));
    let warn = git_sync(wiki, &name, &msg).err().map(|e| format!("nota criada, mas o git falhou: {e}"));
    Ok((path, warn))
}

fn git(wiki: &Path, args: &[&str]) -> Result<()> {
    let out = Command::new("git").arg("-C").arg(wiki).args(args).output().context("executar git")?;
    if !out.status.success() {
        bail!("git {}: {}", args[0], String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

fn git_sync(wiki: &Path, file: &str, msg: &str) -> Result<()> {
    let rel = format!("inbox/{file}");
    git(wiki, &["add", &rel])?;
    // só a nota entra no commit, mesmo que haja outras mudanças preparadas na wiki
    git(wiki, &["commit", "-q", "-m", msg, "--", &rel])?;
    if let Err(e) = git(wiki, &["pull", "-q", "--rebase", "--autostash"]) {
        let _ = git(wiki, &["rebase", "--abort"]);
        return Err(e);
    }
    git(wiki, &["push", "-q"])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Stage, Task};

    fn fixture() -> (Meeting, Vec<Speaker>, Vec<Segment>, Minutes) {
        let m = Meeting {
            id: "a".into(),
            title: "Alinhamento do contrato".into(),
            started_at: "2026-11-27T10:00:00-03:00".into(),
            duration_secs: 3725.0,
            audio_path: None,
            mode: "online".into(),
            num_speakers: None,
            stage: Stage::Done,
            error: None,
        };
        let sp = vec![Speaker { cluster: 0, name: Some("Ana".into()), voice_id: None, seconds: 9.0, embedding: vec![] }];
        let seg = vec![Segment { start: 0.0, end: 2.0, text: "Vamos fechar.".into(), cluster: Some(0) }];
        let min = Minutes {
            summary: "Fechamos o contrato.".into(),
            decisions: vec!["Assinar até sexta".into()],
            open_points: vec!["Valor do frete".into()],
            tasks: vec![Task { text: "Enviar minuta".into(), assignee: Some("Ana".into()), due: Some("sexta".into()), done: false }],
        };
        (m, sp, seg, min)
    }

    #[test]
    fn markdown_tem_todas_as_secoes() {
        let (m, sp, seg, min) = fixture();
        let md = markdown(&Full { meeting: &m, speakers: &sp, segments: &seg, minutes: Some(&min) });
        for trecho in [
            "# Alinhamento do contrato",
            "**Data:** 27/11/2026 10:00 · **Duração:** 1h02 · **Participantes:** Ana",
            "## Resumo\n\nFechamos o contrato.",
            "## Decisões\n\n- Assinar até sexta",
            "- [ ] Enviar minuta — Ana (até sexta)",
            "## Pontos em aberto\n\n- Valor do frete",
            "[00:00] Ana: Vamos fechar.",
        ] {
            assert!(md.contains(trecho), "faltou: {trecho}\n{md}");
        }
        let sem = markdown(&Full { meeting: &m, speakers: &[], segments: &[], minutes: None });
        assert!(!sem.contains("## "), "sem ata nem transcrição não cria seções vazias");
    }

    #[test]
    fn nota_da_wiki_no_formato_da_skill_sem_transcricao() {
        let (m, sp, seg, min) = fixture();
        let n = wiki_note(&Full { meeting: &m, speakers: &sp, segments: &seg, minutes: Some(&min) }).unwrap();
        assert!(n.starts_with("---\ndata: ") && n.contains("\norigem: RenanMeet @ ") && n.contains("assunto: Reunião — Alinhamento do contrato\n---\n- Reunião"));
        assert!(n.contains("- Decisão: Assinar até sexta") && n.contains("- Tarefa: Enviar minuta — Ana (até sexta)"));
        assert!(!n.contains("Vamos fechar"), "a transcrição não vai para a wiki");
        assert!(wiki_note(&Full { meeting: &m, speakers: &sp, segments: &seg, minutes: None }).is_err());
    }

    #[test]
    fn envia_para_a_inbox_e_faz_commit() {
        let dir = std::env::temp_dir().join(format!("rm-wiki-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        let (m, sp, seg, min) = fixture();
        let f = Full { meeting: &m, speakers: &sp, segments: &seg, minutes: Some(&min) };
        // sem repositório git: a nota é criada e o aviso do git volta junto
        let (path, warn) = send_to_wiki(&f, &dir).unwrap();
        assert!(path.exists() && path.to_string_lossy().ends_with("-alinhamento-do-contrato.md"));
        assert!(warn.unwrap().contains("git"));
        assert!(send_to_wiki(&f, &dir.join("nao-existe")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn slug_sem_acento() {
        assert_eq!(slug("Reunião de Negociação — Ação!"), "reuniao-de-negociacao-acao");
        assert_eq!(slug("???"), "reuniao");
    }
}
