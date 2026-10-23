mod actions;
mod chat;
mod explorer;
pub(crate) mod markdown;
pub(crate) mod sidebar;
mod theme;

use crate::{
    db::Index,
    llm::LlmConfig,
    model::{
        AssignedLabel, EmailRecord, IndexedSource, LabelDefinition, LabelFilter, MailScope,
        MailSort, SearchHit,
    },
};
use actions::load_config;
use chat::{ChatPaneMessage, chat_question_input_id};
use explorer::{ExplorerMessage, explorer_filter_input_id};
use iced::{
    Element, Fill, Subscription, Task, Theme,
    keyboard::{self, Key, key::Named},
    widget::{container, row, text_input},
};
use markdown::{MdBlock, parse_markdown};
use sidebar::SidebarMessage;
use std::{
    collections::HashSet,
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use theme::{build_app_theme, page_style};

#[derive(Debug, Clone)]
enum WorkerEvent {
    Progress(String),
    EmailLabeled {
        message_id: i64,
        labels: Vec<AssignedLabel>,
    },
    Finished(WorkerResult),
}

#[derive(Debug, Clone)]
enum WorkerResult {
    Indexed(Result<IndexOutcome, String>),
    Searched(Result<Arc<Vec<SearchHit>>, String>),
    Answered(Result<Arc<(String, Vec<SearchHit>, String)>, String>),
    Labeled(Result<LabelOutcome, String>),
    LayaDownloaded(Result<PathBuf, String>),
}

#[derive(Debug, Clone)]
struct IndexOutcome {
    added: usize,
    embedded: usize,
}

#[derive(Debug, Clone)]
struct LabelOutcome {
    messages_processed: usize,
    labels_assigned: usize,
    source_name: String,
    cancelled: bool,
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
    blocks: Vec<MdBlock>,
    context_preview: String,
    show_context: bool,
}

impl ChatMessage {
    fn user(content: String) -> Self {
        let blocks = parse_markdown(&content);
        Self {
            role: ChatRole::User,
            content,
            blocks,
            context_preview: String::new(),
            show_context: false,
        }
    }
    fn assistant(content: String, context_preview: String) -> Self {
        let blocks = parse_markdown(&content);
        Self {
            role: ChatRole::Assistant,
            content,
            blocks,
            context_preview,
            show_context: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum KeyShortcut {
    SelectPrevEmail,
    SelectNextEmail,
    ToggleCheckSelected,
    FocusQuickFilter,
    ClearFilterOrSelection,
}

pub struct PstRagApp {
    theme: Theme,
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
    sidebar_open: bool,
    chat_open: bool,
    chat_width: f32,
    reader_split: f32,
    busy: bool,
    config: LlmConfig,
    cancel_flag: Option<Arc<AtomicBool>>,
}

#[derive(Debug, Clone)]
enum Message {
    Sidebar(SidebarMessage),
    Explorer(ExplorerMessage),
    Chat(ChatPaneMessage),
    ArchiveChosen(Option<PathBuf>),
    StorageDirChosen(Option<PathBuf>),
    LayaGgufChosen(Option<PathBuf>),
    Worker(WorkerEvent),
    KeyPressed(KeyShortcut),
}

pub fn run() -> iced::Result {
    iced::application(
        "PSTbeetle - Mail Explorer and Local RAG",
        PstRagApp::update,
        PstRagApp::view,
    )
    .subscription(PstRagApp::subscription)
    .theme(PstRagApp::theme)
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
            theme: build_app_theme(),
            storage_location_path,
            storage_dir,
            db_path,
            archive: None,
            question: String::new(),
            messages: vec![],
            hits: vec![],
            show_sources: false,
            expanded_hit: None,
            status: "Ready - Thunderbird mail explorer with Laya and LLM Judge labeling".into(),
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
            sidebar_open: true,
            chat_open: true,
            chat_width: 410.0,
            reader_split: 45.0,
            busy: false,
            config,
            cancel_flag: None,
        };
        app.refresh_explorer();
        app
    }

    fn theme(&self) -> Theme {
        self.theme.clone()
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Sidebar(msg) => self.update_sidebar(msg),
            Message::Explorer(msg) => self.update_explorer(msg),
            Message::Chat(msg) => self.update_chat(msg),
            Message::ArchiveChosen(Some(path)) => {
                self.archive = Some(path);
                Task::none()
            }
            Message::ArchiveChosen(None) => Task::none(),
            Message::StorageDirChosen(Some(path)) => {
                self.apply_storage_dir(path);
                Task::none()
            }
            Message::StorageDirChosen(None) => Task::none(),
            Message::LayaGgufChosen(Some(path)) => {
                self.apply_laya_gguf(path);
                Task::none()
            }
            Message::LayaGgufChosen(None) => Task::none(),
            Message::Worker(event) => {
                self.handle_worker_event(event);
                Task::none()
            }
            Message::KeyPressed(shortcut) => self.handle_key_shortcut(shortcut),
        }
    }

    fn update_sidebar(&mut self, message: SidebarMessage) -> Task<Message> {
        match message {
            SidebarMessage::ToggleSidebar => {
                self.sidebar_open = !self.sidebar_open;
                Task::none()
            }
            SidebarMessage::ToggleSection(section) => {
                let index = section.index();
                self.collapsed_sections[index] = !self.collapsed_sections[index];
                Task::none()
            }
            SidebarMessage::ChooseArchive => self.choose_archive(),
            SidebarMessage::Index => self.index(),
            SidebarMessage::ChooseStorage => self.choose_storage_dir(),
            SidebarMessage::SaveCredentials => {
                self.save_credentials();
                Task::none()
            }
            SidebarMessage::SelectMailScope(scope) => {
                self.mail_scope = scope;
                self.refresh_explorer_messages();
                Task::none()
            }
            SidebarMessage::SourceEnabledChanged(source, enabled) => {
                self.set_source_enabled(&source, enabled);
                Task::none()
            }
            SidebarMessage::FolderTreeEnabledChanged(source, folders, enabled) => {
                self.set_folders_enabled(&source, &folders, enabled);
                Task::none()
            }
            SidebarMessage::ToggleFolderTreeNode(source, folder) => {
                let key = (source, folder);
                if !self.collapsed_source_folders.remove(&key) {
                    self.collapsed_source_folders.insert(key);
                }
                Task::none()
            }
            SidebarMessage::RemoveSourceRequested(source) => {
                self.pending_remove_source = Some(source);
                Task::none()
            }
            SidebarMessage::ConfirmRemoveSource => {
                self.remove_pending_source();
                Task::none()
            }
            SidebarMessage::CancelRemoveSource => {
                self.pending_remove_source = None;
                Task::none()
            }
            SidebarMessage::SelectLabelFilter(filter) => {
                self.label_filter = filter;
                self.refresh_explorer_messages();
                Task::none()
            }
            SidebarMessage::NewLabelNameChanged(value) => {
                self.new_label_name = value;
                Task::none()
            }
            SidebarMessage::NewLabelDescriptionChanged(value) => {
                self.new_label_description = value;
                Task::none()
            }
            SidebarMessage::SaveLabelDefinition => {
                self.save_label_definition();
                Task::none()
            }
            SidebarMessage::RemoveLabelDefinition(name) => {
                self.remove_label_definition(&name);
                Task::none()
            }
            SidebarMessage::LayaGgufPathChanged(path) => {
                self.config.laya_gguf_path = path;
                Task::none()
            }
            SidebarMessage::ChooseLayaGguf => self.choose_laya_gguf(),
            SidebarMessage::DownloadLayaGguf => self.download_laya_gguf(),
            SidebarMessage::EndpointChanged(value) => {
                self.config.endpoint = value;
                Task::none()
            }
            SidebarMessage::ModelChanged(value) => {
                self.config.model = value;
                Task::none()
            }
            SidebarMessage::TokenChanged(value) => {
                self.config.bearer_token = value;
                Task::none()
            }
            SidebarMessage::RemoteEmbeddingsChanged(value) => {
                self.config.use_remote_embeddings = value;
                Task::none()
            }
            SidebarMessage::EmbeddingModelChanged(value) => {
                self.config.embedding_model = value;
                Task::none()
            }
        }
    }

    fn update_explorer(&mut self, message: ExplorerMessage) -> Task<Message> {
        match message {
            ExplorerMessage::ToggleSidebar => {
                self.sidebar_open = !self.sidebar_open;
                Task::none()
            }
            ExplorerMessage::ToggleChatPane => {
                self.chat_open = !self.chat_open;
                Task::none()
            }
            ExplorerMessage::SelectEmail(id) => {
                self.select_email(id);
                Task::none()
            }
            ExplorerMessage::ToggleCheckEmail(id, checked) => {
                self.toggle_check_email(id, checked);
                Task::none()
            }
            ExplorerMessage::ToggleCheckAllVisible(checked) => {
                self.toggle_check_all_visible(checked);
                Task::none()
            }
            ExplorerMessage::SelectUnlabeledVisible => {
                self.select_unlabeled_visible();
                Task::none()
            }
            ExplorerMessage::SelectRangeTo(id) => {
                self.select_range_to(id);
                Task::none()
            }
            ExplorerMessage::ClearCheckedEmails => {
                self.checked_message_ids.clear();
                self.status = "Cleared mail selection".into();
                Task::none()
            }
            ExplorerMessage::SelectLabelFilter(filter) => {
                self.label_filter = filter;
                self.refresh_explorer_messages();
                Task::none()
            }
            ExplorerMessage::FilterChanged(query) => {
                self.filter_query = query;
                self.refresh_explorer_messages();
                Task::none()
            }
            ExplorerMessage::SetMailSort(sort) => {
                self.mail_sort = sort;
                self.refresh_explorer_messages();
                Task::none()
            }
            ExplorerMessage::SetLabelerBackend(backend) => {
                self.config.labeler_backend = backend;
                self.status = format!("Active mail labeler switched to {}", backend.display_name());
                Task::none()
            }
            ExplorerMessage::LabelFocusedEmail => {
                self.label_selected_message(self.config.labeler_backend)
            }
            ExplorerMessage::LabelSelectedWith(backend) => self.label_selected_message(backend),
            ExplorerMessage::LabelCheckedWith(backend) => self.label_checked_messages(backend),
            ExplorerMessage::LabelVisibleMessages => self.label_visible_messages(),
            ExplorerMessage::ToggleManualLabel(message_id, label) => {
                self.toggle_manual_label(message_id, &label);
                Task::none()
            }
            ExplorerMessage::ToggleManualLabelOnChecked(label) => {
                self.toggle_manual_label_on_checked(&label);
                Task::none()
            }
            ExplorerMessage::AskAboutEmail(message_id) => self.ask_about_email(message_id),
            ExplorerMessage::ReaderSplitChanged(split) => {
                self.reader_split = split.clamp(20.0, 80.0);
                Task::none()
            }
            ExplorerMessage::CancelWorker => {
                if let Some(flag) = &self.cancel_flag {
                    flag.store(true, Ordering::Relaxed);
                }
                self.status = "Cancelling background task...".into();
                Task::none()
            }
        }
    }

    fn update_chat(&mut self, message: ChatPaneMessage) -> Task<Message> {
        match message {
            ChatPaneMessage::ToggleChatPane => {
                self.chat_open = !self.chat_open;
                Task::none()
            }
            ChatPaneMessage::WidthChanged(width) => {
                self.chat_width = width.clamp(310.0, 760.0);
                Task::none()
            }
            ChatPaneMessage::NewChat => {
                self.new_chat();
                Task::none()
            }
            ChatPaneMessage::QuestionChanged(value) => {
                self.question = value;
                Task::none()
            }
            ChatPaneMessage::Ask => self.ask(),
            ChatPaneMessage::SearchLocally => self.search_locally(),
            ChatPaneMessage::UsePrompt(prompt) => {
                self.question = prompt.into();
                self.chat_open = true;
                text_input::focus(chat_question_input_id())
            }
            ChatPaneMessage::ToggleContext(index, value) => {
                if let Some(message) = self.messages.get_mut(index) {
                    message.show_context = value;
                }
                Task::none()
            }
            ChatPaneMessage::ToggleSources => {
                self.show_sources = !self.show_sources;
                Task::none()
            }
            ChatPaneMessage::ToggleHit(index) => {
                self.expanded_hit = (self.expanded_hit != Some(index)).then_some(index);
                Task::none()
            }
            ChatPaneMessage::SelectEmail(id) => {
                self.select_email(id);
                Task::none()
            }
            ChatPaneMessage::MarkdownLinkClicked(url) => {
                self.status = format!("Link: {url}");
                Task::none()
            }
        }
    }

    fn handle_key_shortcut(&mut self, shortcut: KeyShortcut) -> Task<Message> {
        match shortcut {
            KeyShortcut::SelectPrevEmail => {
                self.select_adjacent_email(-1);
                Task::none()
            }
            KeyShortcut::SelectNextEmail => {
                self.select_adjacent_email(1);
                Task::none()
            }
            KeyShortcut::ToggleCheckSelected => {
                self.toggle_check_selected_email();
                Task::none()
            }
            KeyShortcut::FocusQuickFilter => text_input::focus(explorer_filter_input_id()),
            KeyShortcut::ClearFilterOrSelection => {
                if !self.filter_query.is_empty() {
                    self.filter_query.clear();
                    self.refresh_explorer_messages();
                } else if !self.checked_message_ids.is_empty() {
                    self.checked_message_ids.clear();
                    self.status = "Cleared mail selection".into();
                }
                Task::none()
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        keyboard::on_key_press(|key, modifiers| {
            if modifiers.command() || modifiers.control() || modifiers.alt() {
                return None;
            }
            match key.as_ref() {
                Key::Named(Named::ArrowUp) | Key::Character("k") => {
                    Some(Message::KeyPressed(KeyShortcut::SelectPrevEmail))
                }
                Key::Named(Named::ArrowDown) | Key::Character("j") => {
                    Some(Message::KeyPressed(KeyShortcut::SelectNextEmail))
                }
                Key::Named(Named::Space) | Key::Character("x") => {
                    Some(Message::KeyPressed(KeyShortcut::ToggleCheckSelected))
                }
                Key::Character("/") => Some(Message::KeyPressed(KeyShortcut::FocusQuickFilter)),
                Key::Named(Named::Escape) => {
                    Some(Message::KeyPressed(KeyShortcut::ClearFilterOrSelection))
                }
                _ => None,
            }
        })
    }

    fn handle_worker_event(&mut self, event: WorkerEvent) {
        match event {
            WorkerEvent::Progress(status) => {
                self.status = status;
            }
            WorkerEvent::EmailLabeled { message_id, labels } => {
                if let Some(email) = self
                    .explorer_messages
                    .iter_mut()
                    .find(|m| m.id == message_id)
                {
                    let mut merged: Vec<AssignedLabel> = email
                        .labels
                        .iter()
                        .filter(|l| l.source == "manual")
                        .cloned()
                        .collect();
                    for assigned in labels {
                        if !merged.iter().any(|l| l.label == assigned.label) {
                            merged.push(assigned);
                        }
                    }
                    email.labels = merged;
                }
            }
            WorkerEvent::Finished(result) => {
                self.busy = false;
                self.cancel_flag = None;
                self.handle_worker_result(result);
            }
        }
    }

    fn handle_worker_result(&mut self, result: WorkerResult) {
        match result {
            WorkerResult::Indexed(Ok(outcome)) => {
                self.count = Index::open(&self.db_path)
                    .and_then(|db| db.count())
                    .unwrap_or(self.count);
                self.refresh_sources();
                self.refresh_explorer();
                self.status = if outcome.embedded > 0 {
                    format!(
                        "Indexed {} new messages with BM25 and stored {} remote embeddings",
                        outcome.added, outcome.embedded
                    )
                } else {
                    format!("Indexed {} new messages with local BM25", outcome.added)
                };
            }
            WorkerResult::Indexed(Err(error)) => self.status = error,
            WorkerResult::Searched(Ok(hits)) => {
                let hits = Arc::unwrap_or_clone(hits);
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
                self.status =
                    format!("Found {count} local results - nothing was sent over the network");
            }
            WorkerResult::Searched(Err(error)) => {
                self.chat_open = true;
                self.messages.push(ChatMessage::assistant(
                    format!("I could not complete that local search: {error}"),
                    String::new(),
                ));
                self.status = error;
            }
            WorkerResult::Answered(Ok(payload)) => {
                let (answer, hits, preview) = Arc::unwrap_or_clone(payload);
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
                    format!("I could not answer that question: {error}"),
                    String::new(),
                ));
                self.status = error;
            }
            WorkerResult::Labeled(Ok(outcome)) => {
                self.refresh_explorer();
                let prefix = if outcome.cancelled {
                    "Stopped labeling after"
                } else {
                    "Labeled"
                };
                self.status = format!(
                    "{} {} email{} ({} label{} assigned via {})",
                    prefix,
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
                self.refresh_explorer();
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
        container(
            row![
                self.sidebar().map(Message::Sidebar),
                self.explorer().map(Message::Explorer),
                self.chat_pane().map(Message::Chat),
            ]
            .height(Fill),
        )
        .width(Fill)
        .height(Fill)
        .style(page_style)
        .into()
    }
}
