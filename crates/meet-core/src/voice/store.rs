//! Vozes salvas (SQLite): nome + impressão média, reconhecidas nas próximas reuniões.

use std::path::Path;

use anyhow::{bail, Result};
use rusqlite::{params, Connection};

use super::{cosine, weighted_mean};

#[derive(Debug, Clone)]
pub struct Voice {
    pub id: i64,
    pub name: String,
    pub embedding: Vec<f32>,
    /// Segundos de fala que formaram a impressão (peso ao atualizar/juntar).
    pub seconds: f32,
    pub is_owner: bool,
}

pub struct VoiceStore {
    db: Connection,
}

fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}
fn from_blob(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect()
}

impl VoiceStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        Self::init(Connection::open(path)?)
    }

    pub fn memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(db: Connection) -> Result<Self> {
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS voices(
               id INTEGER PRIMARY KEY, name TEXT NOT NULL, embedding BLOB NOT NULL,
               seconds REAL NOT NULL DEFAULT 0, is_owner INTEGER NOT NULL DEFAULT 0,
               created_at TEXT NOT NULL DEFAULT (datetime('now')));",
        )?;
        Ok(Self { db })
    }

    pub fn add(&self, name: &str, embedding: &[f32], seconds: f32, is_owner: bool) -> Result<i64> {
        self.db.execute(
            "INSERT INTO voices(name, embedding, seconds, is_owner) VALUES(?1, ?2, ?3, ?4)",
            params![name, to_blob(embedding), seconds, is_owner],
        )?;
        Ok(self.db.last_insert_rowid())
    }

    pub fn list(&self) -> Result<Vec<Voice>> {
        let mut st = self.db.prepare("SELECT id, name, embedding, seconds, is_owner FROM voices ORDER BY is_owner DESC, name")?;
        let rows = st.query_map([], |r| {
            Ok(Voice { id: r.get(0)?, name: r.get(1)?, embedding: from_blob(&r.get::<_, Vec<u8>>(2)?), seconds: r.get(3)?, is_owner: r.get(4)? })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn rename(&self, id: i64, name: &str) -> Result<()> {
        self.db.execute("UPDATE voices SET name=?1 WHERE id=?2", params![name, id])?;
        Ok(())
    }

    pub fn delete(&self, id: i64) -> Result<()> {
        self.db.execute("DELETE FROM voices WHERE id=?1", params![id])?;
        Ok(())
    }

    /// Reforça a voz com nova fala (média ponderada por segundos, limitada a 10 min de peso).
    pub fn reinforce(&self, id: i64, embedding: &[f32], seconds: f32) -> Result<()> {
        let Some(v) = self.list()?.into_iter().find(|v| v.id == id) else { bail!("voz {id} não existe") };
        let merged = weighted_mean(&[(&v.embedding, v.seconds.min(600.0)), (embedding, seconds)]).unwrap_or(v.embedding);
        self.db.execute(
            "UPDATE voices SET embedding=?1, seconds=?2 WHERE id=?3",
            params![to_blob(&merged), v.seconds + seconds, id],
        )?;
        Ok(())
    }

    /// Junta `from` em `into` (mesma pessoa cadastrada duas vezes) e apaga `from`.
    pub fn merge(&self, into: i64, from: i64) -> Result<()> {
        let all = self.list()?;
        let (Some(a), Some(b)) = (all.iter().find(|v| v.id == into), all.iter().find(|v| v.id == from)) else { bail!("voz inexistente") };
        self.reinforce(a.id, &b.embedding, b.seconds)?;
        self.delete(b.id)
    }

    /// Voz salva mais parecida (cosseno ≥ `threshold`), com a pontuação.
    pub fn identify(&self, embedding: &[f32], threshold: f32) -> Result<Option<(Voice, f32)>> {
        let mut best: Option<(Voice, f32)> = None;
        for v in self.list()? {
            if let Some(s) = cosine(&v.embedding, embedding) {
                if best.as_ref().map_or(true, |(_, b)| s > *b) {
                    best = Some((v, s));
                }
            }
        }
        Ok(best.filter(|(_, s)| *s >= threshold))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn salva_reconhece_junta() {
        let s = VoiceStore::memory().unwrap();
        let a = s.add("Ana", &[1.0, 0.0, 0.0], 10.0, false).unwrap();
        let b = s.add("Ana (2)", &[0.9, 0.1, 0.0], 5.0, false).unwrap();
        s.add("Caio", &[0.0, 1.0, 0.0], 10.0, false).unwrap();
        let (v, sc) = s.identify(&[0.95, 0.05, 0.0], 0.8).unwrap().unwrap();
        assert!(v.name.starts_with("Ana") && sc > 0.99);
        assert!(s.identify(&[0.0, 0.0, 1.0], 0.5).unwrap().is_none());
        s.merge(a, b).unwrap();
        assert_eq!(s.list().unwrap().len(), 2);
        s.rename(a, "Ana Souza").unwrap();
        assert!(s.list().unwrap().iter().any(|v| v.name == "Ana Souza" && v.seconds == 15.0));
    }
}
