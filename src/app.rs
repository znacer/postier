mod actions;
mod chat;
mod explorer;
mod sidebar;
mod theme;

use crate::{
    db::Index,
    llm::LlmConfig,
    model::{
        EmailRecord, IndexedSource, LabelDefinition, LabelFilter, LabelerBackend, MailScope,
        MailSort, SearchHit,
    },
};
use actions::load_config;
use iced::{
    Element, Fill, Subscription, Task,
    widget::{container, row},
};
use std::{collections::HashSet, fs, path::PathBuf, sync::mpsc::Receiver, time::Duration};
use theme::{app_theme, page_style};

enum WorkerResult {
    Indexed(Result<IndexOutcome, String>),
    Searched(Result<Vec<SearchHit>, String>),
    Answered(Result<(String, Vec<SearchHit>, String), String>),
    Labeled(Result<LabelOutcome, String>),
    LayaDownloaded(Result<PathBuf, String>),
}

struct IndexOutcome {
    added: usize,
    embedded: usize,
}

struct LabelOutcome {
    messages_processed: usize,
    labels_assigned: usize,
    source_name: String,
}

#[derive(Debug, Clone, Copy)]
enum SidebarSection {
    Mailboxes,
    Labels,
    LayaModel,
    AiModels,
    Storage,
}

impl SidebarSection {
    fn index(self) -> usize {
        match self {
            Self::Mailboxes => 0,
            Self::Labels => 1,
            Self::LayaModel => 2,
            Self::AiModels => 3,
            Self::Storage => 4,
        }
    }
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
    sources: Vec<IndexedSource>,
    pending_remove_source: Option<String>,
    collapsed_source_folders: HashSet<(String, String)>,
    collapsed_sections: [bool; 5],
    explorer_messages: Vec<EmailRecord>,
    selected_message_id: Option<i64>,
    checked_message_ids: HashSet<i64>,
    mail_scope: MailScope,
    label_filter: LabelFilter,
    filter_query: String,
    mail_sort: MailSort,
    labels_with_counts: Vec<(LabelDefinition, u64)>,
    unlabeled_count: u64,
    new_label_name: String,
    new_label_description: String,
    chat_open: bool,
    chat_width: f32,
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
    EmbeddingModelChanged(String),
    RemoteEmbeddingsChanged(bool),
    SourceEnabledChanged(String, bool),
    FolderTreeEnabledChanged(String, Vec<String>, bool),
    ToggleFolderTreeNode(String, String),
    RemoveSourceRequested(String),
    ConfirmRemoveSource,
    CancelRemoveSource,
    ToggleSidebarSection(SidebarSection),
    TokenChanged(String),
    ToggleContext(usize, bool),
    ToggleSources,
    ToggleHit(usize),
    UsePrompt(&'static str),
    SelectEmail(i64),
    ToggleCheckEmail(i64, bool),
    ToggleCheckAllVisible(bool),
    SelectUnlabeledVisible,
    SelectRangeTo(i64),
    ClearCheckedEmails,
    SelectMailScope(MailScope),
    SelectLabelFilter(LabelFilter),
    ExplorerFilterChanged(String),
    SetMailSort(MailSort),
    SetLabelerBackend(LabelerBackend),
    LabelSelectedMessage,
    LabelSelectedWith(LabelerBackend),
    LabelCheckedWith(LabelerBackend),
    LabelVisibleMessages,
    ToggleManualLabel(i64, String),
    ToggleManualLabelOnChecked(String),
    NewLabelNameChanged(String),
    NewLabelDescriptionChanged(String),
    SaveLabelDefinition,
    RemoveLabelDefinition(String),
    LayaGgufPathChanged(String),
    ChooseLayaGguf,
    DownloadLayaGguf,
    ToggleChatPane,
    ChatWidthChanged(f32),
    AskAboutEmail(i64),
    Tick,
}

pub fn run() -> iced::Result {
    iced::application(
        "PSTbeetle \u{2014} Mail Explorer & Local RAG",
        PstRagApp::update,
        PstRagApp::view,
    )
    .subscription(PstRagApp::subscription)
    .theme(|_| app_theme())
    .window_size((1440.0, 860.0))
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
        let (count, sources) = Index::open(&db_path)
            .map(|db| (db.count().unwrap_or(0), db.sources().unwrap_or_default()))
            .unwrap_or_default();
        let mut app = Self {
            storage_location_path,
            storage_dir,
            db_path,
            archive: None,
            question: String::new(),
            messages: vec![],
            hits: vec![],
            show_sources: false,
            expanded_hit: None,
            status: "Ready \u{2014} Thunderbird mail explorer + Laya / LLM Judge labeling".into(),
            count,
            sources,
            pending_remove_source: None,
            collapsed_source_folders: HashSet::new(),
            collapsed_sections: [false, false, false, true, true],
            explorer_messages: vec![],
            selected_message_id: None,
            checked_message_ids: HashSet::new(),
            mail_scope: MailScope::AllMessages,
            label_filter: LabelFilter::Any,
            filter_query: String::new(),
            mail_sort: MailSort::DateDesc,
            labels_with_counts: vec![],
            unlabeled_count: 0,
            new_label_name: String::new(),
            new_label_description: String::new(),
            chat_open: true,
            chat_width: 410.0,
            busy: false,
            config,
            worker: None,
        };
        app.refresh_explorer();
        app
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
            Message::EmbeddingModelChanged(value) => self.config.embedding_model = value,
            Message::RemoteEmbeddingsChanged(value) => self.config.use_remote_embeddings = value,
            Message::SourceEnabledChanged(source, enabled) => {
                self.set_source_enabled(&source, enabled)
            }
            Message::FolderTreeEnabledChanged(source, folders, enabled) => {
                self.set_folders_enabled(&source, &folders, enabled)
            }
            Message::ToggleFolderTreeNode(source, folder) => {
                let key = (source, folder);
                if !self.collapsed_source_folders.remove(&key) {
                    self.collapsed_source_folders.insert(key);
                }
            }
            Message::RemoveSourceRequested(source) => self.pending_remove_source = Some(source),
            Message::ConfirmRemoveSource => self.remove_pending_source(),
            Message::CancelRemoveSource => self.pending_remove_source = None,
            Message::ToggleSidebarSection(section) => {
                let index = section.index();
                self.collapsed_sections[index] = !self.collapsed_sections[index];
            }
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
            Message::UsePrompt(prompt) => {
                self.question = prompt.into();
                self.chat_open = true;
            }
            Message::SelectEmail(id) => self.select_email(id),
            Message::ToggleCheckEmail(id, checked) => self.toggle_check_email(id, checked),
            Message::ToggleCheckAllVisible(checked) => self.toggle_check_all_visible(checked),
            Message::SelectUnlabeledVisible => self.select_unlabeled_visible(),
            Message::SelectRangeTo(id) => self.select_range_to(id),
            Message::ClearCheckedEmails => {
                self.checked_message_ids.clear();
                self.status = "Cleared mail selection".into();
            }
            Message::SelectMailScope(scope) => {
                self.mail_scope = scope;
                self.refresh_explorer();
            }
            Message::SelectLabelFilter(filter) => {
                self.label_filter = filter;
                self.refresh_explorer();
            }
            Message::ExplorerFilterChanged(query) => {
                self.filter_query = query;
                self.refresh_explorer();
            }
            Message::SetMailSort(sort) => {
                self.mail_sort = sort;
                self.refresh_explorer();
            }
            Message::SetLabelerBackend(backend) => {
                self.config.labeler_backend = backend;
                self.status = format!("Active mail labeler switched to {}", backend.display_name());
            }
            Message::LabelSelectedMessage => {
                self.label_selected_message(self.config.labeler_backend);
            }
            Message::LabelSelectedWith(backend) => {
                self.label_selected_message(backend);
            }
            Message::LabelCheckedWith(backend) => {
                self.label_checked_messages(backend);
            }
            Message::LabelVisibleMessages => {
                self.label_visible_messages();
            }
            Message::ToggleManualLabel(message_id, label) => {
                self.toggle_manual_label(message_id, &label);
            }
            Message::ToggleManualLabelOnChecked(label) => {
                self.toggle_manual_label_on_checked(&label);
            }
            Message::NewLabelNameChanged(value) => self.new_label_name = value,
            Message::NewLabelDescriptionChanged(value) => self.new_label_description = value,
            Message::SaveLabelDefinition => self.save_label_definition(),
            Message::RemoveLabelDefinition(name) => self.remove_label_definition(&name),
            Message::LayaGgufPathChanged(path) => self.config.laya_gguf_path = path,
            Message::ChooseLayaGguf => self.choose_laya_gguf(),
            Message::DownloadLayaGguf => self.download_laya_gguf(),
            Message::ToggleChatPane => self.chat_open = !self.chat_open,
            Message::ChatWidthChanged(width) => self.chat_width = width.clamp(310.0, 760.0),
            Message::AskAboutEmail(message_id) => self.ask_about_email(message_id),
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
            WorkerResult::Indexed(Ok(outcome)) => {
                self.count = Index::open(&self.db_path)
                    .and_then(|db| db.count())
                    .unwrap_or(self.count);
                self.refresh_sources();
                self.refresh_explorer();
                self.status = if outcome.embedded > 0 {
                    format!(
                        "Indexed {} new messages with BM25; stored {} remote embeddings",
                        outcome.added, outcome.embedded
                    )
                } else {
                    format!("Indexed {} new messages with local BM25", outcome.added)
                };
            }
            WorkerResult::Indexed(Err(error)) => self.status = error,
            WorkerResult::Searched(Ok(hits)) => {
                let count = hits.len();
                self.hits = hits;
                self.show_sources = false;
                self.expanded_hit = None;
                self.chat_open = true;
                self.messages.push(ChatMessage::assistant(
                    format!(
                        "I found {count} matching email{} in your local archive. The sources are shown below.",
                        if count == 1 { "" } else { "s" }
                    ),
                    String::new(),
                ));
                self.status = format!(
                    "Found {count} local results \u{2014} nothing was sent over the network"
                );
            }
            WorkerResult::Searched(Err(error)) => {
                self.chat_open = true;
                self.messages.push(ChatMessage::assistant(
                    format!("I couldn't complete that local search: {error}"),
                    String::new(),
                ));
                self.status = error;
            }
            WorkerResult::Answered(Ok((answer, hits, preview))) => {
                self.chat_open = true;
                self.messages.push(ChatMessage::assistant(answer, preview));
                self.hits = hits;
                self.show_sources = false;
                self.expanded_hit = None;
                self.status = "Answer received".into();
            }
            WorkerResult::Answered(Err(error)) => {
                self.chat_open = true;
                self.messages.push(ChatMessage::assistant(
                    format!("I couldn't answer that question: {error}"),
                    String::new(),
                ));
                self.status = error;
            }
            WorkerResult::Labeled(Ok(outcome)) => {
                self.refresh_explorer();
                self.status = format!(
                    "Labeled {} email{} ({} label{} assigned via {})",
                    outcome.messages_processed,
                    if outcome.messages_processed == 1 {
                        ""
                    } else {
                        "s"
                    },
                    outcome.labels_assigned,
                    if outcome.labels_assigned == 1 {
                        ""
                    } else {
                        "s"
                    },
                    outcome.source_name
                );
            }
            WorkerResult::Labeled(Err(error)) => {
                self.status = format!("Labeling failed: {error}");
            }
            WorkerResult::LayaDownloaded(Ok(path)) => {
                self.config.laya_gguf_path = path.to_string_lossy().to_string();
                let _ = actions::save_config(&self.storage_dir, &self.config);
                self.status = format!("Downloaded Laya GGUF weights to {}", path.display());
            }
            WorkerResult::LayaDownloaded(Err(error)) => {
                self.status = format!("Laya GGUF download failed: {error}");
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        container(row![self.sidebar(), self.explorer(), self.chat_pane()].height(Fill))
            .width(Fill)
            .height(Fill)
            .style(page_style)
            .into()
    }
}
