mod labeling;

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::{db::Index, laya, llm, model::EmailRecord, pst};

use super::{
    AppTask, ChatMessage, ChatRole, IndexOutcome, Message, PstRagApp, WorkerEvent, WorkerResult,
    chat::chat_question_input_id,
};

const CUSTOM_LABEL_PALETTE: &[&str] = &[
    "#0f766e", "#2563eb", "#7c3aed", "#d97706", "#be123c", "#4f46e5", "#15803d",
];

pub(super) fn emit_worker_event(
    tx: &tokio::sync::mpsc::UnboundedSender<WorkerEvent>,
    event: WorkerEvent,
) {
    let _ = tx.send(event);
}

pub(super) fn spawn_worker(
    work: impl FnOnce(&tokio::sync::mpsc::UnboundedSender<WorkerEvent>) -> WorkerResult + Send + 'static,
) -> AppTask {
    AppTask::Worker(Box::new(work))
}

impl PstRagApp {
    pub(super) fn db_mut(&mut self) -> anyhow::Result<&mut Index> {
        if self.db.is_none() {
            self.db = Some(Index::open(&self.db_path)?);
        }
        Ok(self.db.as_mut().expect("db initialized"))
    }

    pub(super) fn choose_storage_dir(&self) -> AppTask {
        if self.busy {
            return AppTask::none();
        }
        let dir = self.storage_dir.clone();
        AppTask::perform(
            async move {
                rfd::AsyncFileDialog::new()
                    .set_directory(&dir)
                    .pick_folder()
                    .await
                    .map(PathBuf::from)
            },
            Message::StorageDirChosen,
        )
    }

    pub(super) fn apply_storage_dir(&mut self, path: PathBuf) {
        self.storage_dir = path;
        self.db_path = self.storage_dir.join("index.sqlite");
        self.db = Index::open(&self.db_path).ok();
        self.config = load_config(&self.storage_dir).unwrap_or_default();
        self.count = self.db.as_ref().and_then(|d| d.count().ok()).unwrap_or(0);
        self.checked_message_ids.clear();
        self.range_anchor_id = None;
        self.refresh_sources();
        self.refresh_explorer();
        self.status = match fs::write(
            &self.storage_location_path,
            self.storage_dir.to_string_lossy().as_bytes(),
        ) {
            Ok(()) => self
                .config
                .language
                .status_storage_changed(&self.storage_dir.display().to_string()),
            Err(error) => self
                .config
                .language
                .status_storage_saved_err(&error.to_string()),
        };
    }

    pub(super) fn save_credentials(&mut self) {
        self.status = match save_config(&self.storage_dir, &self.config) {
            Ok(()) => self
                .config
                .language
                .status_settings_saved(&self.storage_dir.display().to_string()),
            Err(error) => self
                .config
                .language
                .status_settings_save_err(&format!("{error:#}")),
        };
    }

    pub(super) fn choose_archive(&self) -> AppTask {
        if self.busy {
            return AppTask::none();
        }
        AppTask::perform(
            async {
                rfd::AsyncFileDialog::new()
                    .add_filter("Outlook archive", &["pst"])
                    .add_filter("Development fixture", &["jsonl"])
                    .pick_file()
                    .await
                    .map(PathBuf::from)
            },
            Message::ArchiveChosen,
        )
    }

    pub(super) fn choose_laya_gguf(&self) -> AppTask {
        if self.busy {
            return AppTask::none();
        }
        let dir = self.storage_dir.clone();
        AppTask::perform(
            async move {
                rfd::AsyncFileDialog::new()
                    .set_directory(&dir)
                    .add_filter("GGUF model weights", &["gguf"])
                    .pick_file()
                    .await
                    .map(PathBuf::from)
            },
            Message::LayaGgufChosen,
        )
    }

    pub(super) fn apply_laya_gguf(&mut self, path: PathBuf) {
        self.config.laya_gguf_path = path.to_string_lossy().to_string();
        let _ = save_config(&self.storage_dir, &self.config);
        self.status = self
            .config
            .language
            .status_selected_laya_gguf(&path.display().to_string());
    }

    pub(super) fn active_laya_gguf_path(&self) -> Option<PathBuf> {
        let configured = self.config.laya_gguf_path.trim();
        if !configured.is_empty() {
            let path = PathBuf::from(configured);
            if path.exists() {
                return Some(path);
            }
        }
        let default = laya::default_gguf_path(&self.storage_dir);
        if default.exists() {
            return Some(default);
        }
        None
    }

    pub(super) fn index(&mut self) -> AppTask {
        if self.busy {
            return AppTask::none();
        }
        let Some(path) = self.archive.clone() else {
            self.status = self.config.language.status_select_archive_first().into();
            return AppTask::none();
        };
        let db_path = self.db_path.clone();
        let config = self.config.clone();
        if config.embeddings_active() && config.embedding_model.trim().is_empty() {
            self.status = self
                .config
                .language
                .status_enter_embedding_model_first()
                .into();
            return AppTask::none();
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel_flag = Some(cancel.clone());
        self.busy = true;
        self.status = match self.config.language {
            crate::i18n::Language::English => {
                if config.embeddings_active() {
                    "Importing archive and indexing BM25 locally...".into()
                } else {
                    "Indexing BM25 locally...".into()
                }
            }
            crate::i18n::Language::French => {
                if config.embeddings_active() {
                    "Importation de l'archive et indexation BM25 locale en cours...".into()
                } else {
                    "Indexation BM25 locale en cours...".into()
                }
            }
        };
        spawn_worker(move |tx| {
            let result = (|| {
                let mut db = Index::open(&db_path)?;
                let mut added = 0usize;
                let mut parsed = 0usize;
                pst::import_streaming(&path, |batch| {
                    if cancel.load(Ordering::Relaxed) {
                        anyhow::bail!("{}", config.language.agent_cancelled());
                    }
                    parsed += batch.len();
                    added += db.index_batch(&batch)?;
                    emit_worker_event(
                        tx,
                        WorkerEvent::Progress(format!(
                            "Indexed {added} new emails ({parsed} parsed)..."
                        )),
                    );
                    Ok(())
                })?;
                let mut embedded = 0;
                if config.embeddings_active() && !cancel.load(Ordering::Relaxed) {
                    let model = config.embedding_model.trim();
                    let documents = db.documents_missing_embeddings(model)?;
                    let total_docs = documents.len();
                    for batch in documents.chunks(64) {
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        emit_worker_event(
                            tx,
                            WorkerEvent::Progress(format!(
                                "Generating remote embeddings ({embedded}/{total_docs})..."
                            )),
                        );
                        let vectors = llm::embed(
                            &config,
                            batch.iter().map(|document| document.text.clone()).collect(),
                        )?;
                        if vectors.len() != batch.len() {
                            anyhow::bail!(
                                "embedding provider returned {} vectors for {} emails",
                                vectors.len(),
                                batch.len()
                            );
                        }
                        let values: Vec<_> = batch
                            .iter()
                            .zip(vectors)
                            .map(|(document, vector)| (document.id, vector))
                            .collect();
                        db.store_embeddings(model, &values)?;
                        embedded += values.len();
                    }
                }
                Ok(IndexOutcome { added, embedded })
            })()
            .map_err(|error: anyhow::Error| format!("{error:#}"));
            WorkerResult::Indexed(result)
        })
    }

    pub(super) fn ask(&mut self) -> AppTask {
        if self.busy {
            return AppTask::none();
        }
        if self.question.trim().is_empty() {
            self.status = self.config.language.status_enter_question_first().into();
            return AppTask::none();
        }
        let question = self.question.trim().to_owned();
        self.chat_open = true;
        let mut completed_pairs = Vec::new();
        let mut idx = 0;
        while idx + 1 < self.messages.len() {
            let first = &self.messages[idx];
            let second = &self.messages[idx + 1];
            if matches!(first.role, ChatRole::User)
                && matches!(second.role, ChatRole::Assistant)
                && !second.context_preview.is_empty()
            {
                completed_pairs.push((first.content.clone(), second.content.clone()));
                idx += 2;
            } else {
                idx += 1;
            }
        }
        let history: Vec<llm::ChatTurn> = completed_pairs
            .into_iter()
            .rev()
            .take(3)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .flat_map(|(user_msg, assistant_msg)| {
                [
                    llm::ChatTurn {
                        role: "user".into(),
                        content: user_msg,
                    },
                    llm::ChatTurn {
                        role: "assistant".into(),
                        content: assistant_msg,
                    },
                ]
            })
            .collect();
        self.messages.push(ChatMessage::user(question.clone()));
        self.question.clear();
        let db_path = self.db_path.clone();
        let config = self.config.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel_flag = Some(cancel.clone());
        self.busy = true;
        self.status = self.config.language.agent_progress_thinking().into();
        spawn_worker(move |tx| {
            let tx_clone = tx.clone();
            let progress: llm::ProgressFn = Arc::new(move |msg| {
                emit_worker_event(&tx_clone, WorkerEvent::Progress(msg));
            });
            let result = (|| {
                let answer =
                    llm::run_agent(&config, &db_path, &question, &history, progress, cancel)?;
                Ok(Arc::new((answer.answer, answer.evidence, answer.trace)))
            })()
            .map_err(|error: anyhow::Error| format!("{error:#}"));
            WorkerResult::Answered(result)
        })
    }

    pub(super) fn ask_about_email(&mut self, message_id: i64) -> AppTask {
        self.select_email(message_id);
        self.chat_open = true;
        if let Some(email) = self.selected_email() {
            self.question = match self.config.language {
                crate::i18n::Language::English => format!(
                    "Summarize the email \"{}\" from {} and list any concrete action items or decisions.",
                    email.subject, email.sender
                ),
                crate::i18n::Language::French => format!(
                    "Résume le courriel « {} » de {} et liste les actions concrètes ou décisions éventuelles.",
                    email.subject, email.sender
                ),
            };
            self.status = self.config.language.status_prepared_prompt_email().into();
        }
        AppTask::focus(chat_question_input_id())
    }

    pub(super) fn search_locally(&mut self) -> AppTask {
        if self.busy {
            return AppTask::none();
        }
        if self.question.trim().is_empty() {
            self.status = self
                .config
                .language
                .status_enter_search_terms_first()
                .into();
            return AppTask::none();
        }
        let query = self.question.trim().to_owned();
        self.chat_open = true;
        self.messages.push(ChatMessage::user(query.clone()));
        self.question.clear();
        let db_path = self.db_path.clone();
        self.cancel_flag = Some(Arc::new(AtomicBool::new(false)));
        self.busy = true;
        self.status = self.config.language.status_searching_locally().into();
        spawn_worker(move |_tx| {
            let result = Index::open(&db_path)
                .and_then(|db| db.search(&query, 50))
                .map(Arc::new)
                .map_err(|error| format!("{error:#}"));
            WorkerResult::Searched(result)
        })
    }

    pub(super) fn new_chat(&mut self) {
        self.messages.clear();
        self.hits.clear();
        self.show_sources = false;
        self.expanded_hit = None;
        self.question.clear();
        self.status = self.config.language.status_new_conversation().into();
    }

    pub(super) fn refresh_sources(&mut self) {
        self.sources = self
            .db_mut()
            .and_then(|db| db.sources())
            .unwrap_or_default();
    }

    pub(super) fn refresh_explorer(&mut self) {
        let Ok(db) = self.db_mut() else {
            return;
        };
        let count = db.count().unwrap_or(0);
        let labels_with_counts = db.label_definitions_with_counts().unwrap_or_default();
        let unlabeled_count = db.unlabeled_count().unwrap_or(0);
        self.count = count;
        self.labels_with_counts = labels_with_counts;
        self.unlabeled_count = unlabeled_count;
        self.refresh_explorer_messages();
    }

    pub(super) fn refresh_explorer_messages(&mut self) {
        if self.db.is_none() {
            self.db = Index::open(&self.db_path).ok();
        }
        let Some(db) = self.db.as_ref() else {
            return;
        };
        let messages = db
            .list_messages(
                &self.mail_scope,
                &self.label_filter,
                &self.filter_query,
                self.mail_sort,
                300,
            )
            .unwrap_or_default();
        self.explorer_messages = messages;

        if let Some(selected_id) = self.selected_message_id {
            if !self
                .explorer_messages
                .iter()
                .any(|message| message.id == selected_id)
            {
                self.selected_message_id = self.explorer_messages.first().map(|m| m.id);
            }
        } else {
            self.selected_message_id = self.explorer_messages.first().map(|m| m.id);
        }
    }

    pub(super) fn select_email(&mut self, id: i64) {
        self.selected_message_id = Some(id);
        self.range_anchor_id = Some(id);
        if !self.explorer_messages.iter().any(|email| email.id == id)
            && let Ok(db) = self.db_mut()
            && let Ok(Some(record)) = db.get_message(id)
        {
            self.explorer_messages.insert(0, record);
        }
    }

    pub(super) fn select_adjacent_email(&mut self, delta: isize, extend_range: bool) {
        if self.explorer_messages.is_empty() {
            return;
        }
        let current_idx = self
            .selected_message_id
            .and_then(|id| self.explorer_messages.iter().position(|m| m.id == id))
            .unwrap_or(0);
        let next_idx = (current_idx as isize + delta)
            .clamp(0, self.explorer_messages.len().saturating_sub(1) as isize)
            as usize;
        if let Some(email) = self.explorer_messages.get(next_idx) {
            let next_id = email.id;
            if extend_range {
                self.select_range_to(next_id);
            } else {
                self.selected_message_id = Some(next_id);
                self.range_anchor_id = Some(next_id);
            }
        }
    }

    pub(super) fn toggle_check_selected_email(&mut self) {
        let Some(id) = self.selected_message_id else {
            return;
        };
        let is_checked = self.checked_message_ids.contains(&id);
        self.toggle_check_email(id, !is_checked);
    }

    pub(super) fn toggle_check_email(&mut self, id: i64, checked: bool) {
        self.range_anchor_id = Some(id);
        if checked {
            self.checked_message_ids.insert(id);
            if self.selected_message_id.is_none() {
                self.selected_message_id = Some(id);
            }
        } else {
            self.checked_message_ids.remove(&id);
        }
    }

    pub(super) fn toggle_check_all_visible(&mut self, checked: bool) {
        if checked {
            for email in &self.explorer_messages {
                self.checked_message_ids.insert(email.id);
            }
            self.status = self
                .config
                .language
                .status_selected_visible_count(self.explorer_messages.len());
        } else {
            for email in &self.explorer_messages {
                self.checked_message_ids.remove(&email.id);
            }
            self.status = self.config.language.status_deselected_visible().into();
        }
    }

    pub(super) fn select_unlabeled_visible(&mut self) {
        let mut added = 0usize;
        for email in &self.explorer_messages {
            if email.labels.is_empty() && self.checked_message_ids.insert(email.id) {
                added += 1;
            }
        }
        self.status = self
            .config
            .language
            .status_selected_unlabeled(added, self.checked_message_ids.len());
    }

    pub(super) fn select_range_to(&mut self, target_id: i64) {
        let anchor_id = self
            .range_anchor_id
            .or(self.selected_message_id)
            .unwrap_or(target_id);
        let anchor_idx = self
            .explorer_messages
            .iter()
            .position(|m| m.id == anchor_id);
        let target_idx = self
            .explorer_messages
            .iter()
            .position(|m| m.id == target_id);
        if let (Some(a), Some(b)) = (anchor_idx, target_idx) {
            let (start, end) = if a <= b { (a, b) } else { (b, a) };
            for email in &self.explorer_messages[start..=end] {
                self.checked_message_ids.insert(email.id);
            }
            self.selected_message_id = Some(target_id);
            self.range_anchor_id = Some(anchor_id);
            self.status = self
                .config
                .language
                .status_selected_range(end - start + 1, self.checked_message_ids.len());
        } else {
            self.checked_message_ids.insert(target_id);
            self.selected_message_id = Some(target_id);
            self.range_anchor_id = Some(target_id);
        }
    }

    pub(super) fn selected_email(&self) -> Option<&EmailRecord> {
        let id = self.selected_message_id?;
        self.explorer_messages.iter().find(|email| email.id == id)
    }

    pub(super) fn set_source_enabled(&mut self, source: &str, enabled: bool) {
        let res = self
            .db_mut()
            .and_then(|db| db.set_source_enabled(source, enabled));
        self.status = match res {
            Ok(()) => {
                self.refresh_sources();
                self.refresh_explorer();
                if enabled {
                    self.config.language.status_pst_enabled().into()
                } else {
                    self.config.language.status_pst_excluded().into()
                }
            }
            Err(error) => format!("Could not update PST selection: {error:#}"),
        };
    }

    pub(super) fn set_folders_enabled(&mut self, source: &str, folders: &[String], enabled: bool) {
        let res = self
            .db_mut()
            .and_then(|db| db.set_folders_enabled(source, folders, enabled));
        self.status = match res {
            Ok(()) => {
                self.refresh_sources();
                self.refresh_explorer();
                if enabled {
                    self.config.language.status_folders_enabled().into()
                } else {
                    self.config.language.status_folders_excluded().into()
                }
            }
            Err(error) => format!("Could not update folder tree selection: {error:#}"),
        };
    }

    pub(super) fn remove_pending_source(&mut self) {
        let Some(source) = self.pending_remove_source.take() else {
            return;
        };
        let res = self.db_mut().and_then(|db| db.remove_source(&source));
        self.status = match res {
            Ok(removed) => {
                self.count = self.count.saturating_sub(removed as u64);
                self.checked_message_ids.clear();
                self.range_anchor_id = None;
                self.refresh_sources();
                self.refresh_explorer();
                self.config.language.status_pst_removed(removed)
            }
            Err(error) => self
                .config
                .language
                .status_pst_remove_err(&format!("{error:#}")),
        };
    }
}

pub(super) fn load_config(storage_dir: &Path) -> anyhow::Result<crate::llm::LlmConfig> {
    let path = storage_dir.join("credentials.json");
    if !path.exists() {
        return Ok(crate::llm::LlmConfig::default());
    }
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

pub(super) fn save_config(
    storage_dir: &Path,
    config: &crate::llm::LlmConfig,
) -> anyhow::Result<()> {
    fs::create_dir_all(storage_dir)?;
    let path = storage_dir.join("credentials.json");
    let bytes = serde_json::to_vec_pretty(config)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(&bytes)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        fs::write(&path, bytes)?;
    }
    Ok(())
}
