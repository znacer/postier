use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::model::{Email, EmbeddingDocument, IndexedFolder, IndexedSource, SearchHit};

pub struct Index {
    conn: Connection,
}

impl Index {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path).context("open local search index")?;
        conn.execute_batch(
            "PRAGMA foreign_keys=ON;
             PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA temp_store=MEMORY;
             CREATE TABLE IF NOT EXISTS messages (
               id INTEGER PRIMARY KEY,
               source_key TEXT NOT NULL,
               message_id TEXT NOT NULL DEFAULT '',
               conversation_id TEXT NOT NULL DEFAULT '',
               folder TEXT NOT NULL DEFAULT '',
               subject TEXT NOT NULL DEFAULT '',
               sender TEXT NOT NULL DEFAULT '',
               recipients TEXT NOT NULL DEFAULT '',
               sent_at TEXT,
               body TEXT NOT NULL DEFAULT '',
               attachment_names TEXT NOT NULL DEFAULT '',
               content_hash TEXT NOT NULL UNIQUE
             );
             CREATE INDEX IF NOT EXISTS messages_source ON messages(source_key);
             CREATE TABLE IF NOT EXISTS indexed_sources (
               source_key TEXT PRIMARY KEY,
               display_name TEXT NOT NULL,
               enabled INTEGER NOT NULL DEFAULT 1
             );
             CREATE TABLE IF NOT EXISTS source_folder_settings (
               source_key TEXT NOT NULL REFERENCES indexed_sources(source_key) ON DELETE CASCADE,
               folder TEXT NOT NULL,
               enabled INTEGER NOT NULL DEFAULT 1,
               PRIMARY KEY(source_key, folder)
             );
             CREATE TABLE IF NOT EXISTS message_embeddings (
               message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
               model TEXT NOT NULL,
               dimensions INTEGER NOT NULL,
               vector BLOB NOT NULL,
               PRIMARY KEY(message_id, model)
             );
             CREATE INDEX IF NOT EXISTS message_embeddings_model
               ON message_embeddings(model);
             CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
               subject, sender, recipients, body, attachment_names,
               content='messages', content_rowid='id',
               tokenize='unicode61 remove_diacritics 2'
             );
             CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
               INSERT INTO messages_fts(rowid,subject,sender,recipients,body,attachment_names)
               VALUES(new.id,new.subject,new.sender,new.recipients,new.body,new.attachment_names);
             END;
             CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
               INSERT INTO messages_fts(messages_fts,rowid,subject,sender,recipients,body,attachment_names)
               VALUES('delete',old.id,old.subject,old.sender,old.recipients,old.body,old.attachment_names);
             END;
             INSERT OR IGNORE INTO indexed_sources(source_key, display_name, enabled)
               SELECT DISTINCT source_key, source_key, 1 FROM messages;"
        )?;
        Ok(Self { conn })
    }

    pub fn count(&self) -> Result<u64> {
        Ok(self
            .conn
            .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))?)
    }

    pub fn index_batch(&mut self, emails: &[Email]) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let mut added = 0;
        {
            let mut source_stmt = tx.prepare(
                "INSERT OR IGNORE INTO indexed_sources(source_key, display_name, enabled)
                 VALUES(?1, ?2, 1)",
            )?;
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO messages
                 (source_key,message_id,conversation_id,folder,subject,sender,recipients,sent_at,body,attachment_names,content_hash)
                 VALUES(?,?,?,?,?,?,?,?,?,?,?)"
            )?;
            for email in emails {
                let display_name = Path::new(&email.source_key)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(&email.source_key);
                source_stmt.execute(params![email.source_key, display_name])?;
                let mut hash = Sha256::new();
                hash.update(email.message_id.as_bytes());
                hash.update(email.subject.as_bytes());
                hash.update(email.sender.as_bytes());
                hash.update(email.sent_at.as_deref().unwrap_or_default().as_bytes());
                hash.update(email.body.as_bytes());
                let base_digest = format!("{:x}", hash.finalize());
                let existing_source: Option<String> = tx
                    .query_row(
                        "SELECT source_key FROM messages WHERE content_hash=?1",
                        [&base_digest],
                        |row| row.get(0),
                    )
                    .optional()?;
                let digest = if existing_source
                    .as_deref()
                    .is_some_and(|source| source != email.source_key)
                {
                    let mut source_hash = Sha256::new();
                    source_hash.update(base_digest.as_bytes());
                    source_hash.update(email.source_key.as_bytes());
                    format!("{:x}", source_hash.finalize())
                } else {
                    base_digest
                };
                added += stmt.execute(params![
                    email.source_key,
                    email.message_id,
                    email.conversation_id,
                    email.folder,
                    email.subject,
                    email.sender,
                    email.recipients,
                    email.sent_at,
                    email.body,
                    email.attachment_names,
                    digest
                ])?;
            }
        }
        tx.commit()?;
        Ok(added)
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let fts_query = query
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .map(|s| format!("\"{}\"", s.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" OR ");
        if fts_query.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare(
            "SELECT m.id,m.source_key,m.subject,m.sender,m.recipients,m.sent_at,m.folder,m.body,
                    bm25(messages_fts,8.0,3.0,2.0,1.0,1.0) AS rank
             FROM messages_fts f JOIN messages m ON m.id=f.rowid
             JOIN indexed_sources s ON s.source_key=m.source_key
             WHERE messages_fts MATCH ?1 AND s.enabled=1
               AND NOT EXISTS (
                 SELECT 1 FROM source_folder_settings sf
                 WHERE sf.source_key=m.source_key AND sf.folder=m.folder AND sf.enabled=0
               )
             ORDER BY rank LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![fts_query, limit as i64], |r| {
            Ok(SearchHit {
                id: r.get(0)?,
                source_key: r.get(1)?,
                subject: r.get(2)?,
                sender: r.get(3)?,
                recipients: r.get(4)?,
                sent_at: r.get(5)?,
                folder: r.get(6)?,
                body: r.get(7)?,
                score: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn documents_missing_embeddings(&self, model: &str) -> Result<Vec<EmbeddingDocument>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.subject, m.sender, m.recipients, m.sent_at, m.body
             FROM messages m
             LEFT JOIN message_embeddings e
               ON e.message_id=m.id AND e.model=?1
             WHERE e.message_id IS NULL ORDER BY m.id",
        )?;
        let rows = stmt.query_map([model], |row| {
            let subject: String = row.get(1)?;
            let sender: String = row.get(2)?;
            let recipients: String = row.get(3)?;
            let sent_at: Option<String> = row.get(4)?;
            let body: String = row.get(5)?;
            let text = format!(
                "Subject: {subject}\nFrom: {sender}\nTo: {recipients}\nDate: {}\nBody:\n{}",
                sent_at.as_deref().unwrap_or("unknown"),
                truncate_chars(&body, 8_000)
            );
            Ok(EmbeddingDocument {
                id: row.get(0)?,
                text,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn store_embeddings(&mut self, model: &str, values: &[(i64, Vec<f64>)]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO message_embeddings
                 (message_id, model, dimensions, vector) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (message_id, vector) in values {
                stmt.execute(params![
                    message_id,
                    model,
                    vector.len() as i64,
                    encode_vector(vector)
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn search_hybrid(
        &self,
        query: &str,
        query_embedding: &[f64],
        model: &str,
        limit: usize,
    ) -> Result<Vec<SearchHit>> {
        use std::collections::HashMap;

        let bm25 = self.search(query, 50)?;
        let mut combined: HashMap<i64, (SearchHit, f64)> = HashMap::new();
        for (rank, hit) in bm25.into_iter().enumerate() {
            combined.insert(hit.id, (hit, reciprocal_rank(rank)));
        }

        let mut stmt = self.conn.prepare(
            "SELECT m.id,m.source_key,m.subject,m.sender,m.recipients,m.sent_at,m.folder,m.body,e.vector
             FROM message_embeddings e JOIN messages m ON m.id=e.message_id
             JOIN indexed_sources s ON s.source_key=m.source_key
             WHERE e.model=?1 AND e.dimensions=?2 AND s.enabled=1
               AND NOT EXISTS (
                 SELECT 1 FROM source_folder_settings sf
                 WHERE sf.source_key=m.source_key AND sf.folder=m.folder AND sf.enabled=0
               )",
        )?;
        let rows = stmt.query_map(params![model, query_embedding.len() as i64], |row| {
            let encoded: Vec<u8> = row.get(8)?;
            Ok((
                SearchHit {
                    id: row.get(0)?,
                    source_key: row.get(1)?,
                    subject: row.get(2)?,
                    sender: row.get(3)?,
                    recipients: row.get(4)?,
                    sent_at: row.get(5)?,
                    folder: row.get(6)?,
                    body: row.get(7)?,
                    score: 0.0,
                },
                encoded,
            ))
        })?;
        let mut semantic = Vec::new();
        for row in rows {
            let (hit, encoded) = row?;
            let vector = decode_vector(&encoded)?;
            semantic.push((cosine_similarity(query_embedding, &vector), hit));
        }
        semantic.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (rank, (_, hit)) in semantic.into_iter().take(50).enumerate() {
            combined
                .entry(hit.id)
                .and_modify(|(_, score)| *score += reciprocal_rank(rank))
                .or_insert((hit, reciprocal_rank(rank)));
        }

        let mut hits: Vec<_> = combined
            .into_values()
            .map(|(mut hit, score)| {
                hit.score = score;
                hit
            })
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        hits.truncate(limit);
        Ok(hits)
    }

    pub fn sources(&self) -> Result<Vec<IndexedSource>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.source_key, s.display_name, s.enabled, count(m.id)
             FROM indexed_sources s LEFT JOIN messages m ON m.source_key=s.source_key
             GROUP BY s.source_key, s.display_name, s.enabled ORDER BY s.display_name",
        )?;
        let source_rows = stmt.query_map([], |row| {
            Ok(IndexedSource {
                key: row.get(0)?,
                name: row.get(1)?,
                enabled: row.get(2)?,
                message_count: row.get(3)?,
                folders: Vec::new(),
            })
        })?;
        let mut sources = source_rows.collect::<Result<Vec<_>, _>>()?;
        let mut folder_stmt = self.conn.prepare(
            "SELECT m.folder, coalesce(sf.enabled, 1), count(m.id)
             FROM messages m
             LEFT JOIN source_folder_settings sf
               ON sf.source_key=m.source_key AND sf.folder=m.folder
             WHERE m.source_key=?1
             GROUP BY m.folder, sf.enabled ORDER BY m.folder",
        )?;
        for source in &mut sources {
            let rows = folder_stmt.query_map([&source.key], |row| {
                Ok(IndexedFolder {
                    path: row.get(0)?,
                    enabled: row.get(1)?,
                    message_count: row.get(2)?,
                })
            })?;
            source.folders = rows.collect::<Result<Vec<_>, _>>()?;
        }
        Ok(sources)
    }

    pub fn set_source_enabled(&self, source_key: &str, enabled: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE indexed_sources SET enabled=?2 WHERE source_key=?1",
            params![source_key, enabled],
        )?;
        Ok(())
    }

    pub fn set_folder_enabled(&self, source_key: &str, folder: &str, enabled: bool) -> Result<()> {
        self.conn.execute(
            "INSERT INTO source_folder_settings(source_key, folder, enabled)
             VALUES(?1, ?2, ?3)
             ON CONFLICT(source_key, folder) DO UPDATE SET enabled=excluded.enabled",
            params![source_key, folder, enabled],
        )?;
        Ok(())
    }

    pub fn remove_source(&mut self, source_key: &str) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let removed = tx.execute("DELETE FROM messages WHERE source_key=?1", [source_key])?;
        tx.execute(
            "DELETE FROM indexed_sources WHERE source_key=?1",
            [source_key],
        )?;
        tx.commit()?;
        Ok(removed)
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn encode_vector(vector: &[f64]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| (*value as f32).to_le_bytes())
        .collect()
}

fn decode_vector(bytes: &[u8]) -> Result<Vec<f64>> {
    if !bytes.len().is_multiple_of(4) {
        anyhow::bail!("stored embedding has an invalid byte length");
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as f64)
        .collect())
}

fn reciprocal_rank(rank: usize) -> f64 {
    1.0 / (60.0 + rank as f64)
}

fn cosine_similarity(left: &[f64], right: &[f64]) -> f64 {
    let dot: f64 = left.iter().zip(right).map(|(a, b)| a * b).sum();
    let left_norm: f64 = left.iter().map(|v| v * v).sum::<f64>().sqrt();
    let right_norm: f64 = right.iter().map(|v| v * v).sum::<f64>().sqrt();
    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        dot / (left_norm * right_norm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_deduplicates_and_searches() {
        let mut db = Index::open(Path::new(":memory:")).unwrap();
        let mail = Email {
            subject: "Project Atlas decision".into(),
            sender: "a@example.test".into(),
            body: "Launch postponed until June".into(),
            ..Default::default()
        };
        assert_eq!(db.index_batch(&[mail.clone(), mail]).unwrap(), 1);
        let hits = db.search("Atlas postponed", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].subject, "Project Atlas decision");
    }

    #[test]
    fn stores_embeddings_by_message_and_model() {
        let mut db = Index::open(Path::new(":memory:")).unwrap();
        db.index_batch(&[
            Email {
                subject: "Alpha".into(),
                body: "first document".into(),
                ..Default::default()
            },
            Email {
                subject: "Beta".into(),
                body: "second document".into(),
                ..Default::default()
            },
        ])
        .unwrap();
        let missing = db.documents_missing_embeddings("embed-v1").unwrap();
        assert_eq!(missing.len(), 2);
        db.store_embeddings("embed-v1", &[(missing[0].id, vec![1.0, 0.0])])
            .unwrap();
        assert_eq!(
            db.documents_missing_embeddings("embed-v1").unwrap().len(),
            1
        );
        assert_eq!(
            db.documents_missing_embeddings("another-model")
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn hybrid_search_can_retrieve_a_semantic_only_match() {
        let mut db = Index::open(Path::new(":memory:")).unwrap();
        db.index_batch(&[
            Email {
                subject: "Exact keyword".into(),
                body: "quasar".into(),
                ..Default::default()
            },
            Email {
                subject: "Semantic result".into(),
                body: "meaning without the query token".into(),
                ..Default::default()
            },
        ])
        .unwrap();
        let documents = db.documents_missing_embeddings("embed-v1").unwrap();
        let exact_id = documents
            .iter()
            .find(|document| document.text.contains("Exact keyword"))
            .unwrap()
            .id;
        let semantic_id = documents
            .iter()
            .find(|document| document.text.contains("Semantic result"))
            .unwrap()
            .id;
        db.store_embeddings(
            "embed-v1",
            &[(exact_id, vec![0.0, 1.0]), (semantic_id, vec![1.0, 0.0])],
        )
        .unwrap();
        let hits = db
            .search_hybrid("quasar", &[1.0, 0.0], "embed-v1", 10)
            .unwrap();
        assert!(hits.iter().any(|hit| hit.id == semantic_id));
    }

    #[test]
    fn manages_multiple_sources_and_folder_filters() {
        let mut db = Index::open(Path::new(":memory:")).unwrap();
        let first = Email {
            source_key: "/archives/first.pst".into(),
            folder: "Mailbox/Inbox".into(),
            subject: "Shared project".into(),
            body: "project lighthouse".into(),
            ..Default::default()
        };
        let second = Email {
            source_key: "/archives/second.pst".into(),
            ..first.clone()
        };
        assert_eq!(db.index_batch(&[first, second]).unwrap(), 2);
        assert_eq!(db.sources().unwrap().len(), 2);
        assert_eq!(db.search("lighthouse", 10).unwrap().len(), 2);

        db.set_source_enabled("/archives/first.pst", false).unwrap();
        let hits = db.search("lighthouse", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source_key, "/archives/second.pst");

        db.set_folder_enabled("/archives/second.pst", "Mailbox/Inbox", false)
            .unwrap();
        assert!(db.search("lighthouse", 10).unwrap().is_empty());
        db.set_folder_enabled("/archives/second.pst", "Mailbox/Inbox", true)
            .unwrap();

        assert_eq!(db.remove_source("/archives/first.pst").unwrap(), 1);
        let sources = db.sources().unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].name, "second.pst");
        assert_eq!(db.count().unwrap(), 1);
    }
}
