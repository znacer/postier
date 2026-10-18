use anyhow::Result;
use rusqlite::{OptionalExtension, params};

use crate::model::{AssignedLabel, EmailRecord, LabelDefinition, SearchHit};

use super::Index;

impl Index {
    pub(super) fn attach_labels(&self, records: &mut [EmailRecord]) -> Result<()> {
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
}
