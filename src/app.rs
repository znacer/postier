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
use explorer::{ExplorerMessage, explorer_filter_input_id, reader_header_id};
use iced::{
    Element, Fill, Point, Rectangle, Size, Subscription, Task, Theme, event,
    keyboard::{self, Key, key::Named},
    mouse,
    widget::{container, row, text_input},
    window,
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
use theme::{SplitAxis, build_app_theme, page_style, splitter};

/// Draggable pane boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Splitter {
    /// Left edge of the chat pane.
    ChatWidth,
    /// Boundary between the message table and the reading pane.
    TableHeight,
    /// Boundary between the mail header (subject, from, to) and the body.
    HeaderHeight,
}

/// A boundary drag in progress.
#[derive(Debug, Clone, Copy)]
struct ActiveResize {
    splitter: Splitter,
    /// Pane size when the drag started. `None` while it is still being measured.
    start_value: Option<f32>,
    /// First cursor position seen during the drag, used for relative moves.
    anchor: Option<Point>,
}

#[derive(Debug, Clone)]
enum ResizeEvent {
    Start(Splitter),
    HeaderMeasured(Option<Rectangle>),
    Moved(Point),
    Released,
    WindowResized(Size),
}

const SIDEBAR_WIDTH: f32 = 285.0;
const SIDEBAR_COLLAPSED_WIDTH: f32 = 36.0;
const CHAT_MIN_WIDTH: f32 = 310.0;
const EXPLORER_MIN_WIDTH: f32 = 420.0;
const TABLE_MIN_HEIGHT: f32 = 100.0;
/// Approximate vertical space used by the toolbar, status bar and a minimal reader.
const TABLE_RESERVED_HEIGHT: f32 = 330.0;
/// Smallest mail header height (roughly the subject line plus padding).
const HEADER_MIN_HEIGHT: f32 = 44.0;
/// Tallest the mail header may grow, even in automatic sizing (e.g. mails with
/// long To/Cc lists). Anything beyond this scrolls inside the header.
const HEADER_MAX_HEIGHT: f32 = 160.0;

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
    table_height: f32,
    /// Fixed mail header height, or `None` to size it to its content.
    reader_header_height: Option<f32>,
    /// Last measured natural (unconstrained) mail header height.
    reader_header_natural: f32,
    window_size: Size,
    resizing: Option<ActiveResize>,
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
    Resize(ResizeEvent),
}

pub fn run() -> iced::Result {
    iced::application(
        PstRagApp::title,
        PstRagApp::update,
        PstRagApp::view,
    )
    .subscription(PstRagApp::subscription)
    .theme(PstRagApp::theme)
    .window_size((1440.0, 860.0))
    .run_with(|| (PstRagApp::new(), Task::none()))
}

impl PstRagApp {
    fn title(&self) -> String {
        self.config.language.app_title().to_owned()
    }

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
            status: config.language.status_ready().into(),
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
            table_height: 320.0,
            reader_header_height: None,
            reader_header_natural: f32::INFINITY,
            window_size: Size::new(1440.0, 860.0),
            resizing: None,
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
            Message::Resize(event) => self.handle_resize_event(event),
        }
    }

    fn is_resizing(&self, splitter: Splitter) -> bool {
        self.resizing.is_some_and(|r| r.splitter == splitter)
    }

    fn max_chat_width(&self) -> f32 {
        let sidebar = if self.sidebar_open {
            SIDEBAR_WIDTH
        } else {
            SIDEBAR_COLLAPSED_WIDTH
        };
        (self.window_size.width - sidebar - EXPLORER_MIN_WIDTH).max(CHAT_MIN_WIDTH)
    }

    fn max_table_height(&self) -> f32 {
        (self.window_size.height - TABLE_RESERVED_HEIGHT).max(TABLE_MIN_HEIGHT)
    }

    fn handle_resize_event(&mut self, event: ResizeEvent) -> Task<Message> {
        match event {
            ResizeEvent::Start(splitter) => {
                let start_value = match splitter {
                    Splitter::ChatWidth => Some(self.chat_width),
                    Splitter::TableHeight => Some(self.table_height),
                    Splitter::HeaderHeight => self.reader_header_height,
                };
                self.resizing = Some(ActiveResize {
                    splitter,
                    start_value,
                    anchor: None,
                });
                // An auto-sized header has no known height yet: measure it.
                if start_value.is_none() {
                    return container::visible_bounds(reader_header_id())
                        .map(|bounds| Message::Resize(ResizeEvent::HeaderMeasured(bounds)));
                }
            }
            ResizeEvent::HeaderMeasured(bounds) => {
                if let Some(bounds) = bounds {
                    self.reader_header_natural = bounds.height;
                    if let Some(active) = self.resizing.as_mut()
                        && active.splitter == Splitter::HeaderHeight
                        && active.start_value.is_none()
                    {
                        active.start_value = Some(bounds.height);
                    }
                } else {
                    // Nothing to measure (no mail shown): abandon the drag.
                    self.resizing = None;
                }
            }
            ResizeEvent::Moved(position) => {
                let Some(active) = self.resizing.as_mut() else {
                    return Task::none();
                };
                let anchor = *active.anchor.get_or_insert(position);
                let Some(start_value) = active.start_value else {
                    return Task::none();
                };
                let dy = position.y - anchor.y;
                match active.splitter {
                    // The chat pane is flush with the right edge of the window,
                    // so its width follows the cursor directly.
                    Splitter::ChatWidth => {
                        self.chat_width = (self.window_size.width - position.x)
                            .clamp(CHAT_MIN_WIDTH, self.max_chat_width());
                    }
                    Splitter::TableHeight => {
                        self.table_height =
                            (start_value + dy).clamp(TABLE_MIN_HEIGHT, self.max_table_height());
                    }
                    // Dragging back to (or past) the natural height returns the
                    // header to automatic sizing, so it never shows empty space.
                    Splitter::HeaderHeight => {
                        let height = start_value + dy;
                        self.reader_header_height = (height < self.reader_header_natural)
                            .then(|| height.max(HEADER_MIN_HEIGHT));
                    }
                }
            }
            ResizeEvent::Released => self.resizing = None,
            ResizeEvent::WindowResized(size) => {
                self.window_size = size;
                self.chat_width = self.chat_width.clamp(CHAT_MIN_WIDTH, self.max_chat_width());
                self.table_height = self
                    .table_height
                    .clamp(TABLE_MIN_HEIGHT, self.max_table_height());
            }
        }
        Task::none()
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
            SidebarMessage::LanguageChanged(language) => {
                self.set_language(language);
                Task::none()
            }
        }
    }

    fn set_language(&mut self, language: crate::i18n::Language) {
        if self.config.language != language {
            self.config.language = language;
            let _ = actions::save_config(&self.storage_dir, &self.config);
            self.status = language.status_language_changed().to_string();
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
                self.status = self.config.language.status_cleared_selection().into();
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
                self.status = match self.config.language {
                    crate::i18n::Language::English => format!("Active mail labeler switched to {}", self.config.language.backend_name(backend)),
                    crate::i18n::Language::French => format!("Moteur d'étiquetage actif basculé sur {}", self.config.language.backend_name(backend)),
                };
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
            ExplorerMessage::ReaderResizeStart => {
                self.handle_resize_event(ResizeEvent::Start(Splitter::TableHeight))
            }
            ExplorerMessage::ReaderHeaderResizeStart => {
                self.handle_resize_event(ResizeEvent::Start(Splitter::HeaderHeight))
            }
            ExplorerMessage::CancelWorker => {
                if let Some(flag) = &self.cancel_flag {
                    flag.store(true, Ordering::Relaxed);
                }
                self.status = match self.config.language {
                    crate::i18n::Language::English => "Cancelling background task...".into(),
                    crate::i18n::Language::French => "Annulation de la tâche en arrière-plan...".into(),
                };
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
                    self.status = self.config.language.status_cleared_selection().into();
                }
                Task::none()
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let keys = keyboard::on_key_press(|key, modifiers| {
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
        });
        // Cursor movement is only listened to while a boundary is being dragged,
        // so ordinary mouse motion doesn't trigger updates.
        let window_events = if self.resizing.is_some() {
            event::listen_with(drag_event)
        } else {
            event::listen_with(window_size_event)
        };
        Subscription::batch([keys, window_events])
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
        let tr = self.config.language;
        match result {
            WorkerResult::Indexed(Ok(outcome)) => {
                self.count = Index::open(&self.db_path)
                    .and_then(|db| db.count())
                    .unwrap_or(self.count);
                self.refresh_sources();
                self.refresh_explorer();
                self.status = tr.worker_indexed(outcome.added, outcome.embedded);
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
                    tr.worker_search_found(count),
                    String::new(),
                ));
                self.status = tr.worker_search_status(count);
            }
            WorkerResult::Searched(Err(error)) => {
                self.chat_open = true;
                self.messages.push(ChatMessage::assistant(
                    tr.worker_search_failed(&error),
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
                self.status = tr.worker_answer_received().into();
            }
            WorkerResult::Answered(Err(error)) => {
                self.chat_open = true;
                self.messages.push(ChatMessage::assistant(
                    tr.worker_answer_failed(&error),
                    String::new(),
                ));
                self.status = error;
            }
            WorkerResult::Labeled(Ok(outcome)) => {
                self.refresh_explorer();
                self.status = tr.worker_labeled(
                    outcome.cancelled,
                    outcome.messages_processed,
                    outcome.labels_assigned,
                    &outcome.source_name,
                );
            }
            WorkerResult::Labeled(Err(error)) => {
                self.refresh_explorer();
                self.status = tr.worker_labeling_failed(&error);
            }
            WorkerResult::LayaDownloaded(Ok(path)) => {
                self.config.laya_gguf_path = path.to_string_lossy().to_string();
                let _ = actions::save_config(&self.storage_dir, &self.config);
                self.status = tr.worker_laya_downloaded(&path.display().to_string());
            }
            WorkerResult::LayaDownloaded(Err(error)) => {
                self.status = format!("Laya GGUF download failed: {error}");
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let mut panes = row![
            self.sidebar().map(Message::Sidebar),
            self.explorer().map(Message::Explorer),
        ]
        .height(Fill);
        if self.chat_open {
            panes = panes.push(splitter(
                SplitAxis::Vertical,
                self.is_resizing(Splitter::ChatWidth),
                Message::Resize(ResizeEvent::Start(Splitter::ChatWidth)),
            ));
        }
        panes = panes.push(self.chat_pane().map(Message::Chat));

        container(panes)
            .width(Fill)
            .height(Fill)
            .style(page_style)
            .into()
    }
}

fn window_size_event(event: iced::Event, _: event::Status, _: window::Id) -> Option<Message> {
    match event {
        iced::Event::Window(window::Event::Resized(size))
        | iced::Event::Window(window::Event::Opened { size, .. }) => {
            Some(Message::Resize(ResizeEvent::WindowResized(size)))
        }
        _ => None,
    }
}

fn drag_event(event: iced::Event, status: event::Status, id: window::Id) -> Option<Message> {
    match event {
        iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
            Some(Message::Resize(ResizeEvent::Moved(position)))
        }
        iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
            Some(Message::Resize(ResizeEvent::Released))
        }
        other => window_size_event(other, status, id),
    }
}
