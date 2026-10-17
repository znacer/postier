use std::{fs, sync::mpsc, thread};

use anyhow::Context;

use crate::{db::Index, llm, pst};

use super::{ChatMessage, IndexOutcome, PstRagApp, WorkerResult};

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
            self.refresh_sources();
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
            Ok(()) => format!("LLM settings saved in {}", self.storage_dir.display()),
            Err(error) => format!("Could not save LLM settings: {error:#}"),
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
            "Indexing BM25 locally and generating remote embeddings…".into()
        } else {
            "Indexing BM25 locally…".into()
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
        self.messages.push(ChatMessage::user(question.clone()));
        self.question.clear();
        let db_path = self.db_path.clone();
        let config = self.config.clone();
        let (tx, rx) = mpsc::channel();
        self.worker = Some(rx);
        self.busy = true;
        self.status = "Searching locally…".into();
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

    pub(super) fn search_locally(&mut self) {
        if self.busy {
            return;
        }
        if self.question.trim().is_empty() {
            self.status = "Enter search terms first".into();
            return;
        }
        let query = self.question.trim().to_owned();
        self.messages.push(ChatMessage::user(query.clone()));
        self.question.clear();
        let db_path = self.db_path.clone();
        let (tx, rx) = mpsc::channel();
        self.worker = Some(rx);
        self.busy = true;
        self.status = "Searching locally — no network request…".into();
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
        self.status = "New conversation — the local index is still available".into();
    }

    pub(super) fn refresh_sources(&mut self) {
        self.sources = Index::open(&self.db_path)
            .and_then(|db| db.sources())
            .unwrap_or_default();
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
                self.refresh_sources();
                format!("Removed {removed} indexed messages and their embeddings")
            }
            Err(error) => format!("Could not remove indexed PST: {error:#}"),
        };
    }
}

pub(super) fn load_config(storage_dir: &std::path::Path) -> anyhow::Result<crate::llm::LlmConfig> {
    let path = storage_dir.join("credentials.json");
    if !path.exists() {
        return Ok(crate::llm::LlmConfig::default());
    }
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

fn save_config(
    storage_dir: &std::path::Path,
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
