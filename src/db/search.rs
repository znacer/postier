use std::collections::HashMap;

use anyhow::Result;
use rusqlite::params;

use crate::model::{EmbeddingDocument, SearchHit};

use super::{Index, truncate_chars};

// Embedding storage and hybrid search are kept but currently disabled
// (`llm::EMBEDDINGS_ENABLED`); retrieval is BM25-only via `query.rs`.
#[allow(dead_code)]
impl Index {
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
        let bm25 = self.search(query, 50)?;
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
                    ..Default::default()
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
        self.attach_search_hit_attachments(&mut hits)?;
        Ok(hits)
    }
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
