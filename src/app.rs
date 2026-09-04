use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    thread,
};

use crate::{
    db::Index,
    llm::{self, LlmConfig},
    model::SearchHit,
    pst,
};
use eframe::egui;

enum WorkerResult {
    Indexed(Result<usize, String>),
    Searched(Result<Vec<SearchHit>, String>),
    Answered(Result<(String, Vec<SearchHit>, String), String>),
}

pub struct PstRagApp {
    db_path: PathBuf,
    archive: Option<PathBuf>,
    question: String,
    answer: String,
    hits: Vec<SearchHit>,
    preview: String,
    show_preview: bool,
    status: String,
    count: u64,
    busy: bool,
    config: LlmConfig,
    worker: Option<Receiver<WorkerResult>>,
}

impl PstRagApp {
    pub fn new(_: &eframe::CreationContext<'_>) -> Self {
        let base = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_owned()))
            .unwrap_or_else(|| PathBuf::from("."));
        let db_path = base.join("data").join("index.sqlite");
        let count = Index::open(&db_path).and_then(|d| d.count()).unwrap_or(0);
        Self {
            db_path,
            archive: None,
            question: String::new(),
            answer: String::new(),
            hits: vec![],
            preview: String::new(),
            show_preview: true,
            status: "Ready — all retrieval stays local".into(),
            count,
            busy: false,
            config: LlmConfig::default(),
            worker: None,
        }
    }

    fn choose_archive(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Outlook archive", &["pst"])
            .add_filter("Development fixture", &["jsonl"])
            .pick_file()
        {
            self.archive = Some(path);
        }
    }

    fn index(&mut self) {
        let Some(path) = self.archive.clone() else {
            self.status = "Select an archive first".into();
            return;
        };
        let db_path = self.db_path.clone();
        let (tx, rx) = mpsc::channel();
        self.worker = Some(rx);
        self.busy = true;
        self.status = "Indexing locally…".into();
        thread::spawn(move || {
            let result = (|| {
                let emails = pst::import(&path)?;
                let mut db = Index::open(&db_path)?;
                db.index_batch(&emails)
            })()
            .map_err(|e: anyhow::Error| format!("{e:#}"));
            let _ = tx.send(WorkerResult::Indexed(result));
        });
    }

    fn ask(&mut self) {
        if self.question.trim().is_empty() {
            self.status = "Enter a question first".into();
            return;
        }
        let question = self.question.clone();
        let db_path = self.db_path.clone();
        let config = self.config.clone();
        let (tx, rx) = mpsc::channel();
        self.worker = Some(rx);
        self.busy = true;
        self.status = "Searching locally…".into();
        thread::spawn(move || {
            let result = (|| {
                let db = Index::open(&db_path)?;
                let hits = db.search(&question, 10)?;
                let context = llm::build_context(&hits, config.max_context_chars);
                let answer = llm::ask(&config, &question, &context)?;
                Ok((answer, hits, context))
            })()
            .map_err(|e: anyhow::Error| format!("{e:#}"));
            let _ = tx.send(WorkerResult::Answered(result));
        });
    }

    fn search_locally(&mut self) {
        if self.question.trim().is_empty() {
            self.status = "Enter search terms first".into();
            return;
        }
        let query = self.question.clone();
        let db_path = self.db_path.clone();
        let (tx, rx) = mpsc::channel();
        self.worker = Some(rx);
        self.busy = true;
        self.status = "Searching locally — no network request…".into();
        thread::spawn(move || {
            let result = Index::open(&db_path)
                .and_then(|db| db.search(&query, 50))
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(WorkerResult::Searched(result));
        });
    }
}

impl eframe::App for PstRagApp {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if let Some(rx) = &self.worker
            && let Ok(result) = rx.try_recv()
        {
            self.busy = false;
            self.worker = None;
            match result {
                WorkerResult::Indexed(Ok(n)) => {
                    self.count = Index::open(&self.db_path)
                        .and_then(|d| d.count())
                        .unwrap_or(self.count);
                    self.status = format!("Indexed {n} new messages");
                }
                WorkerResult::Indexed(Err(e)) => self.status = e,
                WorkerResult::Searched(Ok(hits)) => {
                    let count = hits.len();
                    self.hits = hits;
                    self.answer.clear();
                    self.preview.clear();
                    self.status =
                        format!("Found {count} local results — nothing was sent over the network");
                }
                WorkerResult::Searched(Err(e)) => self.status = e,
                WorkerResult::Answered(Ok((a, h, p))) => {
                    self.answer = a;
                    self.hits = h;
                    self.preview = p;
                    self.status = "Answer received".into();
                }
                WorkerResult::Answered(Err(e)) => self.status = e,
            }
        }
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Dungbettle");
                ui.separator();
                ui.label("Local RAG on PST files");
            });
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!self.busy, egui::Button::new("Select archive…"))
                    .clicked()
                {
                    self.choose_archive();
                }
                ui.label(
                    self.archive
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "No archive selected".into()),
                );
            });
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !self.busy && self.archive.is_some(),
                        egui::Button::new("Index / update"),
                    )
                    .clicked()
                {
                    self.index();
                }
                ui.label(format!("{} messages indexed", self.count));
            });
            ui.separator();
            ui.label("Private LLM endpoint");
            ui.text_edit_singleline(&mut self.config.endpoint);
            ui.horizontal(|ui| {
                ui.label("Model");
                ui.text_edit_singleline(&mut self.config.model);
            });
            ui.horizontal(|ui| {
                ui.label("API key");
                ui.add(
                    egui::TextEdit::singleline(&mut self.config.bearer_token)
                        .password(true)
                        .hint_text("Optional bearer token")
                        .desired_width(f32::INFINITY),
                );
            });
            ui.add_space(8.0);
            ui.label("Ask");
            ui.add(
                egui::TextEdit::multiline(&mut self.question)
                    .desired_rows(3)
                    .hint_text("What was decided about Project Atlas?"),
            );
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !self.busy && self.count > 0,
                        egui::Button::new("Search locally"),
                    )
                    .clicked()
                {
                    self.search_locally();
                }
                if ui
                    .add_enabled(
                        !self.busy && self.count > 0,
                        egui::Button::new("Ask private LLM"),
                    )
                    .clicked()
                {
                    self.ask();
                }
            });
            ui.checkbox(&mut self.show_preview, "Show exact context sent to the LLM");
            ui.label(&self.status);
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Answer");
                ui.label(if self.answer.is_empty() {
                    "No answer yet."
                } else {
                    &self.answer
                });
                if self.show_preview && !self.preview.is_empty() {
                    ui.collapsing("Context preview", |ui| {
                        ui.monospace(&self.preview);
                    });
                }
                ui.heading("Sources");
                for (i, hit) in self.hits.iter().enumerate() {
                    ui.collapsing(
                        format!(
                            "{}. {} — {} — {}",
                            i + 1,
                            hit.sent_at.as_deref().unwrap_or("Unknown date"),
                            hit.sender,
                            hit.subject
                        ),
                        |ui| {
                            ui.label(format!(
                                "Folder: {} · BM25: {:.3} · Local ID: {}",
                                hit.folder, hit.score, hit.id
                            ));
                            ui.label(hit.excerpt(1200));
                        },
                    );
                }
            });
        });
        if self.busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}
