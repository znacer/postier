mod actions;
mod chat;
mod sidebar;
mod theme;

use crate::{db::Index, llm::LlmConfig, model::SearchHit};
use actions::load_config;
use iced::{
    Element, Fill, Subscription, Task,
    widget::{container, row},
};
use std::{fs, path::PathBuf, sync::mpsc::Receiver, time::Duration};
use theme::{app_theme, page_style};

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
    iced::application("PSTbeetle", PstRagApp::update, PstRagApp::view)
        .subscription(PstRagApp::subscription)
        .theme(|_| app_theme())
        .window_size((1120.0, 760.0))
        .run_with(|| (PstRagApp::new(), Task::none()))
}

impl PstRagApp {
    fn new() -> Self {
        let base = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|path| path.to_owned()))
            .unwrap_or_else(|| PathBuf::from("."));
        let storage_location_path = base.join("storage-directory.txt");
        let storage_dir = fs::read_to_string(&storage_location_path)
            .ok()
            .map(|path| PathBuf::from(path.trim()))
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| base.join("data"));
        let db_path = storage_dir.join("index.sqlite");
        let config = load_config(&storage_dir).unwrap_or_default();
        let count = Index::open(&db_path).and_then(|db| db.count()).unwrap_or(0);
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
            Message::QuestionChanged(value) => self.question = value,
            Message::EndpointChanged(value) => self.config.endpoint = value,
            Message::ModelChanged(value) => self.config.model = value,
            Message::TokenChanged(value) => self.config.bearer_token = value,
            Message::ToggleContext(index, value) => {
                if let Some(message) = self.messages.get_mut(index) {
                    message.show_context = value
                }
            }
            Message::ToggleSources => self.show_sources = !self.show_sources,
            Message::ToggleHit(index) => {
                self.expanded_hit = (self.expanded_hit != Some(index)).then_some(index)
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
        let Some(result) = self
            .worker
            .as_ref()
            .and_then(|receiver| receiver.try_recv().ok())
        else {
            return;
        };
        self.busy = false;
        self.worker = None;
        match result {
            WorkerResult::Indexed(Ok(count)) => {
                self.count = Index::open(&self.db_path)
                    .and_then(|db| db.count())
                    .unwrap_or(self.count);
                self.status = format!("Indexed {count} new messages");
            }
            WorkerResult::Indexed(Err(error)) => self.status = error,
            WorkerResult::Searched(Ok(hits)) => {
                let count = hits.len();
                self.hits = hits;
                self.show_sources = false;
                self.expanded_hit = None;
                self.messages.push(ChatMessage::assistant(format!("I found {count} matching email{} in your local archive. The sources are shown below.", if count == 1 { "" } else { "s" }), String::new()));
                self.status =
                    format!("Found {count} local results — nothing was sent over the network");
            }
            WorkerResult::Searched(Err(error)) => {
                self.messages.push(ChatMessage::assistant(
                    format!("I couldn't complete that local search: {error}"),
                    String::new(),
                ));
                self.status = error;
            }
            WorkerResult::Answered(Ok((answer, hits, preview))) => {
                self.messages.push(ChatMessage::assistant(answer, preview));
                self.hits = hits;
                self.show_sources = false;
                self.expanded_hit = None;
                self.status = "Answer received".into();
            }
            WorkerResult::Answered(Err(error)) => {
                self.messages.push(ChatMessage::assistant(
                    format!("I couldn't answer that question: {error}"),
                    String::new(),
                ));
                self.status = error;
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        container(row![self.sidebar(), self.content()].height(Fill))
            .width(Fill)
            .height(Fill)
            .style(page_style)
            .into()
    }
}
