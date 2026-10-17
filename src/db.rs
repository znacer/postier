use std::{collections::HashMap, path::Path};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params, types::Value};
use sha2::{Digest, Sha256};

use crate::model::{
    AssignedLabel, Email, EmailRecord, EmbeddingDocument, IndexedFolder, IndexedSource,
    LabelDefinition, LabelFilter, MailScope, MailSort, SearchHit, default_label_definitions,
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

    fn attach_labels(&self, records: &mut [EmailRecord]) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let mut label_stmt = self.conn.prepare(
            "SELECT label, confidence, source
             FROM message_labels
             WHERE message_id = ?1
             ORDER BY coalesce(confidence, 1.0) DESC, label ASC",
        )?;
        for record in records {
            let rows = label_stmt.query_map([record.id], |row| {
                Ok(AssignedLabel {
                    label: row.get(0)?,
                    confidence: row.get(1)?,
                    source: row.get(2)?,
                })
            })?;
            record.labels = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        }
        Ok(())
    }

    fn search_fts_raw(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let fts_query = build_fts_query(query);
        if fts_query.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare(
            "SELECT m.id,m.source_key,m.subject,m.sender,m.recipients,m.sent_at,m.folder,m.body,
                    m.attachment_names,m.conversation_id,m.message_id,
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
            let raw_rank: f64 = r.get(11)?;
            Ok(SearchHit {
                id: r.get(0)?,
                source_key: r.get(1)?,
                subject: r.get(2)?,
                sender: r.get(3)?,
                recipients: r.get(4)?,
                sent_at: r.get(5)?,
                folder: r.get(6)?,
                body: r.get(7)?,
                attachment_names: r.get(8)?,
                conversation_id: r.get(9)?,
                message_id: r.get(10)?,
                labels: Vec::new(),
                score: -raw_rank,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let mut hits = self.search_fts_raw(query, limit)?;
        self.attach_search_hit_labels(&mut hits)?;
        Ok(hits)
    }

    pub fn label_definitions_with_counts(&self) -> Result<Vec<(LabelDefinition, u64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT d.name, d.description, d.color, count(ml.message_id)
             FROM label_definitions d
             LEFT JOIN message_labels ml ON ml.label = d.name
             GROUP BY d.name, d.description, d.color
             ORDER BY d.name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                LabelDefinition {
                    name: row.get(0)?,
                    description: row.get(1)?,
                    color: row.get(2)?,
                },
                row.get(3)?,
            ))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn label_definitions(&self) -> Result<Vec<LabelDefinition>> {
        Ok(self
            .label_definitions_with_counts()?
            .into_iter()
            .map(|(def, _)| def)
            .collect())
    }

    pub fn unlabeled_count(&self) -> Result<u64> {
        Ok(self.conn.query_row(
            "SELECT count(*) FROM messages m
             WHERE NOT EXISTS (SELECT 1 FROM message_labels ml WHERE ml.message_id = m.id)",
            [],
            |r| r.get(0),
        )?)
    }

    pub fn upsert_label_definition(&self, def: &LabelDefinition) -> Result<()> {
        let name = def.name.trim();
        if name.is_empty() {
            anyhow::bail!("label name cannot be empty");
        }
        self.conn.execute(
            "INSERT INTO label_definitions(name, description, color)
             VALUES(?1, ?2, ?3)
             ON CONFLICT(name) DO UPDATE SET description=excluded.description, color=excluded.color",
            params![name, def.description.trim(), def.color.trim()],
        )?;
        Ok(())
    }

    pub fn remove_label_definition(&mut self, name: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM message_labels WHERE label = ?1", [name])?;
        tx.execute("DELETE FROM label_definitions WHERE name = ?1", [name])?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_message_labels(&self, message_id: i64) -> Result<Vec<AssignedLabel>> {
        let mut stmt = self.conn.prepare(
            "SELECT label, confidence, source
             FROM message_labels
             WHERE message_id = ?1
             ORDER BY coalesce(confidence, 1.0) DESC, label ASC",
        )?;
        let rows = stmt.query_map([message_id], |row| {
            Ok(AssignedLabel {
                label: row.get(0)?,
                confidence: row.get(1)?,
                source: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn attach_search_hit_labels(&self, hits: &mut [SearchHit]) -> Result<()> {
        if hits.is_empty() {
            return Ok(());
        }
        let mut stmt = self.conn.prepare(
            "SELECT label, confidence, source
             FROM message_labels
             WHERE message_id = ?1
             ORDER BY coalesce(confidence, 1.0) DESC, label ASC",
        )?;
        for hit in hits {
            let rows = stmt.query_map([hit.id], |row| {
                Ok(AssignedLabel {
                    label: row.get(0)?,
                    confidence: row.get(1)?,
                    source: row.get(2)?,
                })
            })?;
            hit.labels = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        }
        Ok(())
    }

    pub fn expand_threads(&self, hits: &mut Vec<SearchHit>, max_total: usize) -> Result<()> {
        if hits.is_empty() || hits.len() >= max_total {
            return Ok(());
        }
        let existing_ids: std::collections::HashSet<i64> = hits.iter().map(|h| h.id).collect();
        let mut thread_conv_ids: Vec<(String, f64)> = Vec::new();
        for hit in hits.iter() {
            let conv = hit.conversation_id.trim();
            if !conv.is_empty() && !thread_conv_ids.iter().any(|(c, _)| c == conv) {
                thread_conv_ids.push((conv.to_string(), hit.score));
            }
        }
        if thread_conv_ids.is_empty() {
            return Ok(());
        }

        let remaining_slots = max_total - hits.len();
        let per_thread_cap = if thread_conv_ids.len() <= 1 {
            remaining_slots
        } else {
            remaining_slots.div_ceil(thread_conv_ids.len()).max(2)
        };

        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.source_key, m.subject, m.sender, m.recipients, m.sent_at, m.folder, m.body,
                    m.attachment_names, m.conversation_id, m.message_id
             FROM messages m
             JOIN indexed_sources s ON s.source_key=m.source_key
             WHERE m.conversation_id = ?1 AND s.enabled=1
               AND NOT EXISTS (
                 SELECT 1 FROM source_folder_settings sf
                 WHERE sf.source_key=m.source_key AND sf.folder=m.folder AND sf.enabled=0
               )
             ORDER BY coalesce(m.sent_at, '') ASC, m.id ASC
             LIMIT 10",
        )?;

        let mut companions = Vec::new();
        for (conv_id, parent_score) in thread_conv_ids {
            let rows = stmt.query_map(params![conv_id], |r| {
                Ok(SearchHit {
                    id: r.get(0)?,
                    source_key: r.get(1)?,
                    subject: r.get(2)?,
                    sender: r.get(3)?,
                    recipients: r.get(4)?,
                    sent_at: r.get(5)?,
                    folder: r.get(6)?,
                    body: r.get(7)?,
                    attachment_names: r.get(8)?,
                    conversation_id: r.get(9)?,
                    message_id: r.get(10)?,
                    labels: Vec::new(),
                    score: parent_score * 0.9,
                })
            })?;
            let mut added_for_thread = 0usize;
            for row in rows {
                let hit = row?;
                if !existing_ids.contains(&hit.id)
                    && !companions.iter().any(|c: &SearchHit| c.id == hit.id)
                {
                    companions.push(hit);
                    added_for_thread += 1;
                    if hits.len() + companions.len() >= max_total
                        || added_for_thread >= per_thread_cap
                    {
                        break;
                    }
                }
            }
            if hits.len() + companions.len() >= max_total {
                break;
            }
        }

        self.attach_search_hit_labels(&mut companions)?;
        hits.extend(companions);
        Ok(())
    }

    pub fn set_message_labels(&mut self, message_id: i64, labels: &[AssignedLabel]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM message_labels WHERE message_id = ?1 AND source != 'manual'",
            [message_id],
        )?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO message_labels(message_id, label, confidence, source)
                 VALUES(?1, ?2, ?3, ?4)",
            )?;
            for item in labels {
                stmt.execute(params![
                    message_id,
                    item.label,
                    item.confidence,
                    item.source
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn toggle_manual_label(&self, message_id: i64, label: &str) -> Result<bool> {
        let exists: bool = self
            .conn
            .query_row(
                "SELECT 1 FROM message_labels WHERE message_id = ?1 AND label = ?2",
                params![message_id, label],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if exists {
            self.conn.execute(
                "DELETE FROM message_labels WHERE message_id = ?1 AND label = ?2",
                params![message_id, label],
            )?;
            Ok(false)
        } else {
            self.conn.execute(
                "INSERT INTO message_labels(message_id, label, confidence, source)
                 VALUES(?1, ?2, NULL, 'manual')",
                params![message_id, label],
            )?;
            Ok(true)
        }
    }

    pub fn set_manual_label_for_messages(
        &mut self,
        message_ids: &[i64],
        label: &str,
        assign: bool,
    ) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let mut affected = 0usize;
        if assign {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO message_labels(message_id, label, confidence, source)
                 VALUES(?1, ?2, NULL, 'manual')",
            )?;
            for &id in message_ids {
                affected += stmt.execute(params![id, label])?;
            }
        } else {
            let mut stmt =
                tx.prepare("DELETE FROM message_labels WHERE message_id = ?1 AND label = ?2")?;
            for &id in message_ids {
                affected += stmt.execute(params![id, label])?;
            }
        }
        tx.commit()?;
        Ok(affected)
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
        let bm25 = self.search_fts_raw(query, 50)?;
        let mut combined: HashMap<i64, (SearchHit, f64)> = HashMap::new();
        for (rank, hit) in bm25.into_iter().enumerate() {
            combined.insert(hit.id, (hit, reciprocal_rank(rank)));
        }

        let mut stmt = self.conn.prepare(
            "SELECT m.id,m.source_key,m.subject,m.sender,m.recipients,m.sent_at,m.folder,m.body,
                    m.attachment_names,m.conversation_id,m.message_id,e.vector
             FROM message_embeddings e JOIN messages m ON m.id=e.message_id
             JOIN indexed_sources s ON s.source_key=m.source_key
             WHERE e.model=?1 AND e.dimensions=?2 AND s.enabled=1
               AND NOT EXISTS (
                 SELECT 1 FROM source_folder_settings sf
                 WHERE sf.source_key=m.source_key AND sf.folder=m.folder AND sf.enabled=0
               )",
        )?;
        let rows = stmt.query_map(params![model, query_embedding.len() as i64], |row| {
            let encoded: Vec<u8> = row.get(11)?;
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
                    attachment_names: row.get(8)?,
                    conversation_id: row.get(9)?,
                    message_id: row.get(10)?,
                    labels: Vec::new(),
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
        self.attach_search_hit_labels(&mut hits)?;
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

fn build_fts_query(query: &str) -> String {
    query
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .map(|s| format!("\"{}\"", s.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR ")
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

    #[test]
    fn manages_labels_and_filters_explorer_messages() {
        let mut db = Index::open(Path::new(":memory:")).unwrap();
        db.index_batch(&[
            Email {
                source_key: "/archives/main.pst".into(),
                folder: "Mailbox/Inbox".into(),
                subject: "Approve Q3 budget".into(),
                sender: "cfo@example.test".into(),
                sent_at: Some("2026-03-01T10:00:00Z".into()),
                body: "Please approve the budget today.".into(),
                ..Default::default()
            },
            Email {
                source_key: "/archives/main.pst".into(),
                folder: "Mailbox/Archive".into(),
                subject: "Weekly digest".into(),
                sender: "news@example.test".into(),
                sent_at: Some("2026-03-02T10:00:00Z".into()),
                body: "General company updates.".into(),
                ..Default::default()
            },
        ])
        .unwrap();

        let all = db
            .list_messages(
                &MailScope::AllMessages,
                &LabelFilter::Any,
                "",
                MailSort::DateDesc,
                100,
            )
            .unwrap();
        assert_eq!(all.len(), 2);
        let budget_id = all
            .iter()
            .find(|m| m.subject.contains("Approve"))
            .unwrap()
            .id;

        db.set_message_labels(
            budget_id,
            &[AssignedLabel {
                label: "Action Required".into(),
                confidence: Some(0.93),
                source: "laya".into(),
            }],
        )
        .unwrap();

        let action_only = db
            .list_messages(
                &MailScope::AllMessages,
                &LabelFilter::Labeled("Action Required".into()),
                "",
                MailSort::DateDesc,
                100,
            )
            .unwrap();
        assert_eq!(action_only.len(), 1);
        assert_eq!(action_only[0].id, budget_id);

        let unlabeled = db
            .list_messages(
                &MailScope::AllMessages,
                &LabelFilter::Unlabeled,
                "",
                MailSort::DateDesc,
                100,
            )
            .unwrap();
        assert_eq!(unlabeled.len(), 1);
        assert_eq!(unlabeled[0].subject, "Weekly digest");

        let selected_ids: Vec<i64> = all.iter().map(|m| m.id).collect();
        assert_eq!(
            db.set_manual_label_for_messages(&selected_ids, "FYI", true)
                .unwrap(),
            2
        );
        let batch_records = db.get_messages_by_ids(&selected_ids).unwrap();
        assert_eq!(batch_records.len(), 2);
        assert!(
            batch_records
                .iter()
                .all(|m| m.labels.iter().any(|l| l.label == "FYI"))
        );
    }

    #[test]
    fn search_populates_rich_metadata_labels_and_expands_threads() {
        let mut db = Index::open(Path::new(":memory:")).unwrap();
        db.index_batch(&[
            Email {
                source_key: "/archives/team.pst".into(),
                folder: "Mailbox/Inbox".into(),
                subject: "Server Migration".into(),
                sender: "lead@example.test".into(),
                recipients: "devs@example.test".into(),
                sent_at: Some("2026-04-01T09:00:00Z".into()),
                body: "We are migrating the database server tonight.".into(),
                attachment_names: "migration_guide.pdf".into(),
                conversation_id: "thread-migration-42".into(),
                message_id: "msg-001@example.test".into(),
            },
            Email {
                source_key: "/archives/team.pst".into(),
                folder: "Mailbox/Inbox".into(),
                subject: "Re: Server Migration".into(),
                sender: "ops@example.test".into(),
                recipients: "lead@example.test".into(),
                sent_at: Some("2026-04-01T10:00:00Z".into()),
                body: "Backup completed successfully. Ready to proceed.".into(),
                attachment_names: "backup_checksums.txt".into(),
                conversation_id: "thread-migration-42".into(),
                message_id: "msg-002@example.test".into(),
            },
        ])
        .unwrap();

        let records = db.search("migrating", 10).unwrap();
        assert_eq!(records.len(), 1);
        let id1 = records[0].id;
        db.set_message_labels(
            id1,
            &[AssignedLabel {
                label: "Urgent".into(),
                confidence: Some(0.99),
                source: "laya".into(),
            }],
        )
        .unwrap();

        let mut hits = db.search("migrating", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].score > 0.0);
        assert_eq!(hits[0].attachment_names, "migration_guide.pdf");
        assert_eq!(hits[0].conversation_id, "thread-migration-42");
        assert_eq!(hits[0].message_id, "msg-001@example.test");
        assert_eq!(hits[0].labels.len(), 1);
        assert_eq!(hits[0].labels[0].label, "Urgent");

        db.expand_threads(&mut hits, 10).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits[1].score > 0.0 && hits[1].score < hits[0].score);
        assert!(hits.iter().any(|h| h.sender == "ops@example.test"));
        assert!(
            hits.iter()
                .any(|h| h.attachment_names == "backup_checksums.txt")
        );
    }
}
