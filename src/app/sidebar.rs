use std::collections::{BTreeMap, HashSet};

use dioxus::prelude::*;
use dioxus_primitives::checkbox::CheckboxState;

use super::{DispatchMessage, PstRagApp, SidebarSection, theme::*};
use crate::{
    components::{
        badge::{Badge, BadgeVariant},
        button::{Button, ButtonSize, ButtonVariant},
        checkbox::Checkbox,
        collapsible::{Collapsible, CollapsibleContent},
        input::Input,
        label::Label,
        scroll_area::ScrollArea,
        separator::Separator,
        sidebar::{
            SidebarContent, SidebarGroup, SidebarGroupContent, SidebarHeader, SidebarMenu,
            SidebarMenuBadge, SidebarMenuItem, SidebarMenuSub, SidebarMenuSubItem,
        },
        switch::Switch,
        toggle_group::{ToggleGroup, ToggleItem},
    },
    model::{IndexedFolder, LabelFilter, MailScope},
};

#[derive(Debug, Clone)]
pub(super) enum SidebarMessage {
    ToggleSidebar,
    ToggleSection(SidebarSection),
    ChooseArchive,
    Index,
    ChooseStorage,
    SaveCredentials,
    SelectMailScope(MailScope),
    SourceEnabledChanged(String, bool),
    FolderTreeEnabledChanged(String, Vec<String>, bool),
    ToggleFolderTreeNode(String, String),
    RemoveSourceRequested(String),
    ConfirmRemoveSource,
    CancelRemoveSource,
    SelectLabelFilter(LabelFilter),
    NewLabelNameChanged(String),
    NewLabelDescriptionChanged(String),
    SaveLabelDefinition,
    RemoveLabelDefinition(String),
    LayaGgufPathChanged(String),
    ChooseLayaGguf,
    DownloadLayaGguf,
    EndpointChanged(String),
    ModelChanged(String),
    TokenChanged(String),
    RemoteEmbeddingsChanged(bool),
    EmbeddingModelChanged(String),
    ContextEmailsChanged(String),
    LanguageChanged(crate::i18n::Language),
    ThemeChanged(bool),
}

#[derive(Default)]
struct FolderTree<'a> {
    folder: Option<&'a IndexedFolder>,
    children: BTreeMap<&'a str, FolderTree<'a>>,
}

pub(crate) struct FolderTreeRow<'a> {
    pub(crate) depth: usize,
    pub(crate) name: &'a str,
    path: String,
    pub(crate) folder_paths: Vec<String>,
    message_count: u64,
    enabled: bool,
    has_children: bool,
    pub(crate) collapsed: bool,
}

pub(crate) fn folder_tree_rows<'a>(
    folders: &'a [IndexedFolder],
    collapsed_paths: &HashSet<String>,
) -> Vec<FolderTreeRow<'a>> {
    let mut root = FolderTree::default();
    for folder in folders {
        let mut node = &mut root;
        let mut found_segment = false;
        for segment in folder.path.split('/').filter(|segment| !segment.is_empty()) {
            found_segment = true;
            node = node.children.entry(segment).or_default();
        }
        if found_segment {
            node.folder = Some(folder);
        }
    }

    fn append_rows<'a>(
        node: &FolderTree<'a>,
        depth: usize,
        parent_path: &str,
        collapsed_paths: &HashSet<String>,
        rows: &mut Vec<FolderTreeRow<'a>>,
    ) {
        for (&name, child) in &node.children {
            let path = if parent_path.is_empty() {
                name.to_owned()
            } else {
                format!("{parent_path}/{name}")
            };
            let mut descendant_folders = Vec::new();
            collect_folders(child, &mut descendant_folders);
            let collapsed = collapsed_paths.contains(&path);
            rows.push(FolderTreeRow {
                depth,
                name,
                path: path.clone(),
                folder_paths: descendant_folders
                    .iter()
                    .map(|folder| folder.path.clone())
                    .collect(),
                message_count: descendant_folders
                    .iter()
                    .map(|folder| folder.message_count)
                    .sum(),
                enabled: descendant_folders.iter().all(|folder| folder.enabled),
                has_children: !child.children.is_empty(),
                collapsed,
            });
            if !collapsed {
                append_rows(child, depth + 1, &path, collapsed_paths, rows);
            }
        }
    }

    fn collect_folders<'a>(node: &FolderTree<'a>, folders: &mut Vec<&'a IndexedFolder>) {
        if let Some(folder) = node.folder {
            folders.push(folder);
        }
        for child in node.children.values() {
            collect_folders(child, folders);
        }
    }

    let mut rows = Vec::new();
    append_rows(&root, 0, "", collapsed_paths, &mut rows);
    rows
}

impl PstRagApp {
    pub(super) fn sidebar(&self) -> Element {
        let tr = self.config.language;
        let accent = accent_text();
        let muted_color = muted();
        let bright = bright_text();
        let on_prim = on_primary();
        let sb_style = sidebar_style();

        if !self.sidebar_open {
            let open_label = tr.btn_open();
            return rsx! {
                aside {
                    style: "width: 42px; height: 100%; flex-shrink: 0; {sb_style}",
                    Button {
                        variant: ButtonVariant::Ghost,
                        title: "{open_label}",
                        style: "width: 100%; height: 100%; padding: 14px 4px; display: flex; flex-direction: column; align-items: center; justify-content: flex-start; gap: 10px; border-radius: 0;",
                        onclick: move |_| SidebarMessage::ToggleSidebar.dispatch(),
                        svg {
                            width: "16",
                            height: "16",
                            view_box: "0 0 24 24",
                            fill: "none",
                            stroke: "{accent}",
                            stroke_width: "2",
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                            rect { x: "3", y: "3", width: "18", height: "18", rx: "2" }
                            line { x1: "9", y1: "3", x2: "9", y2: "21" }
                        }
                        span {
                            style: "writing-mode: vertical-rl; text-orientation: mixed; font-size: 10px; font-weight: 600; letter-spacing: 0.12em; color: {muted_color}; text-transform: uppercase;",
                            "MAIL"
                        }
                    }
                }
            };
        }

        let name = self
            .archive
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or_else(|| tr.no_archive_selected());
        let import_text = tr.import_prefix(name);
        let choose = small_button(
            tr.btn_choose_pst(),
            (!self.busy).then_some(SidebarMessage::ChooseArchive),
        );
        let index = primary_button(
            tr.btn_index_now(),
            (!self.busy && self.archive.is_some()).then_some(SidebarMessage::Index),
        );
        let storage = small_button(
            tr.btn_choose_directory(),
            (!self.busy).then_some(SidebarMessage::ChooseStorage),
        );
        let save = primary_button(
            tr.btn_save_ai_settings(),
            Some(SidebarMessage::SaveCredentials),
        );

        let all_selected = self.mail_scope == MailScope::AllMessages;
        let all_nav_class = if all_selected {
            "nav-item active"
        } else {
            "nav-item"
        };
        let all_messages_label = tr.all_messages();
        let total_count = self.count;

        let mut source_elements = Vec::new();
        for source in &self.sources {
            let source_key = source.key.clone();
            let source_key_select = source.key.clone();
            let is_source_selected =
                matches!(&self.mail_scope, MailScope::Source(k) if k == &source.key);
            let source_nav_class = if is_source_selected {
                "nav-item active"
            } else {
                "nav-item"
            };
            let source_enabled = source.enabled;
            let source_name = source.name.clone();
            let source_count = source.message_count;

            let collapsed_paths: HashSet<_> = self
                .collapsed_source_folders
                .iter()
                .filter(|(sk, _)| sk == &source.key)
                .map(|(_, path)| path.clone())
                .collect();

            let mut folder_elements = Vec::new();
            for tree_row in folder_tree_rows(&source.folders, &collapsed_paths) {
                let sk_check = source.key.clone();
                let sk_folder = source.key.clone();
                let folder_paths = tree_row.folder_paths;
                let row_path = tree_row.path.clone();
                let is_folder_selected = matches!(
                    &self.mail_scope,
                    MailScope::Folder { source_key: sk, folder_prefix: fp }
                    if sk == &source.key && fp == &row_path
                );
                let folder_nav_class = if is_folder_selected {
                    "nav-item active"
                } else {
                    "nav-item"
                };
                let indent_px = (tree_row.depth.min(5) as u16) * 10;
                let row_checked = if source.enabled && tree_row.enabled {
                    CheckboxState::Checked
                } else {
                    CheckboxState::Unchecked
                };
                let folder_name = tree_row.name.to_owned();
                let folder_count = tree_row.message_count;
                let has_children = tree_row.has_children;
                let sk_toggle = source.key.clone();
                let folder_path_toggle = tree_row.path;
                let chevron_rot = if tree_row.collapsed { "-90deg" } else { "0deg" };

                folder_elements.push(rsx! {
                    SidebarMenuSubItem {
                        style: "display: flex; flex-direction: row; align-items: center; gap: 6px; padding-left: {indent_px}px;",
                        Checkbox {
                            checked: row_checked,
                            on_checked_change: move |state: CheckboxState| {
                                SidebarMessage::FolderTreeEnabledChanged(
                                    sk_check.clone(),
                                    folder_paths.clone(),
                                    matches!(state, CheckboxState::Checked),
                                )
                                .dispatch();
                            }
                        }
                        Button {
                            variant: ButtonVariant::Ghost,
                            size: ButtonSize::Xs,
                            class: "{folder_nav_class}",
                            style: "flex: 1; min-width: 0; padding: 4px 8px; font-size: 11.5px; justify-content: space-between;",
                            onclick: move |_| {
                                SidebarMessage::SelectMailScope(MailScope::Folder {
                                    source_key: sk_folder.clone(),
                                    folder_prefix: row_path.clone(),
                                })
                                .dispatch();
                            },
                            span {
                                style: "overflow: hidden; text-overflow: ellipsis; white-space: nowrap;",
                                "{folder_name}"
                            }
                            span {
                                style: "font-size: 10.5px; color: {muted_color}; font-variant-numeric: tabular-nums;",
                                "{folder_count}"
                            }
                        }
                        if has_children {
                            Button {
                                variant: ButtonVariant::Ghost,
                                size: ButtonSize::IconXs,
                                style: "padding: 2px; width: 20px; height: 20px; flex-shrink: 0;",
                                onclick: move |_| {
                                    SidebarMessage::ToggleFolderTreeNode(
                                        sk_toggle.clone(),
                                        folder_path_toggle.clone(),
                                    )
                                    .dispatch();
                                },
                                svg {
                                    width: "12",
                                    height: "12",
                                    view_box: "0 0 24 24",
                                    fill: "none",
                                    stroke: "{muted_color}",
                                    stroke_width: "2",
                                    stroke_linecap: "round",
                                    stroke_linejoin: "round",
                                    style: "transition: transform 0.14s ease; transform: rotate({chevron_rot});",
                                    polyline { points: "6 9 12 15 18 9" }
                                }
                            }
                        }
                    }
                });
            }

            let remove_section = if self.pending_remove_source.as_deref() == Some(&source.key) {
                let confirm_msg = tr.confirm_remove_pst();
                let cancel_btn =
                    small_button(tr.btn_cancel(), Some(SidebarMessage::CancelRemoveSource));
                let confirm_lbl = tr.btn_remove();
                rsx! {
                    div {
                        style: "padding: 8px; border-radius: var(--radius-sm); background-color: var(--danger-subtle); border: 1px solid rgba(239, 68, 68, 0.3); display: flex; flex-direction: column; gap: 6px; margin-top: 4px;",
                        span { style: "font-size: 11px; color: #fca5a5;", "{confirm_msg}" }
                        div {
                            style: "display: flex; flex-direction: row; gap: 6px;",
                            {cancel_btn}
                            Button {
                                variant: ButtonVariant::Destructive,
                                size: ButtonSize::Xs,
                                style: "font-size: 11px; padding: 5px 10px;",
                                onclick: move |_| SidebarMessage::ConfirmRemoveSource.dispatch(),
                                "{confirm_lbl}"
                            }
                        }
                    }
                }
            } else {
                let remove_lbl = tr.btn_remove_index();
                let sk_rem = source.key.clone();
                let is_busy = self.busy;
                rsx! {
                    div {
                        style: "padding-left: 20px; margin-top: 2px;",
                        Button {
                            variant: ButtonVariant::Ghost,
                            size: ButtonSize::Xs,
                            disabled: is_busy,
                            style: "font-size: 11px; padding: 3px 8px; color: {muted_color};",
                            onclick: move |_| SidebarMessage::RemoveSourceRequested(sk_rem.clone()).dispatch(),
                            "{remove_lbl}"
                        }
                    }
                }
            };

            let source_checked = if source_enabled {
                CheckboxState::Checked
            } else {
                CheckboxState::Unchecked
            };

            source_elements.push(rsx! {
                SidebarMenuItem {
                    style: "padding: 4px 0px; display: flex; flex-direction: column; gap: 3px; border-bottom: 1px solid var(--border-subtle);",
                    div {
                        style: "display: flex; flex-direction: row; align-items: center; gap: 6px;",
                        Checkbox {
                            checked: source_checked,
                            on_checked_change: move |state: CheckboxState| {
                                SidebarMessage::SourceEnabledChanged(
                                    source_key.clone(),
                                    matches!(state, CheckboxState::Checked),
                                )
                                .dispatch();
                            }
                        }
                        div {
                            style: "position: relative; flex: 1; min-width: 0;",
                            Button {
                                variant: ButtonVariant::Ghost,
                                size: ButtonSize::Sm,
                                class: "{source_nav_class}",
                                style: "width: 100%; justify-content: flex-start; padding-right: 36px;",
                                onclick: move |_| {
                                    SidebarMessage::SelectMailScope(MailScope::Source(
                                        source_key_select.clone(),
                                    ))
                                    .dispatch();
                                },
                                span {
                                    style: "font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;",
                                    "{source_name}"
                                }
                            }
                            SidebarMenuBadge {
                                "{source_count}"
                            }
                        }
                    }
                    if !folder_elements.is_empty() {
                        SidebarMenuSub {
                            style: "margin: 0 0 0 10px; padding: 2px 0 2px 8px;",
                            for fe in folder_elements {
                                {fe}
                            }
                        }
                    }
                    {remove_section}
                }
            });
        }

        let no_pst_msg = tr.no_pst_indexed();
        let sources_empty = self.sources.is_empty();
        let mailboxes_card = card(rsx! {
            div {
                style: "display: flex; flex-direction: column; gap: 8px;",
                SidebarMenu {
                    SidebarMenuItem {
                        Button {
                            variant: ButtonVariant::Ghost,
                            size: ButtonSize::Sm,
                            class: "{all_nav_class}",
                            style: "width: 100%; justify-content: flex-start; padding-right: 36px;",
                            onclick: move |_| SidebarMessage::SelectMailScope(MailScope::AllMessages).dispatch(),
                            span { style: "font-weight: 600;", "{all_messages_label}" }
                        }
                        SidebarMenuBadge {
                            "{total_count}"
                        }
                    }
                }
                Separator {}
                div {
                    style: "padding: 8px; border-radius: var(--radius-sm); background-color: var(--bg-input); border: 1px dashed var(--border-strong); display: flex; flex-direction: column; gap: 8px;",
                    span {
                        style: "font-size: 11px; color: {muted_color}; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;",
                        "{import_text}"
                    }
                    div {
                        style: "display: flex; flex-direction: row; gap: 6px; flex-wrap: wrap;",
                        {choose}
                        {index}
                    }
                }
                if sources_empty {
                    span {
                        class: "form-hint",
                        style: "padding: 4px 2px;",
                        "{no_pst_msg}"
                    }
                } else {
                    SidebarMenu {
                        for se in source_elements {
                            {se}
                        }
                    }
                }
            }
        });

        let all_lbl_text = tr.btn_all();
        let unlabeled_lbl_text = tr.unlabeled_count(self.unlabeled_count);

        let mut filter_pressed = HashSet::new();
        match self.label_filter {
            LabelFilter::Any => {
                filter_pressed.insert(0usize);
            }
            LabelFilter::Unlabeled => {
                filter_pressed.insert(1usize);
            }
            LabelFilter::Labeled(_) => {}
        }

        let mut label_rows = Vec::new();
        for (def, count) in &self.labels_with_counts {
            let label_name = def.name.clone();
            let label_del = def.name.clone();
            let is_active = matches!(&self.label_filter, LabelFilter::Labeled(l) if l == &def.name);
            let nav_class = if is_active {
                "nav-item active"
            } else {
                "nav-item"
            };
            let badge = label_badge(&def.name, &def.color, Some(count.to_string()));
            let del_title = tr.btn_delete();
            label_rows.push(rsx! {
                SidebarMenuItem {
                    style: "display: flex; flex-direction: row; align-items: center; gap: 4px;",
                    Button {
                        variant: ButtonVariant::Ghost,
                        size: ButtonSize::Sm,
                        class: "{nav_class}",
                        style: "flex: 1; min-width: 0; padding: 4px 6px; justify-content: flex-start;",
                        onclick: move |_| {
                            SidebarMessage::SelectLabelFilter(LabelFilter::Labeled(
                                label_name.clone(),
                            ))
                            .dispatch();
                        },
                        {badge}
                    }
                    Button {
                        variant: ButtonVariant::Ghost,
                        size: ButtonSize::Xs,
                        title: "{del_title}",
                        style: "padding: 4px 6px; font-size: 11px; color: {muted_color};",
                        onclick: move |_| SidebarMessage::RemoveLabelDefinition(label_del.clone()).dispatch(),
                        "{del_title}"
                    }
                }
            });
        }

        let can_add_label = !self.new_label_name.trim().is_empty();
        let add_or_update_text = tr.add_or_update_label();
        let name_placeholder = tr.label_name_placeholder();
        let desc_placeholder = tr.label_desc_placeholder();
        let cur_label_name = self.new_label_name.clone();
        let cur_label_desc = self.new_label_description.clone();
        let save_label_btn = primary_button(
            tr.btn_save_label(),
            can_add_label.then_some(SidebarMessage::SaveLabelDefinition),
        );

        let labels_card = card(rsx! {
            div {
                style: "display: flex; flex-direction: column; gap: 8px;",
                ToggleGroup {
                    horizontal: true,
                    allow_multiple_pressed: false,
                    pressed: filter_pressed,
                    on_pressed_change: move |pressed: HashSet<usize>| {
                        if pressed.contains(&0) {
                            SidebarMessage::SelectLabelFilter(LabelFilter::Any).dispatch();
                        } else if pressed.contains(&1) {
                            SidebarMessage::SelectLabelFilter(LabelFilter::Unlabeled).dispatch();
                        }
                    },
                    style: "width: 100%; display: flex;",
                    ToggleItem {
                        index: 0usize,
                        style: "flex: 1; padding: 5px 8px; font-size: 11px;",
                        "{all_lbl_text}"
                    }
                    ToggleItem {
                        index: 1usize,
                        style: "flex: 1; padding: 5px 8px; font-size: 11px;",
                        "{unlabeled_lbl_text}"
                    }
                }
                if !label_rows.is_empty() {
                    SidebarMenu {
                        for lr in label_rows {
                            {lr}
                        }
                    }
                }
                Separator {}
                Label {
                    html_for: "new-label-name-input",
                    style: "font-size: 11px; font-weight: 500; color: var(--text-secondary);",
                    "{add_or_update_text}"
                }
                Input {
                    id: "new-label-name-input",
                    r#type: "text",
                    placeholder: "{name_placeholder}",
                    value: "{cur_label_name}",
                    style: "width: 100%; font-size: 12px;",
                    oninput: move |evt: FormEvent| SidebarMessage::NewLabelNameChanged(evt.value()).dispatch(),
                    onkeydown: move |evt: KeyboardEvent| {
                        if evt.key() != Key::Escape {
                            evt.stop_propagation();
                        }
                        if evt.key() == Key::Enter && can_add_label {
                            SidebarMessage::SaveLabelDefinition.dispatch();
                        }
                    }
                }
                Input {
                    r#type: "text",
                    placeholder: "{desc_placeholder}",
                    value: "{cur_label_desc}",
                    style: "width: 100%; font-size: 12px;",
                    oninput: move |evt: FormEvent| SidebarMessage::NewLabelDescriptionChanged(evt.value()).dispatch(),
                    onkeydown: move |evt: KeyboardEvent| {
                        if evt.key() != Key::Escape {
                            evt.stop_propagation();
                        }
                        if evt.key() == Key::Enter && can_add_label {
                            SidebarMessage::SaveLabelDefinition.dispatch();
                        }
                    }
                }
                {save_label_btn}
            }
        });

        let external_gguf = self.active_laya_gguf_path();
        let laya_status = match &external_gguf {
            Some(path) => tr.laya_ready(&path.display().to_string()),
            None => tr.laya_builtin_active().to_owned(),
        };
        let laya_placeholder = tr.laya_path_placeholder();
        let laya_path_val = self.config.laya_gguf_path.clone();
        let choose_gguf_btn = small_button(
            tr.btn_choose_gguf(),
            (!self.busy).then_some(SidebarMessage::ChooseLayaGguf),
        );
        let download_q8_btn = small_button(
            tr.btn_download_q8(),
            (!self.busy).then_some(SidebarMessage::DownloadLayaGguf),
        );
        let laya_exp = tr.laya_explanation();

        let laya_card = card(rsx! {
            div {
                style: "display: flex; flex-direction: column; gap: 8px;",
                div {
                    style: "display: flex; flex-direction: row; align-items: center; justify-content: space-between;",
                    span { style: "font-size: 12px; font-weight: 600; color: {bright};", "ggml-org/Laya-GGUF" }
                    Badge {
                        variant: BadgeVariant::Secondary,
                        if external_gguf.is_some() { "GGUF" } else { "Built-in" }
                    }
                }
                span { class: "form-hint", style: "word-break: break-all;", "{laya_status}" }
                Input {
                    r#type: "text",
                    placeholder: "{laya_placeholder}",
                    value: "{laya_path_val}",
                    style: "width: 100%; font-size: 12px;",
                    oninput: move |evt: FormEvent| SidebarMessage::LayaGgufPathChanged(evt.value()).dispatch(),
                    onkeydown: move |evt: KeyboardEvent| {
                        if evt.key() != Key::Escape {
                            evt.stop_propagation();
                        }
                    }
                }
                div {
                    style: "display: flex; flex-direction: row; gap: 6px; flex-wrap: wrap;",
                    {choose_gguf_btn}
                    {download_q8_btn}
                }
                span { class: "form-hint", "{laya_exp}" }
            }
        });

        let endpoint_lbl = tr.endpoint_label();
        let endpoint_val = self.config.endpoint.clone();
        let model_lbl = tr.model_label();
        let model_ph = tr.model_placeholder();
        let model_val = self.config.model.clone();
        let api_key_lbl = tr.api_key_label();
        let token_ph = tr.token_placeholder();
        let token_val = self.config.bearer_token.clone();
        let remote_emb_lbl = tr.remote_embeddings_checkbox();
        let remote_emb_val = self.config.use_remote_embeddings;
        let emb_model_lbl = tr.embedding_model_label();
        let emb_model_ph = tr.embedding_model_placeholder();
        let emb_model_val = self.config.embedding_model.clone();
        let ctx_emails_lbl = tr.context_emails_label();
        let ctx_emails_hint = tr.context_emails_hint();
        let ctx_emails_val = self.config.context_email_limit();
        let ctx_emails_min = crate::llm::MIN_CONTEXT_EMAILS;
        let ctx_emails_max = crate::llm::MAX_CONTEXT_EMAILS;
        let https_notice = tr.https_required_notice();

        let llm_card = card(rsx! {
            div {
                style: "display: flex; flex-direction: column; gap: 8px;",
                div {
                    style: "display: flex; flex-direction: column; gap: 3px;",
                    Label {
                        html_for: "llm-endpoint-input",
                        style: "font-size: 11px; font-weight: 500; color: var(--text-secondary);",
                        "{endpoint_lbl}"
                    }
                    Input {
                        id: "llm-endpoint-input",
                        r#type: "text",
                        placeholder: "https://...",
                        value: "{endpoint_val}",
                        style: "width: 100%; font-size: 12px;",
                        oninput: move |evt: FormEvent| SidebarMessage::EndpointChanged(evt.value()).dispatch(),
                        onkeydown: move |evt: KeyboardEvent| {
                            if evt.key() != Key::Escape {
                                evt.stop_propagation();
                            }
                        }
                    }
                }
                div {
                    style: "display: flex; flex-direction: column; gap: 3px;",
                    Label {
                        html_for: "llm-model-input",
                        style: "font-size: 11px; font-weight: 500; color: var(--text-secondary);",
                        "{model_lbl}"
                    }
                    Input {
                        id: "llm-model-input",
                        r#type: "text",
                        placeholder: "{model_ph}",
                        value: "{model_val}",
                        style: "width: 100%; font-size: 12px;",
                        oninput: move |evt: FormEvent| SidebarMessage::ModelChanged(evt.value()).dispatch(),
                        onkeydown: move |evt: KeyboardEvent| {
                            if evt.key() != Key::Escape {
                                evt.stop_propagation();
                            }
                        }
                    }
                }
                div {
                    style: "display: flex; flex-direction: column; gap: 3px;",
                    Label {
                        html_for: "llm-token-input",
                        style: "font-size: 11px; font-weight: 500; color: var(--text-secondary);",
                        "{api_key_lbl}"
                    }
                    Input {
                        id: "llm-token-input",
                        r#type: "password",
                        placeholder: "{token_ph}",
                        value: "{token_val}",
                        style: "width: 100%; font-size: 12px;",
                        oninput: move |evt: FormEvent| SidebarMessage::TokenChanged(evt.value()).dispatch(),
                        onkeydown: move |evt: KeyboardEvent| {
                            if evt.key() != Key::Escape {
                                evt.stop_propagation();
                            }
                        }
                    }
                }
                if crate::llm::EMBEDDINGS_ENABLED {
                    Separator {}
                    div {
                        style: "display: flex; flex-direction: row; align-items: center; justify-content: space-between; gap: 8px; padding: 2px 0;",
                        Label {
                            html_for: "remote-emb-switch",
                            style: "font-size: 11.5px; cursor: pointer;",
                            "{remote_emb_lbl}"
                        }
                        Switch {
                            checked: remote_emb_val,
                            on_checked_change: move |checked: bool| SidebarMessage::RemoteEmbeddingsChanged(checked).dispatch(),
                        }
                    }
                    div {
                        style: "display: flex; flex-direction: column; gap: 3px;",
                        Label {
                            html_for: "llm-emb-model-input",
                            style: "font-size: 11px; font-weight: 500; color: var(--text-secondary);",
                            "{emb_model_lbl}"
                        }
                        Input {
                            id: "llm-emb-model-input",
                            r#type: "text",
                            placeholder: "{emb_model_ph}",
                            value: "{emb_model_val}",
                            style: "width: 100%; font-size: 12px;",
                            oninput: move |evt: FormEvent| SidebarMessage::EmbeddingModelChanged(evt.value()).dispatch(),
                            onkeydown: move |evt: KeyboardEvent| {
                                if evt.key() != Key::Escape {
                                    evt.stop_propagation();
                                }
                            }
                        }
                    }
                }
                Separator {}
                div {
                    style: "display: flex; flex-direction: column; gap: 3px;",
                    Label {
                        html_for: "llm-context-emails-input",
                        style: "font-size: 11px; font-weight: 500; color: var(--text-secondary);",
                        "{ctx_emails_lbl}"
                    }
                    Input {
                        id: "llm-context-emails-input",
                        r#type: "number",
                        min: "{ctx_emails_min}",
                        max: "{ctx_emails_max}",
                        step: "1",
                        value: "{ctx_emails_val}",
                        style: "width: 100%; font-size: 12px;",
                        oninput: move |evt: FormEvent| SidebarMessage::ContextEmailsChanged(evt.value()).dispatch(),
                        onkeydown: move |evt: KeyboardEvent| {
                            if evt.key() != Key::Escape {
                                evt.stop_propagation();
                            }
                        }
                    }
                    span { class: "form-hint", "{ctx_emails_hint}" }
                }
                span { class: "form-hint", "{https_notice}" }
                {save}
            }
        });

        let storage_dir_str = self.storage_dir.display().to_string();
        let storage_notice = tr.storage_notice();
        let storage_card = card(rsx! {
            div {
                style: "display: flex; flex-direction: column; gap: 8px;",
                div {
                    style: "padding: 6px 8px; border-radius: var(--radius-xs); background-color: var(--bg-input); border: 1px solid var(--border-subtle); font-family: var(--font-mono); font-size: 10.5px; color: {bright}; word-break: break-all;",
                    "{storage_dir_str}"
                }
                span { class: "form-hint", "{storage_notice}" }
                {storage}
            }
        });

        let ai_sub_lbl = tr.section_ai();
        let laya_sub_lbl = tr.section_laya();
        let storage_sub_lbl = tr.section_storage();
        let settings_body = rsx! {
            div {
                style: "display: flex; flex-direction: column; gap: 10px;",
                div {
                    style: "display: flex; flex-direction: column; gap: 4px;",
                    span {
                        style: "font-size: 10px; font-weight: 700; letter-spacing: 0.05em; color: {muted_color}; text-transform: uppercase; padding: 0 2px;",
                        "{ai_sub_lbl}"
                    }
                    {llm_card}
                }
                div {
                    style: "display: flex; flex-direction: column; gap: 4px;",
                    span {
                        style: "font-size: 10px; font-weight: 700; letter-spacing: 0.05em; color: {muted_color}; text-transform: uppercase; padding: 0 2px;",
                        "{laya_sub_lbl}"
                    }
                    {laya_card}
                }
                div {
                    style: "display: flex; flex-direction: column; gap: 4px;",
                    span {
                        style: "font-size: 10px; font-weight: 700; letter-spacing: 0.05em; color: {muted_color}; text-transform: uppercase; padding: 0 2px;",
                        "{storage_sub_lbl}"
                    }
                    {storage_card}
                }
            }
        };

        let tile_style = accent_tile_style();
        let brand_sub = tr.brand_subtitle();
        let hide_sb_btn = small_button(tr.btn_hide(), Some(SidebarMessage::ToggleSidebar));

        let lang_lbl = tr.language_label();
        let is_fr = self.config.language == crate::i18n::Language::French;
        let mut lang_pressed = HashSet::new();
        lang_pressed.insert(if is_fr { 0usize } else { 1usize });

        let theme_lbl = tr.theme_label();
        let is_dark = self.config.dark_mode;
        let theme_dark_lbl = tr.theme_dark();
        let theme_light_lbl = tr.theme_light();

        let mut sections = Vec::new();
        for (section, label, body) in [
            (
                SidebarSection::Mailboxes,
                tr.section_mailboxes(),
                mailboxes_card,
            ),
            (SidebarSection::Labels, tr.section_labels(), labels_card),
            (
                SidebarSection::Settings,
                tr.section_settings(),
                settings_body,
            ),
        ] {
            let collapsed = self.collapsed_sections[section.index()];
            let header_el =
                collapsible_section_label(label, collapsed, SidebarMessage::ToggleSection(section));
            sections.push(rsx! {
                SidebarGroup {
                    style: "padding: 0;",
                    Collapsible {
                        open: !collapsed,
                        on_open_change: move |_| SidebarMessage::ToggleSection(section).dispatch(),
                        div {
                            style: "display: flex; flex-direction: column; gap: 6px;",
                            {header_el}
                            CollapsibleContent {
                                SidebarGroupContent {
                                    {body}
                                }
                            }
                        }
                    }
                }
            });
        }

        rsx! {
            aside {
                style: "width: 285px; height: 100%; flex-shrink: 0; display: flex; flex-direction: column; overflow: hidden; {sb_style}",
                SidebarHeader {
                    style: "padding: 14px 14px 8px 14px; gap: 10px;",
                    div {
                        style: "display: flex; flex-direction: row; align-items: center; gap: 10px;",
                        div {
                            style: "width: 34px; height: 34px; display: flex; align-items: center; justify-content: center; flex-shrink: 0; {tile_style}",
                            svg {
                                width: "22",
                                height: "22",
                                view_box: "0 0 128 128",
                                fill: "none",
                                path {
                                    d: "M20 50L64 20L108 50H20Z",
                                    fill: "#93c5fd",
                                    stroke: "{on_prim}",
                                    stroke_width: "6",
                                    stroke_linejoin: "round",
                                }
                                rect {
                                    x: "20",
                                    y: "50",
                                    width: "88",
                                    height: "56",
                                    rx: "8",
                                    fill: "rgba(30, 58, 138, 0.55)",
                                    stroke: "{on_prim}",
                                    stroke_width: "6",
                                    stroke_linejoin: "round",
                                }
                                circle {
                                    cx: "64",
                                    cy: "78",
                                    r: "15",
                                    stroke: "#93c5fd",
                                    stroke_width: "6",
                                }
                                path {
                                    d: "M57 78L62 83L72 73",
                                    stroke: "{on_prim}",
                                    stroke_width: "6",
                                    stroke_linecap: "round",
                                    stroke_linejoin: "round",
                                }
                                path {
                                    d: "M102 22L104.5 29.5L112 32L104.5 34.5L102 42L99.5 34.5L92 32L99.5 29.5L102 22Z",
                                    fill: "{on_prim}",
                                }
                            }
                        }
                        div {
                            style: "flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px;",
                            span { style: "font-size: 14px; color: {bright}; font-weight: 700; letter-spacing: -0.01em;", "PoSTier" }
                            span { style: "font-size: 11px; color: {muted_color}; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;", "{brand_sub}" }
                        }
                        {hide_sb_btn}
                    }
                    div {
                        style: "display: flex; flex-direction: row; align-items: center; justify-content: space-between; gap: 8px;",
                        span { style: "font-size: 11.5px; color: {muted_color}; font-weight: 500;", "{lang_lbl}" }
                        ToggleGroup {
                            horizontal: true,
                            allow_multiple_pressed: false,
                            pressed: lang_pressed,
                            on_pressed_change: move |pressed: HashSet<usize>| {
                                if pressed.contains(&0) {
                                    SidebarMessage::LanguageChanged(crate::i18n::Language::French).dispatch();
                                } else if pressed.contains(&1) {
                                    SidebarMessage::LanguageChanged(crate::i18n::Language::English).dispatch();
                                }
                            },
                            ToggleItem {
                                index: 0usize,
                                style: "padding: 4px 10px; font-size: 11px;",
                                "Français"
                            }
                            ToggleItem {
                                index: 1usize,
                                style: "padding: 4px 10px; font-size: 11px;",
                                "English"
                            }
                        }
                    }
                    div {
                        style: "display: flex; flex-direction: row; align-items: center; justify-content: space-between; gap: 8px;",
                        span { style: "font-size: 11.5px; color: {muted_color}; font-weight: 500;", "{theme_lbl}" }
                        div {
                            style: "display: flex; flex-direction: row; align-items: center; gap: 6px;",
                            svg {
                                width: "13",
                                height: "13",
                                view_box: "0 0 24 24",
                                fill: "none",
                                stroke: if !is_dark { "var(--text-accent)" } else { "var(--text-muted)" },
                                stroke_width: "2",
                                stroke_linecap: "round",
                                stroke_linejoin: "round",
                                circle { cx: "12", cy: "12", r: "5" }
                                line { x1: "12", y1: "1", x2: "12", y2: "3" }
                                line { x1: "12", y1: "21", x2: "12", y2: "23" }
                                line { x1: "4.22", y1: "4.22", x2: "5.64", y2: "5.64" }
                                line { x1: "18.36", y1: "18.36", x2: "19.78", y2: "19.78" }
                                line { x1: "1", y1: "12", x2: "3", y2: "12" }
                                line { x1: "21", y1: "12", x2: "23", y2: "12" }
                                line { x1: "4.22", y1: "19.78", x2: "5.64", y2: "18.36" }
                                line { x1: "18.36", y1: "5.64", x2: "19.78", y2: "4.22" }
                            }
                            span {
                                style: if !is_dark { "font-size: 11px; color: var(--text-accent); font-weight: 600;" } else { "font-size: 11px; color: var(--text-muted); font-weight: 400;" },
                                "{theme_light_lbl}"
                            }
                            Switch {
                                checked: is_dark,
                                on_checked_change: move |dark: bool| {
                                    SidebarMessage::ThemeChanged(dark).dispatch();
                                },
                            }
                            span {
                                style: if is_dark { "font-size: 11px; color: var(--text-accent); font-weight: 600;" } else { "font-size: 11px; color: var(--text-muted); font-weight: 400;" },
                                "{theme_dark_lbl}"
                            }
                            svg {
                                width: "13",
                                height: "13",
                                view_box: "0 0 24 24",
                                fill: "none",
                                stroke: if is_dark { "var(--text-accent)" } else { "var(--text-muted)" },
                                stroke_width: "2",
                                stroke_linecap: "round",
                                stroke_linejoin: "round",
                                path { d: "M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" }
                            }
                        }
                    }
                }
                Separator {}
                ScrollArea {
                    style: "flex: 1; min-height: 0; width: 100%;",
                    SidebarContent {
                        style: "padding: 10px 14px 14px 14px; gap: 12px;",
                        for sec in sections {
                            {sec}
                        }
                    }
                }
            }
        }
    }
}
