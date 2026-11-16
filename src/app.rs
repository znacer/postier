mod actions;
mod chat;
pub(crate) mod email_html;
mod explorer;
pub(crate) mod markdown;
pub(crate) mod sidebar;
mod theme;

use std::{
    cell::Cell,
    collections::HashSet,
    fs,
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use dioxus::{
    desktop::{
        Config, LogicalSize, WindowBuilder, WindowEvent, tao, use_window, use_wry_event_handler,
    },
    prelude::*,
};

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
use markdown::{MdBlock, parse_markdown};
use sidebar::{FolderContextMenu, SidebarMessage, collapsible_folder_paths_under};
use theme::{APP_CSS, DX_COMPONENTS_CSS, DX_COMPONENTS_THEME_CSS, SplitAxis, page_style, splitter};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

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
    HeaderMeasured(Option<f32>),
    Moved(Point),
    Released,
    WindowResized(Size),
}

const SIDEBAR_WIDTH: f32 = 285.0;
const SIDEBAR_COLLAPSED_WIDTH: f32 = 42.0;
const CHAT_MIN_WIDTH: f32 = 310.0;
const EXPLORER_MIN_WIDTH: f32 = 420.0;
const TABLE_MIN_HEIGHT: f32 = 100.0;
/// Approximate vertical space used by the toolbar, status bar and a minimal reader.
const TABLE_RESERVED_HEIGHT: f32 = 330.0;
/// Smallest mail header height (roughly the subject line plus padding).
const HEADER_MIN_HEIGHT: f32 = 44.0;
/// Tallest the mail header may grow, even in automatic sizing (e.g. mails with
/// long To/Cc lists). Anything beyond this scrolls inside the header.
const HEADER_MAX_HEIGHT: f32 = 210.0;

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
    Settings,
}

impl SidebarSection {
    fn index(self) -> usize {
        match self {
            Self::Mailboxes => 0,
            Self::Labels => 1,
            Self::Settings => 2,
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
    SelectPrevEmail { extend_range: bool },
    SelectNextEmail { extend_range: bool },
    ToggleCheckSelected,
    FocusQuickFilter,
    ClearFilterOrSelection,
}

type WorkerFn =
    Box<dyn FnOnce(&tokio::sync::mpsc::UnboundedSender<WorkerEvent>) -> WorkerResult + Send>;

enum AppTask {
    None,
    FocusInput(&'static str),
    MeasureHeaderAndStartResize,
    Async(Pin<Box<dyn Future<Output = Message> + 'static>>),
    Worker(WorkerFn),
}

impl AppTask {
    pub(super) fn none() -> Self {
        Self::None
    }

    pub(super) fn perform<F, T>(fut: F, map: fn(T) -> Message) -> Self
    where
        F: Future<Output = T> + 'static,
        T: 'static,
    {
        Self::Async(Box::pin(async move { map(fut.await) }))
    }

    pub(super) fn focus(id: &'static str) -> Self {
        Self::FocusInput(id)
    }
}

pub struct PstRagApp {
    storage_location_path: PathBuf,
    storage_dir: PathBuf,
    db_path: PathBuf,
    db: Option<Index>,
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
    folder_context_menu: Option<FolderContextMenu>,
    collapsed_sections: [bool; 3],
    explorer_messages: Vec<EmailRecord>,
    selected_message_id: Option<i64>,
    range_anchor_id: Option<i64>,
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

trait DispatchMessage: Clone + 'static {
    fn dispatch(self);
}

impl DispatchMessage for () {
    fn dispatch(self) {}
}

impl DispatchMessage for Message {
    fn dispatch(self) {
        dispatch_message(self);
    }
}

impl DispatchMessage for SidebarMessage {
    fn dispatch(self) {
        dispatch_message(Message::Sidebar(self));
    }
}

impl DispatchMessage for ExplorerMessage {
    fn dispatch(self) {
        dispatch_message(Message::Explorer(self));
    }
}

impl DispatchMessage for ChatPaneMessage {
    fn dispatch(self) {
        dispatch_message(Message::Chat(self));
    }
}

thread_local! {
    static APP_SIGNAL: Cell<Option<Signal<PstRagApp>>> = const { Cell::new(None) };
}

fn dispatch_message(message: Message) {
    let Some(mut signal) = APP_SIGNAL.with(Cell::get) else {
        return;
    };
    let task = signal.write().update(message);
    execute_task(task);
}

fn execute_task(task: AppTask) {
    match task {
        AppTask::None => {}
        AppTask::FocusInput(id) => {
            let _ = document::eval(&format!("document.getElementById('{id}')?.focus();"));
        }
        AppTask::MeasureHeaderAndStartResize => {
            let header_id = reader_header_id();
            spawn(async move {
                let js = format!(
                    "let el = document.getElementById('{header_id}'); return el ? el.getBoundingClientRect().height : null;"
                );
                let measured = match document::eval(&js).await {
                    Ok(val) => val.as_f64().map(|v| v as f32),
                    Err(_) => None,
                };
                dispatch_message(Message::Resize(ResizeEvent::HeaderMeasured(measured)));
            });
        }
        AppTask::Async(fut) => {
            spawn(async move {
                let msg = fut.await;
                dispatch_message(msg);
            });
        }
        AppTask::Worker(work) => {
            spawn(async move {
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<WorkerEvent>();
                std::thread::spawn(move || {
                    let result = work(&tx);
                    let _ = tx.send(WorkerEvent::Finished(result));
                });
                while let Some(event) = rx.recv().await {
                    dispatch_message(Message::Worker(event));
                }
            });
        }
    }
}

pub(crate) fn load_app_icon() -> Option<tao::window::Icon> {
    let img = image::load_from_memory(include_bytes!("../assets/logo.png"))
        .ok()?
        .into_rgba8();
    let (width, height) = img.dimensions();
    tao::window::Icon::from_rgba(img.into_raw(), width, height).ok()
}

pub fn run() {
    let initial_title = LlmConfig::default().language.app_title().to_owned();
    let window = WindowBuilder::new()
        .with_title(initial_title)
        .with_window_icon(load_app_icon())
        .with_inner_size(LogicalSize::new(1440.0, 860.0))
        .with_always_on_top(false);
    let config = Config::new()
        .with_window(window)
        .with_custom_head(r#"<meta charset="utf-8">"#.to_string())
        .with_background_color((11, 15, 23, 255));

    LaunchBuilder::desktop().with_cfg(config).launch(app_root);
}

fn app_root() -> Element {
    let app_signal = use_signal(PstRagApp::new);
    APP_SIGNAL.with(|cell| cell.set(Some(app_signal)));

    let window_ctx = use_window();
    let current_title = app_signal.read().title();
    window_ctx.set_title(&current_title);

    use_wry_event_handler(move |event, _| {
        if let tao::event::Event::WindowEvent {
            event: WindowEvent::Resized(physical_size),
            ..
        } = event
        {
            let scale = use_window().scale_factor();
            let logical = physical_size.to_logical::<f64>(scale);
            dispatch_message(Message::Resize(ResizeEvent::WindowResized(Size::new(
                logical.width as f32,
                logical.height as f32,
            ))));
        }
    });

    let is_dragging = app_signal.read().resizing.is_some();
    let is_dark = app_signal.read().config.dark_mode;
    let theme_name = if is_dark { "dark" } else { "light" };

    use_effect(move || {
        let theme = if app_signal.read().config.dark_mode {
            "dark"
        } else {
            "light"
        };
        let js = format!("document.documentElement.setAttribute('data-theme', '{theme}');");
        let _ = document::eval(&js);
    });

    let view = app_signal.read().view();
    let bg_style = page_style();

    rsx! {
        style { "{DX_COMPONENTS_THEME_CSS}" }
        style { "{DX_COMPONENTS_CSS}" }
        style { "{APP_CSS}" }
        div {
            tabindex: "0",
            "data-theme": "{theme_name}",
            style: "width: 100vw; height: 100vh; overflow: hidden; outline: none; {bg_style}",
            onkeydown: move |evt| {
                let mods = evt.modifiers();
                if mods.meta() || mods.ctrl() || mods.alt() {
                    return;
                }
                let shift = mods.shift();
                let shortcut = match evt.key() {
                    Key::ArrowUp => {
                        evt.prevent_default();
                        Some(KeyShortcut::SelectPrevEmail {
                            extend_range: shift,
                        })
                    }
                    Key::ArrowDown => {
                        evt.prevent_default();
                        Some(KeyShortcut::SelectNextEmail {
                            extend_range: shift,
                        })
                    }
                    Key::Escape => Some(KeyShortcut::ClearFilterOrSelection),
                    Key::Character(ref ch) => match ch.as_str() {
                        "k" | "K" => Some(KeyShortcut::SelectPrevEmail {
                            extend_range: shift,
                        }),
                        "j" | "J" => Some(KeyShortcut::SelectNextEmail {
                            extend_range: shift,
                        }),
                        " " | "x" => Some(KeyShortcut::ToggleCheckSelected),
                        "/" => {
                            evt.prevent_default();
                            Some(KeyShortcut::FocusQuickFilter)
                        }
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(sc) = shortcut {
                    dispatch_message(Message::KeyPressed(sc));
                }
            },
            onmousemove: move |evt| {
                if is_dragging {
                    let coords = evt.client_coordinates();
                    dispatch_message(Message::Resize(ResizeEvent::Moved(Point::new(
                        coords.x as f32,
                        coords.y as f32,
                    ))));
                }
            },
            onmouseup: move |_| {
                if is_dragging {
                    dispatch_message(Message::Resize(ResizeEvent::Released));
                }
            },
            {view}
        }
    }
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
        let db = Index::open(&db_path).ok();
        let (count, sources) = db
            .as_ref()
            .map(|d| (d.count().unwrap_or(0), d.sources().unwrap_or_default()))
            .unwrap_or_default();
        let mut app = Self {
            storage_location_path,
            storage_dir,
            db_path,
            db,
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
            folder_context_menu: None,
            collapsed_sections: [false, false, true],
            explorer_messages: vec![],
            selected_message_id: None,
            range_anchor_id: None,
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

    fn update(&mut self, message: Message) -> AppTask {
        match message {
            Message::Sidebar(msg) => self.update_sidebar(msg),
            Message::Explorer(msg) => self.update_explorer(msg),
            Message::Chat(msg) => self.update_chat(msg),
            Message::ArchiveChosen(Some(path)) => {
                self.archive = Some(path);
                AppTask::none()
            }
            Message::ArchiveChosen(None) => AppTask::none(),
            Message::StorageDirChosen(Some(path)) => {
                self.apply_storage_dir(path);
                AppTask::none()
            }
            Message::StorageDirChosen(None) => AppTask::none(),
            Message::LayaGgufChosen(Some(path)) => {
                self.apply_laya_gguf(path);
                AppTask::none()
            }
            Message::LayaGgufChosen(None) => AppTask::none(),
            Message::Worker(event) => {
                self.handle_worker_event(event);
                AppTask::none()
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

    fn handle_resize_event(&mut self, event: ResizeEvent) -> AppTask {
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
                    return AppTask::MeasureHeaderAndStartResize;
                }
            }
            ResizeEvent::HeaderMeasured(height) => {
                if let Some(height) = height {
                    self.reader_header_natural = height;
                    if let Some(active) = self.resizing.as_mut()
                        && active.splitter == Splitter::HeaderHeight
                        && active.start_value.is_none()
                    {
                        active.start_value = Some(height);
                    }
                } else {
                    // Nothing to measure (no mail shown): abandon the drag.
                    self.resizing = None;
                }
            }
            ResizeEvent::Moved(position) => {
                let Some(active) = self.resizing.as_mut() else {
                    return AppTask::none();
                };
                let anchor = *active.anchor.get_or_insert(position);
                let Some(start_value) = active.start_value else {
                    return AppTask::none();
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
        AppTask::none()
    }

    fn update_sidebar(&mut self, message: SidebarMessage) -> AppTask {
        match message {
            SidebarMessage::ToggleSidebar => {
                self.sidebar_open = !self.sidebar_open;
                self.folder_context_menu = None;
                AppTask::none()
            }
            SidebarMessage::ToggleSection(section) => {
                let index = section.index();
                self.collapsed_sections[index] = !self.collapsed_sections[index];
                AppTask::none()
            }
            SidebarMessage::ChooseArchive => self.choose_archive(),
            SidebarMessage::Index => self.index(),
            SidebarMessage::ChooseStorage => self.choose_storage_dir(),
            SidebarMessage::SaveCredentials => {
                self.save_credentials();
                AppTask::none()
            }
            SidebarMessage::SelectMailScope(scope) => {
                self.mail_scope = scope;
                self.refresh_explorer_messages();
                AppTask::none()
            }
            SidebarMessage::SourceEnabledChanged(source, enabled) => {
                self.set_source_enabled(&source, enabled);
                AppTask::none()
            }
            SidebarMessage::FolderTreeEnabledChanged(source, folders, enabled) => {
                self.set_folders_enabled(&source, &folders, enabled);
                AppTask::none()
            }
            SidebarMessage::ToggleFolderTreeNode(source, folder) => {
                let key = (source, folder);
                if !self.collapsed_source_folders.remove(&key) {
                    self.collapsed_source_folders.insert(key);
                }
                AppTask::none()
            }
            SidebarMessage::OpenFolderContextMenu {
                source_key,
                folder_path,
                position,
            } => {
                self.folder_context_menu = Some(FolderContextMenu {
                    source_key,
                    folder_path,
                    position,
                });
                AppTask::none()
            }
            SidebarMessage::CloseFolderContextMenu => {
                self.folder_context_menu = None;
                AppTask::none()
            }
            SidebarMessage::CollapseAllSubdirectories(source_key, folder_prefix) => {
                self.folder_context_menu = None;
                if let Some(source) = self.sources.iter().find(|s| s.key == source_key) {
                    for path in collapsible_folder_paths_under(&source.folders, &folder_prefix) {
                        self.collapsed_source_folders
                            .insert((source_key.clone(), path));
                    }
                }
                AppTask::none()
            }
            SidebarMessage::ExpandAllSubdirectories(source_key, folder_prefix) => {
                self.folder_context_menu = None;
                if let Some(source) = self.sources.iter().find(|s| s.key == source_key) {
                    for path in collapsible_folder_paths_under(&source.folders, &folder_prefix) {
                        self.collapsed_source_folders
                            .remove(&(source_key.clone(), path));
                    }
                }
                AppTask::none()
            }
            SidebarMessage::RemoveSourceRequested(source) => {
                self.pending_remove_source = Some(source);
                AppTask::none()
            }
            SidebarMessage::ConfirmRemoveSource => {
                self.remove_pending_source();
                AppTask::none()
            }
            SidebarMessage::CancelRemoveSource => {
                self.pending_remove_source = None;
                AppTask::none()
            }
            SidebarMessage::SelectLabelFilter(filter) => {
                self.label_filter = filter;
                self.refresh_explorer_messages();
                AppTask::none()
            }
            SidebarMessage::NewLabelNameChanged(value) => {
                self.new_label_name = value;
                AppTask::none()
            }
            SidebarMessage::NewLabelDescriptionChanged(value) => {
                self.new_label_description = value;
                AppTask::none()
            }
            SidebarMessage::SaveLabelDefinition => {
                self.save_label_definition();
                AppTask::none()
            }
            SidebarMessage::RemoveLabelDefinition(name) => {
                self.remove_label_definition(&name);
                AppTask::none()
            }
            SidebarMessage::LayaGgufPathChanged(path) => {
                self.config.laya_gguf_path = path;
                AppTask::none()
            }
            SidebarMessage::ChooseLayaGguf => self.choose_laya_gguf(),
            SidebarMessage::EndpointChanged(value) => {
                self.config.endpoint = value;
                AppTask::none()
            }
            SidebarMessage::ModelChanged(value) => {
                self.config.model = value;
                AppTask::none()
            }
            SidebarMessage::TokenChanged(value) => {
                self.config.bearer_token = value;
                AppTask::none()
            }
            SidebarMessage::RemoteEmbeddingsChanged(value) => {
                self.config.use_remote_embeddings = value;
                AppTask::none()
            }
            SidebarMessage::EmbeddingModelChanged(value) => {
                self.config.embedding_model = value;
                AppTask::none()
            }
            SidebarMessage::ContextEmailsChanged(value) => {
                if let Ok(count) = value.trim().parse::<usize>() {
                    self.config.max_context_emails = count.clamp(
                        crate::llm::MIN_CONTEXT_EMAILS,
                        self.config.max_search_results_limit(),
                    );
                }
                AppTask::none()
            }
            SidebarMessage::MaxContextTokensChanged(value) => {
                if let Ok(tokens) = value.trim().parse::<usize>() {
                    self.config.max_context_tokens = tokens;
                }
                AppTask::none()
            }
            SidebarMessage::LanguageChanged(language) => {
                self.set_language(language);
                AppTask::none()
            }
            SidebarMessage::ThemeChanged(dark_mode) => {
                self.set_theme_mode(dark_mode);
                AppTask::none()
            }
        }
    }

    fn set_theme_mode(&mut self, dark_mode: bool) {
        if self.config.dark_mode != dark_mode {
            self.config.dark_mode = dark_mode;
            let _ = actions::save_config(&self.storage_dir, &self.config);
        }
    }

    fn set_language(&mut self, language: crate::i18n::Language) {
        if self.config.language != language {
            self.config.language = language;
            let _ = actions::save_config(&self.storage_dir, &self.config);
            self.status = language.status_language_changed().to_string();
        }
    }

    fn update_explorer(&mut self, message: ExplorerMessage) -> AppTask {
        match message {
            ExplorerMessage::ToggleSidebar => {
                self.sidebar_open = !self.sidebar_open;
                AppTask::none()
            }
            ExplorerMessage::ToggleChatPane => {
                self.chat_open = !self.chat_open;
                AppTask::none()
            }
            ExplorerMessage::SelectEmail(id) => {
                self.select_email(id);
                AppTask::none()
            }
            ExplorerMessage::ToggleCheckEmail(id, checked) => {
                self.toggle_check_email(id, checked);
                AppTask::none()
            }
            ExplorerMessage::ToggleCheckAllVisible(checked) => {
                self.toggle_check_all_visible(checked);
                AppTask::none()
            }
            ExplorerMessage::SelectUnlabeledVisible => {
                self.select_unlabeled_visible();
                AppTask::none()
            }
            ExplorerMessage::SelectRangeTo(id) => {
                self.select_range_to(id);
                AppTask::none()
            }
            ExplorerMessage::ClearCheckedEmails => {
                self.checked_message_ids.clear();
                self.status = self.config.language.status_cleared_selection().into();
                AppTask::none()
            }
            ExplorerMessage::SelectLabelFilter(filter) => {
                self.label_filter = filter;
                self.refresh_explorer_messages();
                AppTask::none()
            }
            ExplorerMessage::FilterChanged(query) => {
                self.filter_query = query;
                self.refresh_explorer_messages();
                AppTask::none()
            }
            ExplorerMessage::SetMailSort(sort) => {
                self.mail_sort = sort;
                self.refresh_explorer_messages();
                AppTask::none()
            }
            ExplorerMessage::SetLabelerBackend(backend) => {
                self.config.labeler_backend = backend;
                self.status = match self.config.language {
                    crate::i18n::Language::English => format!(
                        "Active mail labeler switched to {}",
                        self.config.language.backend_name(backend)
                    ),
                    crate::i18n::Language::French => format!(
                        "Moteur d'étiquetage actif basculé sur {}",
                        self.config.language.backend_name(backend)
                    ),
                };
                AppTask::none()
            }
            ExplorerMessage::LabelFocusedEmail => {
                self.label_selected_message(self.config.labeler_backend)
            }
            ExplorerMessage::LabelSelectedWith(backend) => self.label_selected_message(backend),
            ExplorerMessage::LabelCheckedWith(backend) => self.label_checked_messages(backend),
            ExplorerMessage::LabelVisibleMessages => self.label_visible_messages(),
            ExplorerMessage::ToggleManualLabel(message_id, label) => {
                self.toggle_manual_label(message_id, &label);
                AppTask::none()
            }
            ExplorerMessage::ToggleManualLabelOnChecked(label) => {
                self.toggle_manual_label_on_checked(&label);
                AppTask::none()
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
                    crate::i18n::Language::French => {
                        "Annulation de la tâche en arrière-plan...".into()
                    }
                };
                AppTask::none()
            }
        }
    }

    fn update_chat(&mut self, message: ChatPaneMessage) -> AppTask {
        match message {
            ChatPaneMessage::ToggleChatPane => {
                self.chat_open = !self.chat_open;
                AppTask::none()
            }
            ChatPaneMessage::NewChat => {
                self.new_chat();
                AppTask::none()
            }
            ChatPaneMessage::QuestionChanged(value) => {
                self.question = value;
                AppTask::none()
            }
            ChatPaneMessage::Ask => self.ask(),
            ChatPaneMessage::SearchLocally => self.search_locally(),
            ChatPaneMessage::UsePrompt(prompt) => {
                self.question = prompt.into();
                self.chat_open = true;
                AppTask::focus(chat_question_input_id())
            }
            ChatPaneMessage::ToggleContext(index, value) => {
                if let Some(message) = self.messages.get_mut(index) {
                    message.show_context = value;
                }
                AppTask::none()
            }
            ChatPaneMessage::ToggleSources => {
                self.show_sources = !self.show_sources;
                AppTask::none()
            }
            ChatPaneMessage::ToggleHit(index) => {
                self.expanded_hit = (self.expanded_hit != Some(index)).then_some(index);
                AppTask::none()
            }
            ChatPaneMessage::SelectEmail(id) => {
                self.select_email(id);
                AppTask::none()
            }
            ChatPaneMessage::MarkdownLinkClicked(url) => {
                self.status = format!("Link: {url}");
                AppTask::none()
            }
        }
    }

    fn handle_key_shortcut(&mut self, shortcut: KeyShortcut) -> AppTask {
        match shortcut {
            KeyShortcut::SelectPrevEmail { extend_range } => {
                self.select_adjacent_email(-1, extend_range);
                AppTask::none()
            }
            KeyShortcut::SelectNextEmail { extend_range } => {
                self.select_adjacent_email(1, extend_range);
                AppTask::none()
            }
            KeyShortcut::ToggleCheckSelected => {
                self.toggle_check_selected_email();
                AppTask::none()
            }
            KeyShortcut::FocusQuickFilter => AppTask::focus(explorer_filter_input_id()),
            KeyShortcut::ClearFilterOrSelection => {
                if self.folder_context_menu.is_some() {
                    self.folder_context_menu = None;
                } else if !self.filter_query.is_empty() {
                    self.filter_query.clear();
                    self.refresh_explorer_messages();
                } else if !self.checked_message_ids.is_empty() {
                    self.checked_message_ids.clear();
                    self.status = self.config.language.status_cleared_selection().into();
                }
                AppTask::none()
            }
        }
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
        }
    }

    fn view(&self) -> Element {
        let sb = self.sidebar();
        let exp = self.explorer();
        let chat_splitter = if self.chat_open {
            Some(splitter(
                SplitAxis::Vertical,
                self.is_resizing(Splitter::ChatWidth),
                Message::Resize(ResizeEvent::Start(Splitter::ChatWidth)),
            ))
        } else {
            None
        };
        let cp = self.chat_pane();

        rsx! {
            div {
                style: "width: 100%; height: 100%; display: flex; flex-direction: row; overflow: hidden;",
                {sb}
                {exp}
                if let Some(sp) = chat_splitter {
                    {sp}
                }
                {cp}
            }
        }
    }
}
