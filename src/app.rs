use std::{
    fs,
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

#[derive(Clone, Copy)]
enum ChatRole {
    User,
    Assistant,
}

struct ChatMessage {
    role: ChatRole,
    content: String,
    context_preview: String,
    show_context: bool,
}

impl ChatMessage {
    fn user(content: String) -> Self {
        Self {
            role: ChatRole::User,
            content,
            context_preview: String::new(),
            show_context: false,
        }
    }

    fn assistant(content: String, context_preview: String) -> Self {
        Self {
            role: ChatRole::Assistant,
            content,
            context_preview,
            show_context: false,
        }
    }
}

pub struct PstRagApp {
    storage_location_path: PathBuf,
    storage_dir: PathBuf,
    db_path: PathBuf,
    archive: Option<PathBuf>,
    question: String,
    messages: Vec<ChatMessage>,
    hits: Vec<SearchHit>,
    status: String,
    count: u64,
    busy: bool,
    config: LlmConfig,
    worker: Option<Receiver<WorkerResult>>,
}

impl PstRagApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_style(&cc.egui_ctx);
        let base = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_owned()))
            .unwrap_or_else(|| PathBuf::from("."));
        let storage_location_path = base.join("storage-directory.txt");
        let storage_dir = fs::read_to_string(&storage_location_path)
            .ok()
            .map(|path| PathBuf::from(path.trim()))
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| base.join("data"));
        let db_path = storage_dir.join("index.sqlite");
        let config = load_config(&storage_dir).unwrap_or_default();
        let count = Index::open(&db_path).and_then(|d| d.count()).unwrap_or(0);
        Self {
            storage_location_path,
            storage_dir,
            db_path,
            archive: None,
            question: String::new(),
            messages: vec![],
            hits: vec![],
            status: "Ready — all retrieval stays local".into(),
            count,
            busy: false,
            config,
            worker: None,
        }
    }

    fn choose_storage_dir(&mut self) {
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

    fn save_credentials(&mut self) {
        match save_config(&self.storage_dir, &self.config) {
            Ok(()) => self.status = format!("LLM settings saved in {}", self.storage_dir.display()),
            Err(error) => self.status = format!("Could not save LLM settings: {error:#}"),
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
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(WorkerResult::Searched(result));
        });
    }

    fn new_chat(&mut self) {
        if self.busy {
            return;
        }
        self.messages.clear();
        self.hits.clear();
        self.question.clear();
        self.status = "New conversation — the local index is still available".into();
    }
}

fn load_config(storage_dir: &std::path::Path) -> anyhow::Result<LlmConfig> {
    let path = storage_dir.join("credentials.json");
    if !path.exists() {
        return Ok(LlmConfig::default());
    }
    let contents = fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&contents)?)
}

fn save_config(storage_dir: &std::path::Path, config: &LlmConfig) -> anyhow::Result<()> {
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

fn configure_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = egui::Color32::from_rgb(13, 18, 27);
    visuals.window_fill = egui::Color32::from_rgb(18, 24, 35);
    visuals.extreme_bg_color = egui::Color32::from_rgb(9, 13, 20);
    visuals.faint_bg_color = egui::Color32::from_rgb(24, 32, 45);
    visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(29, 39, 54);
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(38, 52, 72);
    visuals.widgets.active.bg_fill = egui::Color32::from_rgb(47, 129, 247);
    visuals.selection.bg_fill = egui::Color32::from_rgb(44, 113, 211);
    visuals.hyperlink_color = egui::Color32::from_rgb(100, 181, 246);
    visuals.window_corner_radius = egui::CornerRadius::same(12);
    ctx.set_visuals(visuals);

    ctx.style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(10.0, 10.0);
        style.spacing.button_padding = egui::vec2(14.0, 8.0);
        style.spacing.interact_size.y = 34.0;
        style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(7);
        style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(7);
        style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(7);
    });
}

fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(egui::Color32::from_rgb(20, 27, 39))
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(42, 53, 70)))
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(egui::Margin::same(16))
}

// Placeholder for the image asset that will be supplied later.
fn logo_placeholder(ui: &mut egui::Ui) {
    egui::Frame::new()
        .fill(egui::Color32::from_rgb(20, 27, 39))
        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(70, 91, 119)))
        .corner_radius(egui::CornerRadius::same(10))
        .show(ui, |ui| {
            ui.set_min_size(egui::vec2(52.0, 52.0));
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("LOGO")
                        .size(9.0)
                        .strong()
                        .color(egui::Color32::from_rgb(151, 164, 183)),
                );
            });
        });
}

impl eframe::App for PstRagApp {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        let window_width = ctx.screen_rect().width();
        let compact = window_width < 850.0;
        let sidebar_width = (window_width * 0.29).clamp(240.0, 310.0);
        let content_margin = if compact { 12 } else { 22 };
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
                    self.messages.push(ChatMessage::assistant(
                        format!(
                            "I found {count} matching email{} in your local archive. The sources are shown below.",
                            if count == 1 { "" } else { "s" }
                        ),
                        String::new(),
                    ));
                    self.status =
                        format!("Found {count} local results — nothing was sent over the network");
                }
                WorkerResult::Searched(Err(e)) => {
                    self.messages.push(ChatMessage::assistant(
                        format!("I couldn't complete that local search: {e}"),
                        String::new(),
                    ));
                    self.status = e;
                }
                WorkerResult::Answered(Ok((a, h, p))) => {
                    self.messages.push(ChatMessage::assistant(a, p));
                    self.hits = h;
                    self.status = "Answer received".into();
                }
                WorkerResult::Answered(Err(e)) => {
                    self.messages.push(ChatMessage::assistant(
                        format!("I couldn't answer that question: {e}"),
                        String::new(),
                    ));
                    self.status = e;
                }
            }
        }
        egui::SidePanel::left("settings")
            .resizable(true)
            .default_width(sidebar_width)
            .width_range(220.0..=400.0)
            .frame(
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(13, 18, 27))
                    .inner_margin(16),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("settings_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("ARCHIVE")
                                .size(11.0)
                                .strong()
                                .color(egui::Color32::from_rgb(125, 143, 168)),
                        );
                        ui.add_space(4.0);
                        card().show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(format!("{}", self.count))
                                    .size(28.0)
                                    .strong()
                                    .color(egui::Color32::from_rgb(104, 181, 255)),
                            );
                            ui.label(
                                egui::RichText::new("messages indexed")
                                    .color(egui::Color32::from_rgb(151, 164, 183)),
                            );
                            ui.separator();
                            let archive_name = self
                                .archive
                                .as_ref()
                                .and_then(|p| p.file_name())
                                .and_then(|n| n.to_str())
                                .unwrap_or("No archive selected");
                            ui.label(egui::RichText::new(archive_name).strong());
                            if let Some(path) = &self.archive {
                                ui.label(
                                    egui::RichText::new(path.display().to_string())
                                        .size(11.0)
                                        .color(egui::Color32::from_rgb(126, 140, 160)),
                                );
                            }
                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(!self.busy, egui::Button::new("Choose PST"))
                                    .clicked()
                                {
                                    self.choose_archive();
                                }
                                if ui
                                    .add_enabled(
                                        !self.busy && self.archive.is_some(),
                                        egui::Button::new("Index now")
                                            .fill(egui::Color32::from_rgb(40, 111, 209)),
                                    )
                                    .clicked()
                                {
                                    self.index();
                                }
                            });
                        });

                        ui.add_space(14.0);
                        ui.label(
                            egui::RichText::new("STORAGE")
                                .size(11.0)
                                .strong()
                                .color(egui::Color32::from_rgb(125, 143, 168)),
                        );
                        ui.add_space(4.0);
                        card().show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(self.storage_dir.display().to_string())
                                    .size(11.0)
                                    .color(egui::Color32::from_rgb(151, 164, 183)),
                            );
                            ui.label(
                                egui::RichText::new("Contains the local index and LLM credentials")
                                    .size(10.0)
                                    .color(egui::Color32::from_rgb(120, 136, 157)),
                            );
                            if ui
                                .add_enabled(!self.busy, egui::Button::new("Choose directory"))
                                .clicked()
                            {
                                self.choose_storage_dir();
                            }
                        });

                        ui.add_space(14.0);
                        ui.label(
                            egui::RichText::new("OPTIONAL LLM")
                                .size(11.0)
                                .strong()
                                .color(egui::Color32::from_rgb(125, 143, 168)),
                        );
                        ui.add_space(4.0);
                        card().show(ui, |ui| {
                            ui.label(egui::RichText::new("Endpoint").size(12.0));
                            ui.add(
                                egui::TextEdit::singleline(&mut self.config.endpoint)
                                    .desired_width(f32::INFINITY),
                            );
                            ui.label(egui::RichText::new("Model").size(12.0));
                            ui.add(
                                egui::TextEdit::singleline(&mut self.config.model)
                                    .desired_width(f32::INFINITY),
                            );
                            ui.label(egui::RichText::new("API key").size(12.0));
                            ui.add(
                                egui::TextEdit::singleline(&mut self.config.bearer_token)
                                    .password(true)
                                    .hint_text("Optional bearer token")
                                    .desired_width(f32::INFINITY),
                            );
                            ui.label(
                                egui::RichText::new("HTTPS required except for 127.0.0.1")
                                    .size(10.0)
                                    .color(egui::Color32::from_rgb(120, 136, 157)),
                            );
                            if ui
                                .add_enabled(!self.busy, egui::Button::new("Save LLM settings"))
                                .clicked()
                            {
                                self.save_credentials();
                            }
                        });
                    });
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(10, 15, 23))
                    .inner_margin(content_margin),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // logo_placeholder(ui);
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Chat with your archive").size(25.0).strong());
                        ui.label(
                            egui::RichText::new("Ask follow-up questions and keep the conversation in view.")
                                .color(egui::Color32::from_rgb(143, 158, 179)),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(!self.busy, egui::Button::new("New chat"))
                            .clicked()
                        {
                            self.new_chat();
                        }
                    });
                });

                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(&self.status)
                            .size(12.0)
                            .color(egui::Color32::from_rgb(151, 164, 183)),
                    );
                });

                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .id_salt("conversation")
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .max_height((ui.available_height() - 150.0).max(160.0))
                    .show(ui, |ui| {
                        if self.messages.is_empty() {
                            ui.vertical_centered(|ui| {
                                ui.add_space(45.0);
                                ui.label(egui::RichText::new("Start a conversation").size(20.0).strong());
                                ui.label(
                                    egui::RichText::new("Ask about decisions, people, dates, or projects in your email archive.")
                                        .color(egui::Color32::from_rgb(135, 150, 171)),
                                );
                            });
                        }
                        for message in &mut self.messages {
                            let (label, fill, align) = match message.role {
                                ChatRole::User => (
                                    "You",
                                    egui::Color32::from_rgb(35, 91, 164),
                                    egui::Align::Max,
                                ),
                                ChatRole::Assistant => (
                                    "Dungbeetle",
                                    egui::Color32::from_rgb(20, 27, 39),
                                    egui::Align::Min,
                                ),
                            };
                            ui.with_layout(egui::Layout::top_down(align), |ui| {
                                egui::Frame::new()
                                    .fill(fill)
                                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(50, 67, 89)))
                                    .corner_radius(egui::CornerRadius::same(12))
                                    .inner_margin(egui::Margin::same(14))
                                    .show(ui, |ui| {
                                        ui.set_max_width(640.0);
                                        ui.label(
                                            egui::RichText::new(label)
                                                .size(11.0)
                                                .strong()
                                                .color(egui::Color32::from_rgb(170, 192, 220)),
                                        );
                                        ui.label(&message.content);
                                        if !message.context_preview.is_empty() {
                                            ui.separator();
                                            ui.checkbox(&mut message.show_context, "Show context sent to the LLM");
                                            if message.show_context {
                                                ui.collapsing("Context preview", |ui| {
                                                    ui.monospace(&message.context_preview);
                                                });
                                            }
                                        }
                                    });
                            });
                            ui.add_space(10.0);
                        }
                        if !self.hits.is_empty() {
                            ui.collapsing(format!("Sources · {} emails", self.hits.len()), |ui| {
                            for (i, hit) in self.hits.iter().enumerate() {
                                card().show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(format!("{:02}", i + 1))
                                            .monospace()
                                            .color(egui::Color32::from_rgb(104, 181, 255)),
                                    );
                                    ui.vertical(|ui| {
                                        ui.label(
                                            egui::RichText::new(if hit.subject.is_empty() {
                                                "(No subject)"
                                            } else {
                                                &hit.subject
                                            })
                                            .strong()
                                            .size(15.0),
                                        );
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "{}  •  {}",
                                                hit.sender,
                                                hit.sent_at.as_deref().unwrap_or("Unknown date")
                                            ))
                                            .size(11.0)
                                            .color(egui::Color32::from_rgb(131, 147, 169)),
                                        );
                                    });
                                });
                                ui.add_space(4.0);
                                ui.label(hit.excerpt(420));
                                ui.collapsing("View full excerpt and metadata", |ui| {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "Folder: {}  •  BM25: {:.3}  •  Local ID: {}",
                                            hit.folder, hit.score, hit.id
                                        ))
                                        .size(11.0)
                                        .color(egui::Color32::from_rgb(126, 140, 160)),
                                    );
                                    ui.separator();
                                    ui.label(hit.excerpt(1200));
                                });
                                });
                                ui.add_space(8.0);
                            }
                            });
                        }
                    });

                ui.add_space(10.0);
                card().show(ui, |ui| {
                    ui.add_enabled(
                        !self.busy,
                        egui::TextEdit::multiline(&mut self.question)
                            .desired_rows(2)
                            .hint_text("Message your archive…")
                            .desired_width(f32::INFINITY)
                            .frame(false),
                    );
                    ui.separator();
                    ui.horizontal(|ui| {
                        let enabled = !self.busy && self.count > 0 && !self.question.trim().is_empty();
                        if ui
                            .add_enabled(enabled, egui::Button::new("Send").fill(egui::Color32::from_rgb(40, 111, 209)))
                            .clicked()
                        {
                            self.ask();
                        }
                        if ui
                            .add_enabled(enabled, egui::Button::new("Search only"))
                            .clicked()
                        {
                            self.search_locally();
                        }
                        if self.busy {
                            ui.spinner();
                            ui.label("Thinking…");
                        }
                    });
                });
            });
        if self.busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}
