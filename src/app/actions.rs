use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
};

use anyhow::Context;

use crate::{
    db::Index,
    laya::{self, LayaModel},
    llm,
    model::{EmailRecord, LabelDefinition, LabelFilter, LabelerBackend},
    pst,
};

use super::{ChatMessage, IndexOutcome, LabelOutcome, PstRagApp, WorkerResult};

const CUSTOM_LABEL_PALETTE: &[&str] = &[
    "#0f766e", "#2563eb", "#7c3aed", "#d97706", "#be123c", "#4f46e5", "#15803d",
];

impl PstRagApp {
    pub(super) fn choose_storage_dir(&mut self) {
        if self.busy {
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .set_directory(&self.storage_dir)
            .pick_folder()
        {
            self.storage_dir = path;
            self.db_path = self.storage_dir.join("index.sqlite");
            self.config = load_config(&self.storage_dir).unwrap_or_default();
            self.count = Index::open(&self.db_path)
                .and_then(|d| d.count())
                .unwrap_or(0);
            self.checked_message_ids.clear();
            self.refresh_sources();
            self.refresh_explorer();
            self.status = match fs::write(
                &self.storage_location_path,
                self.storage_dir.to_string_lossy().as_bytes(),
            ) {
                Ok(()) => format!(
                    "Storage directory changed to {}",
                    self.storage_dir.display()
                ),
                Err(error) => format!(
                    "Storage changed for this session, but the selection could not be saved: {error}"
                ),
            };
        }
    }

    pub(super) fn save_credentials(&mut self) {
        if self.busy {
            return;
        }
        self.status = match save_config(&self.storage_dir, &self.config) {
            Ok(()) => format!("Settings saved in {}", self.storage_dir.display()),
            Err(error) => format!("Could not save settings: {error:#}"),
        };
    }

    pub(super) fn choose_archive(&mut self) {
        if self.busy {
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Outlook archive", &["pst"])
            .add_filter("Development fixture", &["jsonl"])
            .pick_file()
        {
            self.archive = Some(path);
        }
    }

    pub(super) fn choose_laya_gguf(&mut self) {
        if self.busy {
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .set_directory(&self.storage_dir)
            .add_filter("GGUF model weights", &["gguf"])
            .pick_file()
        {
            self.config.laya_gguf_path = path.to_string_lossy().to_string();
            let _ = save_config(&self.storage_dir, &self.config);
            self.status = format!("Selected Laya GGUF weights at {}", path.display());
        }
    }

    pub(super) fn download_laya_gguf(&mut self) {
        if self.busy {
            return;
        }
        let target = if self.config.laya_gguf_path.trim().is_empty() {
            laya::default_gguf_path(&self.storage_dir)
        } else {
            PathBuf::from(self.config.laya_gguf_path.trim())
        };
        let (tx, rx) = mpsc::channel();
        self.worker = Some(rx);
        self.busy = true;
        self.status = format!(
            "Downloading ggml-org/Laya-GGUF to {}\u{2026}",
            target.display()
        );
        thread::spawn(move || {
            let result = laya::download_laya_gguf(&target)
                .map(|_| target)
                .map_err(|error| format!("{error:#}"));
            let _ = tx.send(WorkerResult::LayaDownloaded(result));
        });
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

    pub(super) fn index(&mut self) {
        if self.busy {
            return;
        }
        let Some(path) = self.archive.clone() else {
            self.status = "Select an archive first".into();
            return;
        };
        let db_path = self.db_path.clone();
        let config = self.config.clone();
        if config.use_remote_embeddings && config.embedding_model.trim().is_empty() {
            self.status = "Enter an embedding model before enabling remote embeddings".into();
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.worker = Some(rx);
        self.busy = true;
        self.status = if config.use_remote_embeddings {
            "Indexing BM25 locally and generating remote embeddings\u{2026}".into()
        } else {
            "Indexing BM25 locally\u{2026}".into()
        };
        thread::spawn(move || {
            let result = (|| {
                let emails = pst::import(&path)?;
                let mut db = Index::open(&db_path)?;
                let added = db.index_batch(&emails)?;
                let mut embedded = 0;
                if config.use_remote_embeddings {
                    let model = config.embedding_model.trim();
                    let documents = db.documents_missing_embeddings(model)?;
                    for batch in documents.chunks(64) {
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
            let _ = tx.send(WorkerResult::Indexed(result));
        });
    }

    pub(super) fn ask(&mut self) {
        if self.busy {
            return;
        }
        if self.question.trim().is_empty() {
            self.status = "Enter a question first".into();
            return;
        }
        let question = self.question.trim().to_owned();
        self.chat_open = true;
        self.messages.push(ChatMessage::user(question.clone()));
        self.question.clear();
        let db_path = self.db_path.clone();
        let config = self.config.clone();
        let (tx, rx) = mpsc::channel();
        self.worker = Some(rx);
        self.busy = true;
        self.status = "Searching locally\u{2026}".into();
        thread::spawn(move || {
            let result = (|| {
                let db = Index::open(&db_path)?;
                let hits =
                    if config.use_remote_embeddings && !config.embedding_model.trim().is_empty() {
                        let mut embeddings = llm::embed(&config, vec![question.clone()])?;
                        let query_embedding = embeddings
                            .pop()
                            .context("embedding provider returned no query vector")?;
                        db.search_hybrid(
                            &question,
                            &query_embedding,
                            config.embedding_model.trim(),
                            10,
                        )?
                    } else {
                        db.search(&question, 10)?
                    };
                let context = llm::build_context(&hits, config.max_context_chars);
                let answer = llm::ask(&config, &question, &context)?;
                Ok((answer, hits, context))
            })()
            .map_err(|error: anyhow::Error| format!("{error:#}"));
            let _ = tx.send(WorkerResult::Answered(result));
        });
    }

    pub(super) fn ask_about_email(&mut self, message_id: i64) {
        self.select_email(message_id);
        self.chat_open = true;
        if let Some(email) = self.selected_email() {
            self.question = format!(
                "Summarize the email \"{}\" from {} and list any concrete action items or decisions.",
                email.subject, email.sender
            );
            self.status = "Prepared prompt for the selected email in the Chat pane".into();
        }
    }

    pub(super) fn search_locally(&mut self) {
        if self.busy {
            return;
        }
        if self.question.trim().is_empty() {
            self.status = "Enter search terms first".into();
            return;
        }
        let query = self.question.trim().to_owned();
        self.chat_open = true;
        self.messages.push(ChatMessage::user(query.clone()));
        self.question.clear();
        let db_path = self.db_path.clone();
        let (tx, rx) = mpsc::channel();
        self.worker = Some(rx);
        self.busy = true;
        self.status = "Searching locally \u{2014} no network request\u{2026}".into();
        thread::spawn(move || {
            let result = Index::open(&db_path)
                .and_then(|db| db.search(&query, 50))
                .map_err(|error| format!("{error:#}"));
            let _ = tx.send(WorkerResult::Searched(result));
        });
    }

    pub(super) fn new_chat(&mut self) {
        if self.busy {
            return;
        }
        self.messages.clear();
        self.hits.clear();
        self.show_sources = false;
        self.expanded_hit = None;
        self.question.clear();
        self.status = "New conversation \u{2014} the local index is still available".into();
    }

    pub(super) fn refresh_sources(&mut self) {
        self.sources = Index::open(&self.db_path)
            .and_then(|db| db.sources())
            .unwrap_or_default();
    }

    pub(super) fn refresh_explorer(&mut self) {
        let Ok(db) = Index::open(&self.db_path) else {
            return;
        };
        self.count = db.count().unwrap_or(self.count);
        self.labels_with_counts = db.label_definitions_with_counts().unwrap_or_default();
        self.unlabeled_count = db.unlabeled_count().unwrap_or(0);
        self.explorer_messages = db
            .list_messages(
                &self.mail_scope,
                &self.label_filter,
                &self.filter_query,
                self.mail_sort,
                300,
            )
            .unwrap_or_default();

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
        if !self.explorer_messages.iter().any(|email| email.id == id)
            && let Ok(db) = Index::open(&self.db_path)
            && let Ok(Some(record)) = db.get_message(id)
        {
            self.explorer_messages.insert(0, record);
        }
    }

    pub(super) fn toggle_check_email(&mut self, id: i64, checked: bool) {
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
            self.status = format!(
                "Selected {} visible email{}",
                self.explorer_messages.len(),
                if self.explorer_messages.len() == 1 {
                    ""
                } else {
                    "s"
                }
            );
        } else {
            for email in &self.explorer_messages {
                self.checked_message_ids.remove(&email.id);
            }
            self.status = "Deselected visible emails".into();
        }
    }

    pub(super) fn select_unlabeled_visible(&mut self) {
        let mut added = 0usize;
        for email in &self.explorer_messages {
            if email.labels.is_empty() && self.checked_message_ids.insert(email.id) {
                added += 1;
            }
        }
        self.status = format!(
            "Selected {} unlabeled email{} in current view ({} total selected)",
            added,
            if added == 1 { "" } else { "s" },
            self.checked_message_ids.len()
        );
    }

    pub(super) fn select_range_to(&mut self, target_id: i64) {
        let anchor_id = self.selected_message_id.unwrap_or(target_id);
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
            self.status = format!(
                "Selected range of {} emails ({} total selected)",
                end - start + 1,
                self.checked_message_ids.len()
            );
        } else {
            self.checked_message_ids.insert(target_id);
            self.selected_message_id = Some(target_id);
        }
    }

    pub(super) fn selected_email(&self) -> Option<&EmailRecord> {
        let id = self.selected_message_id?;
        self.explorer_messages.iter().find(|email| email.id == id)
    }

    pub(super) fn label_color(&self, label_name: &str) -> &str {
        self.labels_with_counts
            .iter()
            .find(|(def, _)| def.name == label_name)
            .map(|(def, _)| def.color.as_str())
            .unwrap_or("#2563eb")
    }

    pub(super) fn label_selected_message(&mut self, backend: LabelerBackend) {
        if self.busy {
            return;
        }
        let Some(selected) = self.selected_email().cloned() else {
            self.status = "Select an email in the explorer first".into();
            return;
        };
        self.run_labeling_job(vec![selected], backend);
    }

    pub(super) fn label_checked_messages(&mut self, backend: LabelerBackend) {
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

    pub(super) fn label_visible_messages(&mut self) {
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
        let count = emails.len();
        let (tx, rx) = mpsc::channel();
        self.worker = Some(rx);
        self.busy = true;
        self.status = format!(
            "Labeling {} email{} with {}\u{2026}",
            count,
            if count == 1 { "" } else { "s" },
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
                let source_name = match backend {
                    LabelerBackend::Laya => {
                        let model = LayaModel::load_or_embedded(laya_path.as_deref())?;
                        for email in &emails {
                            let assigned = model.label_email(email, &definitions)?;
                            total_assigned += assigned.len();
                            db.set_message_labels(email.id, &assigned)?;
                        }
                        if model.is_embedded {
                            "laya-packaged".to_string()
                        } else {
                            "laya-gguf".to_string()
                        }
                    }
                    LabelerBackend::LlmJudge => {
                        for email in &emails {
                            let assigned = llm::judge_labels(&config, email, &definitions)?;
                            total_assigned += assigned.len();
                            db.set_message_labels(email.id, &assigned)?;
                        }
                        "llm-judge".to_string()
                    }
                };
                Ok(LabelOutcome {
                    messages_processed: emails.len(),
                    labels_assigned: total_assigned,
                    source_name,
                })
            })()
            .map_err(|error: anyhow::Error| format!("{error:#}"));
            let _ = tx.send(WorkerResult::Labeled(result));
        });
    }

    pub(super) fn toggle_manual_label(&mut self, message_id: i64, label: &str) {
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

    pub(super) fn toggle_manual_label_on_checked(&mut self, label: &str) {
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

    pub(super) fn save_label_definition(&mut self) {
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

    pub(super) fn remove_label_definition(&mut self, name: &str) {
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

    pub(super) fn set_source_enabled(&mut self, source: &str, enabled: bool) {
        self.status = match Index::open(&self.db_path)
            .and_then(|db| db.set_source_enabled(source, enabled))
        {
            Ok(()) => {
                self.refresh_sources();
                if enabled {
                    "PST enabled for retrieval".into()
                } else {
                    "PST excluded from retrieval".into()
                }
            }
            Err(error) => format!("Could not update PST selection: {error:#}"),
        };
    }

    pub(super) fn set_folders_enabled(&mut self, source: &str, folders: &[String], enabled: bool) {
        self.status = match Index::open(&self.db_path)
            .and_then(|mut db| db.set_folders_enabled(source, folders, enabled))
        {
            Ok(()) => {
                self.refresh_sources();
                if enabled {
                    "Folder tree enabled for retrieval".into()
                } else {
                    "Folder tree excluded from retrieval".into()
                }
            }
            Err(error) => format!("Could not update folder tree selection: {error:#}"),
        };
    }

    pub(super) fn remove_pending_source(&mut self) {
        let Some(source) = self.pending_remove_source.take() else {
            return;
        };
        self.status = match Index::open(&self.db_path).and_then(|mut db| db.remove_source(&source))
        {
            Ok(removed) => {
                self.count = self.count.saturating_sub(removed as u64);
                self.checked_message_ids.clear();
                self.refresh_sources();
                self.refresh_explorer();
                format!("Removed {removed} indexed messages and their embeddings")
            }
            Err(error) => format!("Could not remove indexed PST: {error:#}"),
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
    fs::write(&path, serde_json::to_vec_pretty(config)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
