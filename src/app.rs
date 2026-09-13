use std::{
    fs,
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    thread,
    time::Duration,
};

use crate::{
    db::Index,
    llm::{self, LlmConfig},
    model::SearchHit,
    pst,
};
use iced::{
    Alignment, Border, Color, Element, Fill, Shadow, Subscription, Task, Theme, Vector,
    widget::{
        Space, button, checkbox, column, container, horizontal_rule, row, scrollable, text,
        text_input,
    },
};

const BACKGROUND: Color = Color::from_rgb(0.035, 0.055, 0.086);
const SIDEBAR: Color = Color::from_rgb(0.055, 0.078, 0.118);
const SURFACE: Color = Color::from_rgb(0.075, 0.105, 0.155);
const SURFACE_HIGH: Color = Color::from_rgb(0.105, 0.145, 0.205);
const BORDER_COLOR: Color = Color::from_rgb(0.17, 0.22, 0.30);
const ACCENT: Color = Color::from_rgb(0.31, 0.68, 0.98);
const MUTED: Color = Color::from_rgb(0.57, 0.64, 0.73);
const USER_BUBBLE: Color = Color::from_rgb(0.10, 0.34, 0.64);

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
    show_sources: bool,
    expanded_hit: Option<usize>,
    status: String,
    count: u64,
    busy: bool,
    config: LlmConfig,
    worker: Option<Receiver<WorkerResult>>,
}

#[derive(Debug, Clone)]
enum Message {
    ChooseArchive,
    ChooseStorage,
    Index,
    SaveCredentials,
    Ask,
    SearchLocally,
    NewChat,
    QuestionChanged(String),
    EndpointChanged(String),
    ModelChanged(String),
    TokenChanged(String),
    ToggleContext(usize, bool),
    ToggleSources,
    ToggleHit(usize),
    UsePrompt(&'static str),
    Tick,
}

pub fn run() -> iced::Result {
    iced::application("PST RAG", PstRagApp::update, PstRagApp::view)
        .subscription(PstRagApp::subscription)
        .theme(|_| {
            Theme::custom(
                "Dungbeetle".into(),
                iced::theme::Palette {
                    background: BACKGROUND,
                    text: Color::from_rgb(0.91, 0.94, 0.98),
                    primary: ACCENT,
                    success: Color::from_rgb(0.28, 0.78, 0.58),
                    danger: Color::from_rgb(0.95, 0.36, 0.40),
                },
            )
        })
        .window_size((1120.0, 760.0))
        .run_with(|| (PstRagApp::new(), Task::none()))
}

impl PstRagApp {
    fn new() -> Self {
        let base = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_owned()))
            .unwrap_or_else(|| PathBuf::from("."));
        let storage_location_path = base.join("storage-directory.txt");
        let storage_dir = fs::read_to_string(&storage_location_path)
            .ok()
            .map(|p| PathBuf::from(p.trim()))
            .filter(|p| !p.as_os_str().is_empty())
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
            show_sources: false,
            expanded_hit: None,
            status: "Ready — all retrieval stays local".into(),
            count,
            busy: false,
            config,
            worker: None,
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ChooseArchive => self.choose_archive(),
            Message::ChooseStorage => self.choose_storage_dir(),
            Message::Index => self.index(),
            Message::SaveCredentials => self.save_credentials(),
            Message::Ask => self.ask(),
            Message::SearchLocally => self.search_locally(),
            Message::NewChat => self.new_chat(),
            Message::QuestionChanged(v) => self.question = v,
            Message::EndpointChanged(v) => self.config.endpoint = v,
            Message::ModelChanged(v) => self.config.model = v,
            Message::TokenChanged(v) => self.config.bearer_token = v,
            Message::ToggleContext(i, v) => {
                if let Some(m) = self.messages.get_mut(i) {
                    m.show_context = v
                }
            }
            Message::ToggleSources => self.show_sources = !self.show_sources,
            Message::ToggleHit(i) => {
                self.expanded_hit = (self.expanded_hit != Some(i)).then_some(i)
            }
            Message::UsePrompt(prompt) => self.question = prompt.into(),
            Message::Tick => self.poll_worker(),
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        if self.busy {
            iced::time::every(Duration::from_millis(100)).map(|_| Message::Tick)
        } else {
            Subscription::none()
        }
    }

    fn poll_worker(&mut self) {
        let Some(result) = self.worker.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return;
        };
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
            WorkerResult::Searched(Ok(h)) => {
                let n = h.len();
                self.hits = h;
                self.show_sources = false;
                self.expanded_hit = None;
                self.messages.push(ChatMessage::assistant(format!("I found {n} matching email{} in your local archive. The sources are shown below.", if n == 1 { "" } else { "s" }), String::new()));
                self.status =
                    format!("Found {n} local results — nothing was sent over the network");
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
                self.show_sources = false;
                self.expanded_hit = None;
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

    fn choose_storage_dir(&mut self) {
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
            self.status = match fs::write(
                &self.storage_location_path,
                self.storage_dir.to_string_lossy().as_bytes(),
            ) {
                Ok(()) => format!(
                    "Storage directory changed to {}",
                    self.storage_dir.display()
                ),
                Err(e) => format!(
                    "Storage changed for this session, but the selection could not be saved: {e}"
                ),
            };
        }
    }

    fn save_credentials(&mut self) {
        if self.busy {
            return;
        }
        self.status = match save_config(&self.storage_dir, &self.config) {
            Ok(()) => format!("LLM settings saved in {}", self.storage_dir.display()),
            Err(e) => format!("Could not save LLM settings: {e:#}"),
        };
    }

    fn choose_archive(&mut self) {
        if self.busy {
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Outlook archive", &["pst"])
            .add_filter("Development fixture", &["jsonl"])
            .pick_file()
        {
            self.archive = Some(path)
        }
    }

    fn index(&mut self) {
        if self.busy {
            return;
        }
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
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(WorkerResult::Searched(result));
        });
    }

    fn new_chat(&mut self) {
        if !self.busy {
            self.messages.clear();
            self.hits.clear();
            self.show_sources = false;
            self.expanded_hit = None;
            self.question.clear();
            self.status = "New conversation — the local index is still available".into();
        }
    }

    fn view(&self) -> Element<'_, Message> {
        container(row![self.sidebar(), self.content()].height(Fill))
            .width(Fill)
            .height(Fill)
            .style(page_style)
            .into()
    }

    fn sidebar(&self) -> Element<'_, Message> {
        let name = self
            .archive
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("No archive selected");
        let choose = enabled_button("Choose PST", (!self.busy).then_some(Message::ChooseArchive));
        let index = enabled_button(
            "Index now",
            (!self.busy && self.archive.is_some()).then_some(Message::Index),
        );
        let storage = enabled_button(
            "Choose directory",
            (!self.busy).then_some(Message::ChooseStorage),
        );
        let save = enabled_button(
            "Save LLM settings",
            (!self.busy).then_some(Message::SaveCredentials),
        );
        let archive = card(
            column![
                row![
                    text(self.count).size(30).color(ACCENT),
                    column![
                        text("MESSAGES").size(10).color(MUTED),
                        text("indexed locally").size(12).color(MUTED)
                    ]
                    .spacing(1)
                ]
                .spacing(10)
                .align_y(Alignment::Center),
                horizontal_rule(1),
                text(name).size(15),
                text(
                    self.archive
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                )
                .size(11),
                row![choose, index].spacing(8)
            ]
            .spacing(8),
        );
        let storage_card = card(
            column![
                text(self.storage_dir.display().to_string()).size(11),
                text("Contains the local index and LLM credentials").size(11),
                storage
            ]
            .spacing(8),
        );
        let llm = card(
            column![
                text("Endpoint").size(12),
                text_input("https://…", &self.config.endpoint).on_input(Message::EndpointChanged),
                text("Model").size(12),
                text_input("Model name", &self.config.model).on_input(Message::ModelChanged),
                text("API key").size(12),
                text_input("Optional bearer token", &self.config.bearer_token)
                    .secure(true)
                    .on_input(Message::TokenChanged),
                text("HTTPS required except for 127.0.0.1").size(10),
                save
            ]
            .spacing(7),
        );
        let brand = row![
            container(text("D").size(17).color(BACKGROUND))
                .width(38)
                .height(38)
                .center_x(38)
                .center_y(38)
                .style(accent_tile_style),
            column![
                text("DUNGBEETLE").size(14),
                text("Private archive search").size(11).color(MUTED)
            ]
            .spacing(2)
        ]
        .spacing(11)
        .align_y(Alignment::Center);
        container(scrollable(
            column![
                brand,
                horizontal_rule(1),
                section_label("ARCHIVE"),
                archive,
                section_label("STORAGE"),
                storage_card,
                section_label("OPTIONAL LLM"),
                llm
            ]
            .spacing(12)
            .padding(18),
        ))
        .width(310)
        .height(Fill)
        .style(sidebar_style)
        .into()
    }

    fn content(&self) -> Element<'_, Message> {
        let header = row![
            column![
                text("Chat with your archive").size(27),
                text("Answers grounded in your indexed email.")
                    .size(13)
                    .color(MUTED)
            ]
            .spacing(4)
            .width(Fill),
            enabled_button("New chat", (!self.busy).then_some(Message::NewChat))
        ]
        .align_y(Alignment::Center);
        let status = container(
            row![
                text("●").size(9).color(if self.busy {
                    ACCENT
                } else {
                    Color::from_rgb(0.28, 0.78, 0.58)
                }),
                text(&self.status).size(11).color(MUTED)
            ]
            .spacing(7)
            .align_y(Alignment::Center),
        )
        .padding([6, 10])
        .style(status_style);

        let mut conversation = column![].spacing(14).width(Fill);
        if self.messages.is_empty() {
            let prompts = column![
                prompt_button(
                    "Summarize the latest decisions",
                    "Summarize the latest decisions in my archive"
                ),
                prompt_button(
                    "Find messages about a project",
                    "Find the latest messages about my current projects"
                ),
                prompt_button(
                    "What needs a follow-up?",
                    "Which recent messages need a follow-up?"
                )
            ]
            .spacing(8)
            .width(420);
            conversation = conversation.push(
                container(
                    column![
                        container(text("✦").size(22).color(ACCENT))
                            .width(52).height(52).center_x(52).center_y(52)
                            .style(icon_tile_style),
                        text("What would you like to find?").size(22),
                        text("Ask about decisions, people, dates, or projects. Only selected evidence is sent to your configured LLM.")
                            .size(13).color(MUTED).width(480),
                        prompts
                    ]
                    .spacing(12)
                    .align_x(Alignment::Center),
                )
                .padding([54, 10])
                .center_x(Fill),
            );
        }
        for (i, message) in self.messages.iter().enumerate() {
            conversation = conversation.push(self.message_view(i, message));
        }
        if !self.hits.is_empty() {
            let disclosure_label = if self.show_sources {
                "Hide evidence  ▴"
            } else {
                "Show evidence  ▾"
            };
            conversation = conversation.push(Space::new(0, 4)).push(
                button(
                    row![
                        text("EVIDENCE").size(11).color(ACCENT),
                        container(text(self.hits.len()).size(10))
                            .padding([2, 7])
                            .style(badge_style),
                        Space::new(Fill, 0),
                        text(disclosure_label).size(11).color(MUTED)
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .width(Fill)
                .padding([10, 12])
                .style(button::secondary)
                .on_press(Message::ToggleSources),
            );
            if self.show_sources {
                for (i, hit) in self.hits.iter().enumerate() {
                    conversation = conversation.push(self.hit_view(i, hit));
                }
            }
        }
        let enabled = !self.busy && self.count > 0 && !self.question.trim().is_empty();
        let mut question_input = text_input("Message your archive…", &self.question)
            .on_input(Message::QuestionChanged)
            .padding(12);
        if enabled {
            question_input = question_input.on_submit(Message::Ask);
        }
        let helper = if self.count == 0 {
            "Index an archive to start chatting"
        } else if self.busy {
            "Working with your local archive…"
        } else {
            "Enter to send  ·  Search only never contacts the LLM"
        };
        let composer = container(
            column![
                question_input,
                row![
                    text(helper).size(10).color(MUTED).width(Fill),
                    enabled_button("Search only", enabled.then_some(Message::SearchLocally)),
                    primary_button("Send  →", enabled.then_some(Message::Ask))
                ]
                .spacing(8)
                .align_y(Alignment::Center)
            ]
            .spacing(9),
        )
        .padding(12)
        .style(composer_style);
        container(
            column![
                header,
                status,
                scrollable(container(conversation).padding([8, 4])).height(Fill),
                composer
            ]
            .spacing(14)
            .padding([24, 28]),
        )
        .width(Fill)
        .height(Fill)
        .into()
    }

    fn message_view<'a>(&'a self, i: usize, message: &'a ChatMessage) -> Element<'a, Message> {
        let is_user = matches!(message.role, ChatRole::User);
        let avatar = container(text(if is_user { "Y" } else { "D" }).size(11))
            .width(30)
            .height(30)
            .center_x(30)
            .center_y(30)
            .style(if is_user {
                user_avatar_style
            } else {
                assistant_avatar_style
            });
        let mut contents = column![
            text(if is_user { "YOU" } else { "DUNGBEETLE" })
                .size(10)
                .color(if is_user { Color::WHITE } else { ACCENT }),
            text(&message.content).size(14).line_height(1.45)
        ]
        .spacing(7);
        if !message.context_preview.is_empty() {
            contents = contents.push(horizontal_rule(1)).push(
                checkbox("Show context sent to the LLM", message.show_context)
                    .on_toggle(move |v| Message::ToggleContext(i, v)),
            );
            if message.show_context {
                contents = contents.push(text(&message.context_preview).size(11));
            }
        }
        let bubble = container(contents)
            .max_width(680)
            .padding([13, 16])
            .style(if is_user {
                user_bubble_style
            } else {
                assistant_bubble_style
            });
        match message.role {
            ChatRole::User => row![Space::new(Fill, 0), bubble, avatar]
                .spacing(10)
                .align_y(Alignment::End)
                .into(),
            ChatRole::Assistant => row![avatar, bubble, Space::new(Fill, 0)]
                .spacing(10)
                .align_y(Alignment::Start)
                .into(),
        }
    }

    fn hit_view<'a>(&'a self, i: usize, hit: &'a SearchHit) -> Element<'a, Message> {
        let subject = if hit.subject.is_empty() {
            "(No subject)"
        } else {
            &hit.subject
        };
        let mut contents = column![
            row![
                text(format!("{:02}", i + 1)).size(13),
                column![
                    text(subject).size(15),
                    text(format!(
                        "{}  •  {}",
                        hit.sender,
                        hit.sent_at.as_deref().unwrap_or("Unknown date")
                    ))
                    .size(11)
                ]
                .spacing(3)
            ]
            .spacing(10),
            text(hit.excerpt(420)).size(13),
            button(if self.expanded_hit == Some(i) {
                "Hide details"
            } else {
                "View full excerpt and metadata"
            })
            .on_press(Message::ToggleHit(i))
        ]
        .spacing(8);
        if self.expanded_hit == Some(i) {
            contents = contents
                .push(
                    text(format!(
                        "Folder: {}  •  BM25: {:.3}  •  Local ID: {}",
                        hit.folder, hit.score, hit.id
                    ))
                    .size(11),
                )
                .push(horizontal_rule(1))
                .push(text(hit.excerpt(1200)).size(12));
        }
        container(contents)
            .width(Fill)
            .padding(14)
            .style(source_style)
            .into()
    }
}

fn enabled_button<'a>(
    label: &'a str,
    message: Option<Message>,
) -> iced::widget::Button<'a, Message> {
    let button = button(label);
    if let Some(message) = message {
        button.on_press(message)
    } else {
        button
    }
}
fn primary_button<'a>(
    label: &'a str,
    message: Option<Message>,
) -> iced::widget::Button<'a, Message> {
    let button = button(label).padding([9, 16]).style(button::primary);
    if let Some(message) = message {
        button.on_press(message)
    } else {
        button
    }
}
fn prompt_button<'a>(label: &'a str, prompt: &'static str) -> iced::widget::Button<'a, Message> {
    button(row![
        text(label).size(13).width(Fill),
        text("→").size(13).color(ACCENT)
    ])
    .width(Fill)
    .padding([10, 14])
    .style(button::secondary)
    .on_press(Message::UsePrompt(prompt))
}
fn section_label(label: &str) -> Element<'_, Message> {
    text(label).size(10).color(MUTED).into()
}
fn card<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .width(Fill)
        .padding(16)
        .style(card_style)
        .into()
}

fn panel(background: Color, border: Color, radius: f32) -> iced::widget::container::Style {
    iced::widget::container::Style {
        background: Some(background.into()),
        border: Border::default().rounded(radius).color(border).width(1),
        ..Default::default()
    }
}

fn page_style(_: &Theme) -> iced::widget::container::Style {
    BACKGROUND.into()
}
fn sidebar_style(_: &Theme) -> iced::widget::container::Style {
    iced::widget::container::Style {
        background: Some(SIDEBAR.into()),
        border: Border {
            color: BORDER_COLOR,
            width: 1.0,
            ..Default::default()
        },
        ..Default::default()
    }
}
fn card_style(_: &Theme) -> iced::widget::container::Style {
    panel(SURFACE, BORDER_COLOR, 12.0)
}
fn status_style(_: &Theme) -> iced::widget::container::Style {
    panel(SURFACE, BORDER_COLOR, 20.0)
}
fn badge_style(_: &Theme) -> iced::widget::container::Style {
    panel(SURFACE_HIGH, Color::TRANSPARENT, 10.0)
}
fn source_style(_: &Theme) -> iced::widget::container::Style {
    panel(SURFACE, BORDER_COLOR, 10.0)
}
fn icon_tile_style(_: &Theme) -> iced::widget::container::Style {
    panel(SURFACE_HIGH, BORDER_COLOR, 16.0)
}
fn accent_tile_style(_: &Theme) -> iced::widget::container::Style {
    panel(ACCENT, ACCENT, 11.0)
}
fn user_avatar_style(_: &Theme) -> iced::widget::container::Style {
    panel(USER_BUBBLE, USER_BUBBLE, 15.0)
}
fn assistant_avatar_style(_: &Theme) -> iced::widget::container::Style {
    panel(SURFACE_HIGH, BORDER_COLOR, 15.0).color(ACCENT)
}
fn user_bubble_style(_: &Theme) -> iced::widget::container::Style {
    panel(USER_BUBBLE, USER_BUBBLE, 14.0)
}
fn assistant_bubble_style(_: &Theme) -> iced::widget::container::Style {
    panel(SURFACE, BORDER_COLOR, 14.0)
}
fn composer_style(_: &Theme) -> iced::widget::container::Style {
    let mut style = panel(SURFACE, Color::from_rgb(0.24, 0.34, 0.46), 14.0);
    style.shadow = Shadow {
        color: Color::from_rgba(0.0, 0.0, 0.0, 0.28),
        offset: Vector::new(0.0, 5.0),
        blur_radius: 18.0,
    };
    style
}

fn load_config(storage_dir: &std::path::Path) -> anyhow::Result<LlmConfig> {
    let path = storage_dir.join("credentials.json");
    if !path.exists() {
        return Ok(LlmConfig::default());
    }
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
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
