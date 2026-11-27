//! Banco local (SQLite): reuniões, transcrição com busca (FTS5), falantes, ata e tarefas.

use std::path::Path;

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stage {
    Recording,
    Recorded,
    Processing,
    Done,
    Failed,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Recording => "recording",
            Stage::Recorded => "recorded",
            Stage::Processing => "processing",
            Stage::Done => "done",
            Stage::Failed => "failed",
        }
    }
    fn parse(s: &str) -> Stage {
        match s {
            "recording" => Stage::Recording,
            "recorded" => Stage::Recorded,
            "processing" => Stage::Processing,
            "done" => Stage::Done,
            _ => Stage::Failed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meeting {
    pub id: String,
    pub title: String,
    /// RFC 3339, fuso America/Sao_Paulo.
    pub started_at: String,
    pub duration_secs: f64,
    pub audio_path: Option<String>,
    /// `online` (mic + sistema) ou `presencial` (só mic).
    pub mode: String,
    pub num_speakers: Option<u32>,
    pub stage: Stage,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub cluster: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Speaker {
    pub cluster: i64,
    pub name: Option<String>,
    pub voice_id: Option<i64>,
    pub seconds: f64,
    pub embedding: Vec<f32>,
}

impl Speaker {
    pub fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| format!("Falante {}", self.cluster + 1))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Task {
    pub text: String,
    pub assignee: Option<String>,
    pub due: Option<String>,
    pub done: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Minutes {
    pub summary: String,
    pub decisions: Vec<String>,
    pub open_points: Vec<String>,
    pub tasks: Vec<Task>,
}

#[derive(Debug, Clone)]
pub struct Hit {
    pub meeting_id: String,
    pub title: String,
    pub snippet: String,
}

pub struct Db {
    c: Connection,
}

const SCHEMA: &str = "
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS meeting(
  id TEXT PRIMARY KEY, title TEXT NOT NULL, started_at TEXT NOT NULL,
  duration_secs REAL NOT NULL DEFAULT 0, audio_path TEXT,
  mode TEXT NOT NULL DEFAULT 'online', num_speakers INTEGER,
  stage TEXT NOT NULL DEFAULT 'recording', error TEXT);
CREATE INDEX IF NOT EXISTS idx_meeting_started ON meeting(started_at DESC);
CREATE TABLE IF NOT EXISTS segment(
  id INTEGER PRIMARY KEY, meeting_id TEXT NOT NULL REFERENCES meeting(id) ON DELETE CASCADE,
  start_secs REAL NOT NULL, end_secs REAL NOT NULL, text TEXT NOT NULL, cluster INTEGER);
CREATE INDEX IF NOT EXISTS idx_segment_meeting ON segment(meeting_id, start_secs);
CREATE VIRTUAL TABLE IF NOT EXISTS segment_fts USING fts5(
  text, content='segment', content_rowid='id', tokenize='unicode61 remove_diacritics 2');
CREATE TRIGGER IF NOT EXISTS segment_ai AFTER INSERT ON segment BEGIN
  INSERT INTO segment_fts(rowid, text) VALUES (new.id, new.text); END;
CREATE TRIGGER IF NOT EXISTS segment_ad AFTER DELETE ON segment BEGIN
  INSERT INTO segment_fts(segment_fts, rowid, text) VALUES ('delete', old.id, old.text); END;
CREATE TRIGGER IF NOT EXISTS segment_au AFTER UPDATE OF text ON segment BEGIN
  INSERT INTO segment_fts(segment_fts, rowid, text) VALUES ('delete', old.id, old.text);
  INSERT INTO segment_fts(rowid, text) VALUES (new.id, new.text); END;
CREATE TABLE IF NOT EXISTS speaker(
  meeting_id TEXT NOT NULL REFERENCES meeting(id) ON DELETE CASCADE,
  cluster INTEGER NOT NULL, name TEXT, voice_id INTEGER,
  seconds REAL NOT NULL DEFAULT 0, embedding BLOB,
  PRIMARY KEY(meeting_id, cluster));
CREATE TABLE IF NOT EXISTS minutes(
  meeting_id TEXT PRIMARY KEY REFERENCES meeting(id) ON DELETE CASCADE,
  summary TEXT NOT NULL, decisions TEXT NOT NULL, open_points TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS task(
  id INTEGER PRIMARY KEY, meeting_id TEXT NOT NULL REFERENCES meeting(id) ON DELETE CASCADE,
  text TEXT NOT NULL, assignee TEXT, due TEXT, done INTEGER NOT NULL DEFAULT 0);
CREATE INDEX IF NOT EXISTS idx_task_meeting ON task(meeting_id);
CREATE TABLE IF NOT EXISTS setting(key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

use crate::voice::store::{from_blob, to_blob};

fn blob(v: &[f32]) -> Vec<u8> {
    to_blob(v)
}
fn unblob(b: Option<Vec<u8>>) -> Vec<f32> {
    from_blob(&b.unwrap_or_default())
}

/// Entrada do usuário → expressão FTS5 segura (cada palavra vira prefixo entre aspas).
fn fts_query(q: &str) -> Option<String> {
    let words: Vec<String> = q
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\"*"))
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

fn meeting_from_row(r: &rusqlite::Row) -> rusqlite::Result<Meeting> {
    Ok(Meeting {
        id: r.get(0)?,
        title: r.get(1)?,
        started_at: r.get(2)?,
        duration_secs: r.get(3)?,
        audio_path: r.get(4)?,
        mode: r.get(5)?,
        num_speakers: r.get(6)?,
        stage: Stage::parse(&r.get::<_, String>(7)?),
        error: r.get(8)?,
    })
}

const MEETING_COLS: &str = "id, title, started_at, duration_secs, audio_path, mode, num_speakers, stage, error";

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let c = Connection::open(path)?;
        c.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(c)
    }

    pub fn memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(c: Connection) -> Result<Self> {
        c.execute_batch(SCHEMA).context("criar o esquema do banco")?;
        Ok(Self { c })
    }

    pub fn create_meeting(&self, m: &Meeting) -> Result<()> {
        self.c.execute(
            "INSERT INTO meeting(id, title, started_at, duration_secs, audio_path, mode, num_speakers, stage, error)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![m.id, m.title, m.started_at, m.duration_secs, m.audio_path, m.mode, m.num_speakers, m.stage.as_str(), m.error],
        )?;
        Ok(())
    }

    pub fn meeting(&self, id: &str) -> Result<Meeting> {
        self.c
            .query_row(&format!("SELECT {MEETING_COLS} FROM meeting WHERE id=?1"), [id], meeting_from_row)
            .optional()?
            .with_context(|| format!("reunião {id} não encontrada"))
    }

    pub fn meetings(&self) -> Result<Vec<Meeting>> {
        let mut st = self.c.prepare(&format!("SELECT {MEETING_COLS} FROM meeting ORDER BY started_at DESC"))?;
        let rows = st.query_map([], meeting_from_row)?.collect::<Result<_, _>>()?;
        Ok(rows)
    }

    pub fn set_stage(&self, id: &str, stage: Stage, error: Option<&str>) -> Result<()> {
        self.c.execute("UPDATE meeting SET stage=?2, error=?3 WHERE id=?1", params![id, stage.as_str(), error])?;
        Ok(())
    }

    pub fn finish_recording(&self, id: &str, duration_secs: f64, audio_path: &str) -> Result<()> {
        self.c.execute(
            "UPDATE meeting SET duration_secs=?2, audio_path=?3, stage='recorded', error=NULL WHERE id=?1",
            params![id, duration_secs, audio_path],
        )?;
        Ok(())
    }

    pub fn rename_meeting(&self, id: &str, title: &str) -> Result<()> {
        self.c.execute("UPDATE meeting SET title=?2 WHERE id=?1", params![id, title])?;
        Ok(())
    }

    pub fn delete_meeting(&self, id: &str) -> Result<()> {
        self.c.execute("DELETE FROM meeting WHERE id=?1", [id])?;
        Ok(())
    }

    /// Troca a transcrição e os falantes de uma reunião de uma vez (reprocessar não duplica).
    pub fn replace_transcript(&mut self, id: &str, segments: &[Segment], speakers: &[Speaker]) -> Result<()> {
        let tx = self.c.transaction()?;
        tx.execute("DELETE FROM segment WHERE meeting_id=?1", [id])?;
        tx.execute("DELETE FROM speaker WHERE meeting_id=?1", [id])?;
        {
            let mut ins = tx.prepare("INSERT INTO segment(meeting_id, start_secs, end_secs, text, cluster) VALUES(?1,?2,?3,?4,?5)")?;
            for s in segments {
                ins.execute(params![id, s.start, s.end, s.text, s.cluster])?;
            }
            let mut sp = tx.prepare("INSERT INTO speaker(meeting_id, cluster, name, voice_id, seconds, embedding) VALUES(?1,?2,?3,?4,?5,?6)")?;
            for s in speakers {
                sp.execute(params![id, s.cluster, s.name, s.voice_id, s.seconds, blob(&s.embedding)])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn segments(&self, id: &str) -> Result<Vec<Segment>> {
        let mut st = self.c.prepare("SELECT start_secs, end_secs, text, cluster FROM segment WHERE meeting_id=?1 ORDER BY start_secs, id")?;
        let rows = st
            .query_map([id], |r| Ok(Segment { start: r.get(0)?, end: r.get(1)?, text: r.get(2)?, cluster: r.get(3)? }))?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    pub fn speakers(&self, id: &str) -> Result<Vec<Speaker>> {
        let mut st = self.c.prepare("SELECT cluster, name, voice_id, seconds, embedding FROM speaker WHERE meeting_id=?1 ORDER BY seconds DESC")?;
        let rows = st
            .query_map([id], |r| {
                Ok(Speaker { cluster: r.get(0)?, name: r.get(1)?, voice_id: r.get(2)?, seconds: r.get(3)?, embedding: unblob(r.get(4)?) })
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// `None` ou vazio volta ao nome padrão ("Falante N"). `voice_id` liga o falante à voz salva.
    pub fn rename_speaker(&self, id: &str, cluster: i64, name: Option<&str>, voice_id: Option<i64>) -> Result<()> {
        let name = name.map(str::trim).filter(|n| !n.is_empty());
        let n = self.c.execute("UPDATE speaker SET name=?3, voice_id=?4 WHERE meeting_id=?1 AND cluster=?2", params![id, cluster, name, voice_id])?;
        if n == 0 {
            bail!("falante {cluster} não existe nesta reunião");
        }
        Ok(())
    }

    pub fn save_minutes(&mut self, id: &str, m: &Minutes) -> Result<()> {
        let tx = self.c.transaction()?;
        tx.execute(
            "INSERT INTO minutes(meeting_id, summary, decisions, open_points) VALUES(?1,?2,?3,?4)
             ON CONFLICT(meeting_id) DO UPDATE SET summary=?2, decisions=?3, open_points=?4",
            params![id, m.summary, serde_json::to_string(&m.decisions)?, serde_json::to_string(&m.open_points)?],
        )?;
        tx.execute("DELETE FROM task WHERE meeting_id=?1", [id])?;
        {
            let mut ins = tx.prepare("INSERT INTO task(meeting_id, text, assignee, due, done) VALUES(?1,?2,?3,?4,?5)")?;
            for t in &m.tasks {
                ins.execute(params![id, t.text, t.assignee, t.due, t.done])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn minutes(&self, id: &str) -> Result<Option<Minutes>> {
        let head: Option<(String, String, String)> = self
            .c
            .query_row("SELECT summary, decisions, open_points FROM minutes WHERE meeting_id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        let Some((summary, decisions, open_points)) = head else { return Ok(None) };
        let mut st = self.c.prepare("SELECT text, assignee, due, done FROM task WHERE meeting_id=?1 ORDER BY id")?;
        let tasks = st
            .query_map([id], |r| Ok(Task { text: r.get(0)?, assignee: r.get(1)?, due: r.get(2)?, done: r.get(3)? }))?
            .collect::<Result<_, _>>()?;
        Ok(Some(Minutes { summary, decisions: serde_json::from_str(&decisions)?, open_points: serde_json::from_str(&open_points)?, tasks }))
    }

    /// Busca no título e no texto falado (sem acento, por prefixo). Uma linha por reunião.
    pub fn search(&self, q: &str, limit: usize) -> Result<Vec<Hit>> {
        let Some(fts) = fts_query(q) else { return Ok(vec![]) };
        let mut st = self.c.prepare(
            "SELECT m.id, m.title, snippet(segment_fts, 0, '[', ']', '…', 12)
             FROM segment_fts JOIN segment s ON s.id = segment_fts.rowid JOIN meeting m ON m.id = s.meeting_id
             WHERE segment_fts MATCH ?1 ORDER BY rank LIMIT ?2",
        )?;
        let mut hits: Vec<Hit> = Vec::new();
        for r in st.query_map(params![fts, (limit * 4) as i64], |r| {
            Ok(Hit { meeting_id: r.get(0)?, title: r.get(1)?, snippet: r.get(2)? })
        })? {
            let h = r?;
            if !hits.iter().any(|x| x.meeting_id == h.meeting_id) && hits.len() < limit {
                hits.push(h);
            }
        }
        let like = format!("%{}%", q.trim().replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        let mut st = self.c.prepare("SELECT id, title FROM meeting WHERE title LIKE ?1 ESCAPE '\\' ORDER BY started_at DESC LIMIT ?2")?;
        for r in st.query_map(params![like, limit as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (id, title) = r?;
            if !hits.iter().any(|x| x.meeting_id == id) && hits.len() < limit {
                hits.push(Hit { meeting_id: id, title: title.clone(), snippet: title });
            }
        }
        Ok(hits)
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self.c.query_row("SELECT value FROM setting WHERE key=?1", [key], |r| r.get(0)).optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.c.execute("INSERT INTO setting(key, value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=?2", params![key, value])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meeting(id: &str, title: &str) -> Meeting {
        Meeting {
            id: id.into(),
            title: title.into(),
            started_at: "2026-11-27T10:00:00-03:00".into(),
            duration_secs: 0.0,
            audio_path: None,
            mode: "online".into(),
            num_speakers: None,
            stage: Stage::Recording,
            error: None,
        }
    }

    fn seg(start: f64, text: &str, cluster: i64) -> Segment {
        Segment { start, end: start + 2.0, text: text.into(), cluster: Some(cluster) }
    }

    #[test]
    fn busca_sem_acento_por_prefixo_e_uma_linha_por_reuniao() {
        let mut db = Db::memory().unwrap();
        db.create_meeting(&meeting("a", "Alinhamento")).unwrap();
        db.create_meeting(&meeting("b", "Outra")).unwrap();
        db.replace_transcript("a", &[seg(0.0, "Vamos fechar a negociação do contrato", 0), seg(3.0, "O contrato sai amanhã", 1)], &[]).unwrap();
        db.replace_transcript("b", &[seg(0.0, "Nada a ver", 0)], &[]).unwrap();
        let h = db.search("negociacao contr", 10).unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].meeting_id, "a");
        assert!(h[0].snippet.contains('['));
        assert_eq!(db.search("alinha", 10).unwrap().len(), 1, "acha pelo título");
        assert!(db.search("\" OR *", 10).unwrap().is_empty(), "entrada hostil não quebra a consulta");
    }

    #[test]
    fn reprocessar_troca_a_transcricao_sem_duplicar_e_mantem_o_indice() {
        let mut db = Db::memory().unwrap();
        db.create_meeting(&meeting("a", "R")).unwrap();
        db.replace_transcript("a", &[seg(0.0, "primeira versão", 0)], &[]).unwrap();
        db.replace_transcript("a", &[seg(0.0, "segunda versão", 0)], &[]).unwrap();
        assert_eq!(db.segments("a").unwrap().len(), 1);
        assert!(db.search("primeira", 5).unwrap().is_empty());
        assert_eq!(db.search("segunda", 5).unwrap().len(), 1);
    }

    #[test]
    fn falante_ata_e_apagar_em_cascata() {
        let mut db = Db::memory().unwrap();
        db.create_meeting(&meeting("a", "R")).unwrap();
        let sp = Speaker { cluster: 0, name: None, voice_id: None, seconds: 5.0, embedding: vec![0.5, -1.0] };
        db.replace_transcript("a", &[seg(0.0, "oi", 0)], &[sp]).unwrap();
        assert_eq!(db.speakers("a").unwrap()[0].label(), "Falante 1");
        db.rename_speaker("a", 0, Some(" Ana "), None).unwrap();
        let s = &db.speakers("a").unwrap()[0];
        assert_eq!((s.label(), s.embedding.clone()), ("Ana".to_string(), vec![0.5, -1.0]));
        assert!(db.rename_speaker("a", 9, Some("X"), None).is_err());
        let m = Minutes {
            summary: "Resumo".into(),
            decisions: vec!["D1".into()],
            open_points: vec![],
            tasks: vec![Task { text: "Enviar proposta".into(), assignee: Some("Ana".into()), due: None, done: false }],
        };
        db.save_minutes("a", &m).unwrap();
        db.save_minutes("a", &m).unwrap();
        assert_eq!(db.minutes("a").unwrap().unwrap(), m);
        db.delete_meeting("a").unwrap();
        assert!(db.segments("a").unwrap().is_empty() && db.minutes("a").unwrap().is_none());
        assert!(db.search("oi", 5).unwrap().is_empty());
    }

    #[test]
    fn configuracao_e_etapas() {
        let db = Db::memory().unwrap();
        db.create_meeting(&meeting("a", "R")).unwrap();
        db.finish_recording("a", 61.5, "x.opus").unwrap();
        let m = db.meeting("a").unwrap();
        assert_eq!((m.stage, m.duration_secs), (Stage::Recorded, 61.5));
        db.set_stage("a", Stage::Failed, Some("sem rede")).unwrap();
        assert_eq!(db.meeting("a").unwrap().error.as_deref(), Some("sem rede"));
        db.set_setting("k", "1").unwrap();
        db.set_setting("k", "2").unwrap();
        assert_eq!(db.setting("k").unwrap().as_deref(), Some("2"));
        assert!(db.meeting("nada").is_err());
    }
}
