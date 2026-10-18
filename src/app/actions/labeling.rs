use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

use crate::{
    db::Index,
    laya::LayaModel,
    llm,
    model::{EmailRecord, LabelDefinition, LabelFilter, LabelerBackend},
};

use super::{
    super::{LabelOutcome, PstRagApp, WorkerEvent, WorkerResult},
    CUSTOM_LABEL_PALETTE,
};

impl PstRagApp {
    pub(in crate::app) fn label_color(&self, label_name: &str) -> &str {
        self.labels_with_counts
            .iter()
            .find(|(def, _)| def.name == label_name)
            .map(|(def, _)| def.color.as_str())
            .unwrap_or("#2563eb")
    }

    pub(in crate::app) fn label_selected_message(&mut self, backend: LabelerBackend) {
        if self.busy {
            return;
        }
        let Some(selected) = self.selected_email().cloned() else {
            self.status = "Select an email in the explorer first".into();
            return;
        };
        self.run_labeling_job(vec![selected], backend);
    }

    pub(in crate::app) fn label_checked_messages(&mut self, backend: LabelerBackend) {
        if self.busy {
            return;
        }
        if self.checked_message_ids.is_empty() {
            self.label_selected_message(backend);
            return;
        }
        let mut ordered_ids: Vec<i64> = self
            .explorer_messages
            .iter()
            .filter(|m| self.checked_message_ids.contains(&m.id))
            .map(|m| m.id)
            .collect();
        for &id in &self.checked_message_ids {
            if !ordered_ids.contains(&id) {
                ordered_ids.push(id);
            }
        }
        let batch =
            match Index::open(&self.db_path).and_then(|db| db.get_messages_by_ids(&ordered_ids)) {
                Ok(records) if !records.is_empty() => records,
                Ok(_) => {
                    self.status = "No matching emails found for selection".into();
                    return;
                }
                Err(error) => {
                    self.status = format!("Could not load selected emails: {error:#}");
                    return;
                }
            };
        self.run_labeling_job(batch, backend);
    }

    pub(in crate::app) fn label_visible_messages(&mut self) {
        if self.busy {
            return;
        }
        if self.explorer_messages.is_empty() {
            self.status = "No messages in the current explorer view to label".into();
            return;
        }
        let batch: Vec<EmailRecord> = self.explorer_messages.iter().take(50).cloned().collect();
        self.run_labeling_job(batch, self.config.labeler_backend);
    }

    fn run_labeling_job(&mut self, emails: Vec<EmailRecord>, backend: LabelerBackend) {
        let db_path = self.db_path.clone();
        let config = self.config.clone();
        let laya_path = self.active_laya_gguf_path();
        let total = emails.len();
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.worker = Some(rx);
        self.cancel_flag = Some(cancel.clone());
        self.busy = true;
        self.status = format!(
            "Labeling {} email{} with {}...",
            total,
            if total == 1 { "" } else { "s" },
            backend.display_name()
        );
        thread::spawn(move || {
            let result = (|| {
                let mut db = Index::open(&db_path)?;
                let definitions = db.label_definitions()?;
                if definitions.is_empty() {
                    anyhow::bail!("no label definitions configured");
                }
                let mut total_assigned = 0usize;
                let mut processed = 0usize;
                let source_name = match backend {
                    LabelerBackend::Laya => {
                        let model = LayaModel::load_or_embedded(laya_path.as_deref())?;
                        for (idx, email) in emails.iter().enumerate() {
                            if cancel.load(Ordering::Relaxed) {
                                break;
                            }
                            if total > 1 {
                                let _ = tx.send(WorkerEvent::Progress(format!(
                                    "Labeling email {} of {} with Laya...",
                                    idx + 1,
                                    total
                                )));
                            }
                            let assigned = model.label_email(email, &definitions)?;
                            total_assigned += assigned.len();
                            db.set_message_labels(email.id, &assigned)?;
                            let _ = tx.send(WorkerEvent::EmailLabeled {
                                message_id: email.id,
                                labels: assigned,
                            });
                            processed += 1;
                        }
                        if model.is_embedded {
                            "laya-packaged".to_string()
                        } else {
                            "laya-gguf".to_string()
                        }
                    }
                    LabelerBackend::LlmJudge => {
                        for (idx, email) in emails.iter().enumerate() {
                            if cancel.load(Ordering::Relaxed) {
                                break;
                            }
                            let _ = tx.send(WorkerEvent::Progress(format!(
                                "Labeling email {} of {} with LLM Judge...",
                                idx + 1,
                                total
                            )));
                            let assigned = llm::judge_labels(&config, email, &definitions)?;
                            total_assigned += assigned.len();
                            db.set_message_labels(email.id, &assigned)?;
                            let _ = tx.send(WorkerEvent::EmailLabeled {
                                message_id: email.id,
                                labels: assigned,
                            });
                            processed += 1;
                        }
                        "llm-judge".to_string()
                    }
                };
                Ok(LabelOutcome {
                    messages_processed: processed,
                    labels_assigned: total_assigned,
                    source_name,
                    cancelled: cancel.load(Ordering::Relaxed),
                })
            })()
            .map_err(|error: anyhow::Error| format!("{error:#}"));
            let _ = tx.send(WorkerEvent::Finished(WorkerResult::Labeled(result)));
        });
    }

    pub(in crate::app) fn toggle_manual_label(&mut self, message_id: i64, label: &str) {
        match Index::open(&self.db_path).and_then(|db| db.toggle_manual_label(message_id, label)) {
            Ok(added) => {
                self.refresh_explorer();
                self.status = if added {
                    format!("Applied label '{label}'")
                } else {
                    format!("Removed label '{label}'")
                };
            }
            Err(error) => {
                self.status = format!("Could not update label '{label}': {error:#}");
            }
        }
    }

    pub(in crate::app) fn toggle_manual_label_on_checked(&mut self, label: &str) {
        if self.checked_message_ids.is_empty() {
            self.status = "Check one or more emails first".into();
            return;
        }
        let ids: Vec<i64> = self.checked_message_ids.iter().copied().collect();
        let all_have_label =
            match Index::open(&self.db_path).and_then(|db| db.get_messages_by_ids(&ids)) {
                Ok(records) => {
                    !records.is_empty()
                        && records
                            .iter()
                            .all(|m| m.labels.iter().any(|l| l.label == label))
                }
                Err(_) => false,
            };
        let assign = !all_have_label;
        match Index::open(&self.db_path)
            .and_then(|mut db| db.set_manual_label_for_messages(&ids, label, assign))
        {
            Ok(_) => {
                self.refresh_explorer();
                self.status = if assign {
                    format!("Applied label '{label}' to {} selected emails", ids.len())
                } else {
                    format!("Removed label '{label}' from {} selected emails", ids.len())
                };
            }
            Err(error) => {
                self.status = format!("Could not update label '{label}' on selection: {error:#}");
            }
        }
    }

    pub(in crate::app) fn save_label_definition(&mut self) {
        let name = self.new_label_name.trim().to_string();
        if name.is_empty() {
            self.status = "Enter a label name first".into();
            return;
        }
        let description = if self.new_label_description.trim().is_empty() {
            format!("Emails related to {name}.")
        } else {
            self.new_label_description.trim().to_string()
        };
        let color = CUSTOM_LABEL_PALETTE
            [self.labels_with_counts.len() % CUSTOM_LABEL_PALETTE.len()]
        .to_string();
        let def = LabelDefinition {
            name: name.clone(),
            description,
            color,
        };
        match Index::open(&self.db_path).and_then(|db| db.upsert_label_definition(&def)) {
            Ok(()) => {
                self.new_label_name.clear();
                self.new_label_description.clear();
                self.refresh_explorer();
                self.status = format!("Saved label '{name}'");
            }
            Err(error) => {
                self.status = format!("Could not save label '{name}': {error:#}");
            }
        }
    }

    pub(in crate::app) fn remove_label_definition(&mut self, name: &str) {
        match Index::open(&self.db_path).and_then(|mut db| db.remove_label_definition(name)) {
            Ok(()) => {
                if self.label_filter == LabelFilter::Labeled(name.to_string()) {
                    self.label_filter = LabelFilter::Any;
                }
                self.refresh_explorer();
                self.status = format!("Removed label '{name}'");
            }
            Err(error) => {
                self.status = format!("Could not remove label '{name}': {error:#}");
            }
        }
    }
}
