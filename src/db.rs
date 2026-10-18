mod labels;
mod search;

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params, types::Value};
use sha2::{Digest, Sha256};

use crate::model::{
    Email, EmailRecord, IndexedFolder, IndexedSource, LabelFilter, MailScope, MailSort,
    default_label_definitions,
};

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
             CREATE INDEX IF NOT EXISTS messages_source_folder ON messages(source_key, folder);
             CREATE INDEX IF NOT EXISTS messages_sent_at ON messages(sent_at DESC, id DESC);
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
             CREATE TABLE IF NOT EXISTS label_definitions (
               name TEXT PRIMARY KEY,
               description TEXT NOT NULL DEFAULT '',
               color TEXT NOT NULL DEFAULT '#166EA6'
             );
             CREATE TABLE IF NOT EXISTS message_labels (
               message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
               label TEXT NOT NULL,
               confidence REAL,
               source TEXT NOT NULL DEFAULT 'manual',
               PRIMARY KEY(message_id, label)
             );
             CREATE INDEX IF NOT EXISTS message_labels_label ON message_labels(label);
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
             END;",
        )?;

        let source_count: u64 =
            conn.query_row("SELECT count(*) FROM indexed_sources", [], |r| r.get(0))?;
        if source_count == 0 {
            conn.execute(
                "INSERT OR IGNORE INTO indexed_sources(source_key, display_name, enabled)
                 SELECT DISTINCT source_key, source_key, 1 FROM messages",
                [],
            )?;
        }

        let label_count: u64 =
            conn.query_row("SELECT count(*) FROM label_definitions", [], |r| r.get(0))?;
        if label_count == 0 {
            for def in default_label_definitions() {
                conn.execute(
                    "INSERT OR IGNORE INTO label_definitions(name, description, color) VALUES(?1, ?2, ?3)",
                    params![def.name, def.description, def.color],
                )?;
            }
        }

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
                 VALUES(?,?,?,?,?,?,?,?,?,?,?)",
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

    pub fn get_message(&self, id: i64) -> Result<Option<EmailRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.source_key, m.message_id, m.conversation_id, m.folder,
                    m.subject, m.sender, m.recipients, m.sent_at, m.body, m.attachment_names
             FROM messages m WHERE m.id = ?1",
        )?;
        let record = stmt.query_row([id], map_email_record).optional()?;
        let Some(mut record) = record else {
            return Ok(None);
        };
        record.labels = self.get_message_labels(record.id)?;
        Ok(Some(record))
    }

    pub fn get_messages_by_ids(&self, ids: &[i64]) -> Result<Vec<EmailRecord>> {
        let mut out = Vec::with_capacity(ids.len());
        for &id in ids {
            if let Some(record) = self.get_message(id)? {
                out.push(record);
            }
        }
        Ok(out)
    }

    pub fn list_messages(
        &self,
        scope: &MailScope,
        label_filter: &LabelFilter,
        query: &str,
        sort: MailSort,
        limit: usize,
    ) -> Result<Vec<EmailRecord>> {
        let fts_query = build_fts_query(query);
        let order_clause = match sort {
            MailSort::DateDesc => "coalesce(m.sent_at, '') DESC, m.id DESC",
            MailSort::DateAsc => "coalesce(m.sent_at, '') ASC, m.id ASC",
            MailSort::SubjectAsc => "lower(m.subject) ASC, m.id DESC",
            MailSort::SenderAsc => "lower(m.sender) ASC, m.id DESC",
        };

        let mut sql = String::new();
        let mut bind_values: Vec<Value> = Vec::new();

        if fts_query.is_empty() {
            sql.push_str(
                "SELECT m.id, m.source_key, m.message_id, m.conversation_id, m.folder,
                        m.subject, m.sender, m.recipients, m.sent_at, m.body, m.attachment_names
                 FROM messages m
                 JOIN indexed_sources s ON s.source_key = m.source_key
                 WHERE s.enabled = 1
                   AND NOT EXISTS (
                     SELECT 1 FROM source_folder_settings sf
                     WHERE sf.source_key = m.source_key AND sf.folder = m.folder AND sf.enabled = 0
                   )",
            );
        } else {
            sql.push_str(
                "SELECT m.id, m.source_key, m.message_id, m.conversation_id, m.folder,
                        m.subject, m.sender, m.recipients, m.sent_at, m.body, m.attachment_names
                 FROM messages_fts f
                 JOIN messages m ON m.id = f.rowid
                 JOIN indexed_sources s ON s.source_key = m.source_key
                 WHERE messages_fts MATCH ?
                   AND s.enabled = 1
                   AND NOT EXISTS (
                     SELECT 1 FROM source_folder_settings sf
                     WHERE sf.source_key = m.source_key AND sf.folder = m.folder AND sf.enabled = 0
                   )",
            );
            bind_values.push(Value::Text(fts_query.clone()));
        }

        match scope {
            MailScope::AllMessages => {}
            MailScope::Source(source_key) => {
                sql.push_str(" AND m.source_key = ?");
                bind_values.push(Value::Text(source_key.clone()));
            }
            MailScope::Folder {
                source_key,
                folder_prefix,
            } => {
                sql.push_str(
                    " AND m.source_key = ? AND (m.folder = ? OR (m.folder > ? AND m.folder < ?))",
                );
                let slash_lower = format!("{folder_prefix}/");
                let slash_upper = format!("{folder_prefix}0");
                bind_values.push(Value::Text(source_key.clone()));
                bind_values.push(Value::Text(folder_prefix.clone()));
                bind_values.push(Value::Text(slash_lower));
                bind_values.push(Value::Text(slash_upper));
            }
        }

        match label_filter {
            LabelFilter::Any => {}
            LabelFilter::Labeled(target) => {
                sql.push_str(
                    " AND EXISTS (
                        SELECT 1 FROM message_labels ml
                        WHERE ml.message_id = m.id AND ml.label = ?
                      )",
                );
                bind_values.push(Value::Text(target.clone()));
            }
            LabelFilter::Unlabeled => {
                sql.push_str(
                    " AND NOT EXISTS (
                        SELECT 1 FROM message_labels ml
                        WHERE ml.message_id = m.id
                      )",
                );
            }
        }

        if fts_query.is_empty() {
            sql.push_str(&format!(" ORDER BY {order_clause} LIMIT ?"));
        } else {
            sql.push_str(&format!(
                " ORDER BY bm25(messages_fts, 8.0, 3.0, 2.0, 1.0, 1.0), {order_clause} LIMIT ?"
            ));
        }
        bind_values.push(Value::Integer(limit as i64));

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(bind_values), map_email_record)?;
        let mut records: Vec<EmailRecord> = rows.collect::<rusqlite::Result<Vec<_>>>()?;

        self.attach_labels(&mut records)?;
        Ok(records)
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

    #[cfg(test)]
    pub fn set_folder_enabled(&self, source_key: &str, folder: &str, enabled: bool) -> Result<()> {
        self.conn.execute(
            "INSERT INTO source_folder_settings(source_key, folder, enabled)
             VALUES(?1, ?2, ?3)
             ON CONFLICT(source_key, folder) DO UPDATE SET enabled=excluded.enabled",
            params![source_key, folder, enabled],
        )?;
        Ok(())
    }

    pub fn set_folders_enabled(
        &mut self,
        source_key: &str,
        folders: &[String],
        enabled: bool,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut statement = tx.prepare(
                "INSERT INTO source_folder_settings(source_key, folder, enabled)
                 VALUES(?1, ?2, ?3)
                 ON CONFLICT(source_key, folder) DO UPDATE SET enabled=excluded.enabled",
            )?;
            for folder in folders {
                statement.execute(params![source_key, folder, enabled])?;
            }
        }
        tx.commit()?;
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

fn map_email_record(r: &rusqlite::Row<'_>) -> rusqlite::Result<EmailRecord> {
    Ok(EmailRecord {
        id: r.get(0)?,
        source_key: r.get(1)?,
        message_id: r.get(2)?,
        conversation_id: r.get(3)?,
        folder: r.get(4)?,
        subject: r.get(5)?,
        sender: r.get(6)?,
        recipients: r.get(7)?,
        sent_at: r.get(8)?,
        body: r.get(9)?,
        attachment_names: r.get(10)?,
        labels: Vec::new(),
    })
}

pub(super) fn build_fts_query(query: &str) -> String {
    query
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .map(|s| format!("\"{}\"", s.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR ")
}

pub(super) fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}
