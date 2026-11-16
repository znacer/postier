use anyhow::Result;
use rusqlite::{OptionalExtension, params, types::Value};
use std::collections::HashMap;

use crate::model::{AssignedLabel, EmailRecord, LabelDefinition, SearchHit};

use super::Index;

const IN_CHUNK_SIZE: usize = 500;

impl Index {
    fn fetch_labels_for_ids(&self, ids: &[i64]) -> Result<HashMap<i64, Vec<AssignedLabel>>> {
        let mut by_id: HashMap<i64, Vec<AssignedLabel>> = HashMap::with_capacity(ids.len());
        if ids.is_empty() {
            return Ok(by_id);
        }
        for chunk in ids.chunks(IN_CHUNK_SIZE) {
            let placeholders = vec!["?"; chunk.len()].join(", ");
            let sql = format!(
                "SELECT message_id, label, confidence, source
                 FROM message_labels
                 WHERE message_id IN ({placeholders})
                 ORDER BY message_id ASC, coalesce(confidence, 1.0) DESC, label ASC"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let bind_values = chunk.iter().copied().map(Value::Integer);
            let rows = stmt.query_map(rusqlite::params_from_iter(bind_values), |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    AssignedLabel {
                        label: row.get(1)?,
                        confidence: row.get(2)?,
                        source: row.get(3)?,
                    },
                ))
            })?;
            for row in rows {
                let (message_id, assigned) = row?;
                by_id.entry(message_id).or_default().push(assigned);
            }
        }
        Ok(by_id)
    }

    pub(super) fn attach_labels(&self, records: &mut [EmailRecord]) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let ids: Vec<i64> = records.iter().map(|r| r.id).collect();
        let mut by_id = self.fetch_labels_for_ids(&ids)?;
        for record in records {
            record.labels = by_id.remove(&record.id).unwrap_or_default();
        }
        Ok(())
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
        let ids: Vec<i64> = hits.iter().map(|h| h.id).collect();
        let by_id = self.fetch_labels_for_ids(&ids)?;
        for hit in hits {
            hit.labels = by_id.get(&hit.id).cloned().unwrap_or_default();
        }
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
                "INSERT INTO message_labels(message_id, label, confidence, source)
                 VALUES(?1, ?2, ?3, ?4)
                 ON CONFLICT(message_id, label) DO NOTHING",
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
}
