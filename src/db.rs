use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

use crate::model::{Email, SearchHit};

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
            "PRAGMA journal_mode=WAL;
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
             END;"
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
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO messages
                 (source_key,message_id,conversation_id,folder,subject,sender,recipients,sent_at,body,attachment_names,content_hash)
                 VALUES(?,?,?,?,?,?,?,?,?,?,?)"
            )?;
            for email in emails {
                let mut hash = Sha256::new();
                hash.update(email.message_id.as_bytes());
                hash.update(email.subject.as_bytes());
                hash.update(email.sender.as_bytes());
                hash.update(email.sent_at.as_deref().unwrap_or_default().as_bytes());
                hash.update(email.body.as_bytes());
                let digest = format!("{:x}", hash.finalize());
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
            "SELECT m.id,m.subject,m.sender,m.recipients,m.sent_at,m.folder,m.body,
                    bm25(messages_fts,8.0,3.0,2.0,1.0,1.0) AS rank
             FROM messages_fts f JOIN messages m ON m.id=f.rowid
             WHERE messages_fts MATCH ?1 ORDER BY rank LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![fts_query, limit as i64], |r| {
            Ok(SearchHit {
                id: r.get(0)?,
                subject: r.get(1)?,
                sender: r.get(2)?,
                recipients: r.get(3)?,
                sent_at: r.get(4)?,
                folder: r.get(5)?,
                body: r.get(6)?,
                score: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
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
}
