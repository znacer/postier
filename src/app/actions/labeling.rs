use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use iced::Task;

use crate::{
    db::Index,
    laya::LayaModel,
    llm,
    model::{EmailRecord, LabelDefinition, LabelFilter, LabelerBackend},
};

use super::{
    super::{LabelOutcome, Message, PstRagApp, WorkerEvent, WorkerResult},
    CUSTOM_LABEL_PALETTE, emit_worker_event, spawn_worker,
};

impl PstRagApp {
    pub(in crate::app) fn label_color(&self, label_name: &str) -> &str {
        self.labels_with_counts
            .iter()
            .find(|(def, _)| def.name == label_name)
            .map(|(def, _)| def.color.as_str())
            .unwrap_or("#2563eb")
    }

    pub(in crate::app) fn label_selected_message(
        &mut self,
        backend: LabelerBackend,
    ) -> Task<Message> {
        if self.busy {
            return Task::none();
        }
        let Some(selected) = self.selected_email().cloned() else {
            self.status = self.config.language.status_labeling_select_first().into();
            return Task::none();
        };
        self.run_labeling_job(vec![selected], backend)
    }

    pub(in crate::app) fn label_checked_messages(
        &mut self,
        backend: LabelerBackend,
    ) -> Task<Message> {
        if self.busy {
            return Task::none();
        }
        if self.checked_message_ids.is_empty() {
            return self.label_selected_message(backend);
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
                    self.status = self.config.language.status_labeling_no_matches().into();
                    return Task::none();
                }
                Err(error) => {
                    self.status = self
                        .config
                        .language
                        .status_labeling_load_err(&format!("{error:#}"));
                    return Task::none();
                }
            };
        self.run_labeling_job(batch, backend)
    }

    pub(in crate::app) fn label_visible_messages(&mut self) -> Task<Message> {
        if self.busy {
            return Task::none();
        }
        if self.explorer_messages.is_empty() {
            self.status = self.config.language.status_labeling_no_visible().into();
            return Task::none();
        }
        let batch: Vec<EmailRecord> = self.explorer_messages.iter().take(50).cloned().collect();
        self.run_labeling_job(batch, self.config.labeler_backend)
    }

    fn run_labeling_job(
        &mut self,
        emails: Vec<EmailRecord>,
        backend: LabelerBackend,
    ) -> Task<Message> {
        let db_path = self.db_path.clone();
        let config = self.config.clone();
        let laya_path = self.active_laya_gguf_path();
        let total = emails.len();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel_flag = Some(cancel.clone());
        self.busy = true;
        self.status = self
            .config
            .language
            .status_labeling_emails_with(total, backend.display_name());
        spawn_worker(move |tx| {
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
                                emit_worker_event(
                                    tx,
                                    WorkerEvent::Progress(config.language.status_labeling_progress(
                                        idx + 1,
                                        total,
                                        "Laya",
                                    )),
                                );
                            }
                            let assigned = model.label_email(email, &definitions)?;
                            total_assigned += assigned.len();
                            db.set_message_labels(email.id, &assigned)?;
                            emit_worker_event(
                                tx,
                                WorkerEvent::EmailLabeled {
                                    message_id: email.id,
                                    labels: assigned,
                                },
                            );
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
                            emit_worker_event(
                                tx,
                                WorkerEvent::Progress(config.language.status_labeling_progress(
                                    idx + 1,
                                    total,
                                    "LLM Judge",
                                )),
                            );
                            let assigned = llm::judge_labels(&config, email, &definitions)?;
                            total_assigned += assigned.len();
                            db.set_message_labels(email.id, &assigned)?;
                            emit_worker_event(
                                tx,
                                WorkerEvent::EmailLabeled {
                                    message_id: email.id,
                                    labels: assigned,
                                },
                            );
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
            WorkerResult::Labeled(result)
        })
    }

    pub(in crate::app) fn toggle_manual_label(&mut self, message_id: i64, label: &str) {
        match Index::open(&self.db_path).and_then(|db| db.toggle_manual_label(message_id, label)) {
            Ok(added) => {
                self.refresh_explorer();
                self.status = self.config.language.status_label_updated(label, added);
            }
            Err(error) => {
                self.status = self
                    .config
                    .language
                    .status_label_update_err(label, &format!("{error:#}"));
            }
        }
    }

    pub(in crate::app) fn toggle_manual_label_on_checked(&mut self, label: &str) {
        if self.checked_message_ids.is_empty() {
            self.status = self.config.language.status_check_emails_first().into();
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
                self.status =
                    self.config
                        .language
                        .status_label_batch_updated(label, assign, ids.len());
            }
            Err(error) => {
                self.status = self
                    .config
                    .language
                    .status_label_batch_err(label, &format!("{error:#}"));
            }
        }
    }

    pub(in crate::app) fn save_label_definition(&mut self) {
        let name = self.new_label_name.trim().to_string();
        if name.is_empty() {
            self.status = self.config.language.status_enter_label_name_first().into();
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
                self.status = self.config.language.status_saved_label(&name);
            }
            Err(error) => {
                self.status = self
                    .config
                    .language
                    .status_label_save_err(&name, &format!("{error:#}"));
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
                self.status = self.config.language.status_removed_label(name);
            }
            Err(error) => {
                self.status = self
                    .config
                    .language
                    .status_label_remove_err(name, &format!("{error:#}"));
            }
        }
    }
}
