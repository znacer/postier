use std::collections::{BTreeMap, HashSet};

use dioxus::prelude::*;

use super::{DispatchMessage, PstRagApp, SidebarSection, theme::*};
use crate::model::{IndexedFolder, LabelFilter, MailScope};

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
    LanguageChanged(crate::i18n::Language),
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
                div {
                    style: "width: 36px; height: 100%; flex-shrink: 0; {sb_style}",
                    button {
                        class: "btn-text",
                        style: "width: 100%; height: 100%; padding: 16px 4px; display: flex; flex-direction: column; align-items: center; justify-content: flex-start; gap: 2px;",
                        onclick: move |_| SidebarMessage::ToggleSidebar.dispatch(),
                        span { style: "font-size: 10px; color: {accent};", "{open_label}" }
                        div { style: "height: 8px;" }
                        span { style: "font-size: 10px; color: {muted_color};", "M" }
                        span { style: "font-size: 10px; color: {muted_color};", "A" }
                        span { style: "font-size: 10px; color: {muted_color};", "I" }
                        span { style: "font-size: 10px; color: {muted_color};", "L" }
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
        let index = small_button(
            tr.btn_index_now(),
            (!self.busy && self.archive.is_some()).then_some(SidebarMessage::Index),
        );
        let storage = small_button(
            tr.btn_choose_directory(),
            (!self.busy).then_some(SidebarMessage::ChooseStorage),
        );
        let save = small_button(
            tr.btn_save_ai_settings(),
            Some(SidebarMessage::SaveCredentials),
        );

        let all_selected = self.mail_scope == MailScope::AllMessages;
        let all_btn_class = if all_selected {
            "btn-primary"
        } else {
            "btn-secondary"
        };
        let all_messages_label = tr.all_messages();
        let total_count = self.count;

        let mut source_elements = Vec::new();
        for source in &self.sources {
            let source_key = source.key.clone();
            let source_key_select = source.key.clone();
            let is_source_selected =
                matches!(&self.mail_scope, MailScope::Source(k) if k == &source.key);
            let source_btn_class = if is_source_selected {
                "btn-primary"
            } else {
                "btn-text"
            };
            let source_enabled = source.enabled;
            let source_title = format!("{} ({})", source.name, source.message_count);

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
                let folder_btn_class = if is_folder_selected {
                    "btn-primary"
                } else {
                    "btn-text"
                };
                let indent_px = ((tree_row.depth + 1).min(6) as u16) * 12;
                let row_checked = source.enabled && tree_row.enabled;
                let folder_label = format!("{} ({})", tree_row.name, tree_row.message_count);
                let has_children = tree_row.has_children;
                let sk_toggle = source.key.clone();
                let folder_path_toggle = tree_row.path;
                let toggle_label = if tree_row.collapsed {
                    tr.btn_show()
                } else {
                    tr.btn_hide()
                };

                folder_elements.push(rsx! {
                    div {
                        style: "display: flex; flex-direction: row; align-items: center; gap: 4px; padding-left: {indent_px}px;",
                        input {
                            r#type: "checkbox",
                            checked: row_checked,
                            onchange: move |evt| {
                                SidebarMessage::FolderTreeEnabledChanged(
                                    sk_check.clone(),
                                    folder_paths.clone(),
                                    evt.checked(),
                                )
                                .dispatch();
                            }
                        }
                        button {
                            class: "{folder_btn_class}",
                            style: "flex: 1; padding: 2px 5px; font-size: 11px; justify-content: flex-start;",
                            onclick: move |_| {
                                SidebarMessage::SelectMailScope(MailScope::Folder {
                                    source_key: sk_folder.clone(),
                                    folder_prefix: row_path.clone(),
                                })
                                .dispatch();
                            },
                            "{folder_label}"
                        }
                        if has_children {
                            button {
                                class: "btn-text",
                                style: "padding: 2px 4px; font-size: 10px; color: {muted_color};",
                                onclick: move |_| {
                                    SidebarMessage::ToggleFolderTreeNode(
                                        sk_toggle.clone(),
                                        folder_path_toggle.clone(),
                                    )
                                    .dispatch();
                                },
                                "{toggle_label}"
                            }
                        }
                    }
                });
            }

            let remove_section = if self.pending_remove_source.as_deref() == Some(&source.key) {
                let confirm_msg = tr.confirm_remove_pst();
                let cancel_btn =
                    small_button(tr.btn_cancel(), Some(SidebarMessage::CancelRemoveSource));
                let confirm_btn =
                    small_button(tr.btn_remove(), Some(SidebarMessage::ConfirmRemoveSource));
                rsx! {
                    div {
                        style: "display: flex; flex-direction: column; gap: 4px;",
                        span { style: "font-size: 10px;", "{confirm_msg}" }
                        div {
                            style: "display: flex; flex-direction: row; gap: 6px;",
                            {cancel_btn}
                            {confirm_btn}
                        }
                    }
                }
            } else {
                small_button(
                    tr.btn_remove_index(),
                    (!self.busy)
                        .then_some(SidebarMessage::RemoveSourceRequested(source.key.clone())),
                )
            };

            source_elements.push(rsx! {
                div {
                    style: "padding: 4px 0px; display: flex; flex-direction: column; gap: 3px;",
                    div {
                        style: "display: flex; flex-direction: row; align-items: center; gap: 4px;",
                        input {
                            r#type: "checkbox",
                            checked: source_enabled,
                            onchange: move |evt| {
                                SidebarMessage::SourceEnabledChanged(
                                    source_key.clone(),
                                    evt.checked(),
                                )
                                .dispatch();
                            }
                        }
                        button {
                            class: "{source_btn_class}",
                            style: "flex: 1; padding: 3px 6px; font-size: 12px; justify-content: flex-start;",
                            onclick: move |_| {
                                SidebarMessage::SelectMailScope(MailScope::Source(
                                    source_key_select.clone(),
                                ))
                                .dispatch();
                            },
                            "{source_title}"
                        }
                    }
                    for fe in folder_elements {
                        {fe}
                    }
                    {remove_section}
                }
            });
        }

        let no_pst_msg = tr.no_pst_indexed();
        let sources_empty = self.sources.is_empty();
        let mailboxes_card = card(rsx! {
            div {
                style: "display: flex; flex-direction: column; gap: 7px;",
                button {
                    class: "{all_btn_class}",
                    style: "width: 100%; padding: 6px 10px; display: flex; flex-direction: row; align-items: center; justify-content: space-between;",
                    onclick: move |_| SidebarMessage::SelectMailScope(MailScope::AllMessages).dispatch(),
                    span { style: "font-size: 12px;", "{all_messages_label}" }
                    span { style: "font-size: 11px; color: {accent};", "{total_count}" }
                }
                hr {}
                span { style: "font-size: 11px; color: {muted_color};", "{import_text}" }
                div {
                    style: "display: flex; flex-direction: row; gap: 6px; flex-wrap: wrap;",
                    {choose}
                    {index}
                }
                hr {}
                if sources_empty {
                    span { style: "font-size: 11px; color: {muted_color};", "{no_pst_msg}" }
                }
                for se in source_elements {
                    {se}
                }
            }
        });

        let btn_all = small_button(
            tr.btn_all(),
            Some(SidebarMessage::SelectLabelFilter(LabelFilter::Any)),
        );
        let btn_unlabeled = small_button(
            tr.unlabeled_count(self.unlabeled_count),
            Some(SidebarMessage::SelectLabelFilter(LabelFilter::Unlabeled)),
        );

        let mut label_rows = Vec::new();
        for (def, count) in &self.labels_with_counts {
            let label_name = def.name.clone();
            let label_del = def.name.clone();
            let is_active = matches!(&self.label_filter, LabelFilter::Labeled(l) if l == &def.name);
            let btn_class = if is_active { "btn-primary" } else { "btn-text" };
            let badge = label_badge(&def.name, &def.color, Some(count.to_string()));
            let del_btn = small_button(
                tr.btn_delete(),
                Some(SidebarMessage::RemoveLabelDefinition(label_del)),
            );
            label_rows.push(rsx! {
                div {
                    style: "display: flex; flex-direction: row; align-items: center; gap: 4px;",
                    button {
                        class: "{btn_class}",
                        style: "flex: 1; padding: 3px 6px; justify-content: flex-start;",
                        onclick: move |_| {
                            SidebarMessage::SelectLabelFilter(LabelFilter::Labeled(
                                label_name.clone(),
                            ))
                            .dispatch();
                        },
                        {badge}
                    }
                    {del_btn}
                }
            });
        }

        let can_add_label = !self.new_label_name.trim().is_empty();
        let add_or_update_text = tr.add_or_update_label();
        let name_placeholder = tr.label_name_placeholder();
        let desc_placeholder = tr.label_desc_placeholder();
        let cur_label_name = self.new_label_name.clone();
        let cur_label_desc = self.new_label_description.clone();
        let save_label_btn = small_button(
            tr.btn_save_label(),
            can_add_label.then_some(SidebarMessage::SaveLabelDefinition),
        );

        let labels_card = card(rsx! {
            div {
                style: "display: flex; flex-direction: column; gap: 6px;",
                div {
                    style: "display: flex; flex-direction: row; gap: 6px; flex-wrap: wrap;",
                    {btn_all}
                    {btn_unlabeled}
                }
                for lr in label_rows {
                    {lr}
                }
                hr {}
                span { style: "font-size: 11px; color: {muted_color};", "{add_or_update_text}" }
                input {
                    r#type: "text",
                    placeholder: "{name_placeholder}",
                    value: "{cur_label_name}",
                    style: "font-size: 11px; padding: 6px; width: 100%;",
                    oninput: move |evt| SidebarMessage::NewLabelNameChanged(evt.value()).dispatch(),
                    onkeydown: move |evt| {
                        if evt.key() != Key::Escape {
                            evt.stop_propagation();
                        }
                        if evt.key() == Key::Enter && can_add_label {
                            SidebarMessage::SaveLabelDefinition.dispatch();
                        }
                    }
                }
                input {
                    r#type: "text",
                    placeholder: "{desc_placeholder}",
                    value: "{cur_label_desc}",
                    style: "font-size: 11px; padding: 6px; width: 100%;",
                    oninput: move |evt| SidebarMessage::NewLabelDescriptionChanged(evt.value()).dispatch(),
                    onkeydown: move |evt| {
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
                style: "display: flex; flex-direction: column; gap: 6px;",
                span { style: "font-size: 12px; color: {bright};", "ggml-org/Laya-GGUF" }
                span { style: "font-size: 10px; color: {muted_color}; word-break: break-all;", "{laya_status}" }
                input {
                    r#type: "text",
                    placeholder: "{laya_placeholder}",
                    value: "{laya_path_val}",
                    style: "font-size: 11px; padding: 6px; width: 100%;",
                    oninput: move |evt| SidebarMessage::LayaGgufPathChanged(evt.value()).dispatch(),
                    onkeydown: move |evt| {
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
                span { style: "font-size: 10px; color: {muted_color};", "{laya_exp}" }
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
        let https_notice = tr.https_required_notice();

        let llm_card = card(rsx! {
            div {
                style: "display: flex; flex-direction: column; gap: 6px;",
                span { style: "font-size: 11px;", "{endpoint_lbl}" }
                input {
                    r#type: "text",
                    placeholder: "https://...",
                    value: "{endpoint_val}",
                    style: "font-size: 11px; padding: 6px; width: 100%;",
                    oninput: move |evt| SidebarMessage::EndpointChanged(evt.value()).dispatch(),
                    onkeydown: move |evt| {
                        if evt.key() != Key::Escape {
                            evt.stop_propagation();
                        }
                    }
                }
                span { style: "font-size: 11px;", "{model_lbl}" }
                input {
                    r#type: "text",
                    placeholder: "{model_ph}",
                    value: "{model_val}",
                    style: "font-size: 11px; padding: 6px; width: 100%;",
                    oninput: move |evt| SidebarMessage::ModelChanged(evt.value()).dispatch(),
                    onkeydown: move |evt| {
                        if evt.key() != Key::Escape {
                            evt.stop_propagation();
                        }
                    }
                }
                span { style: "font-size: 11px;", "{api_key_lbl}" }
                input {
                    r#type: "password",
                    placeholder: "{token_ph}",
                    value: "{token_val}",
                    style: "font-size: 11px; padding: 6px; width: 100%;",
                    oninput: move |evt| SidebarMessage::TokenChanged(evt.value()).dispatch(),
                    onkeydown: move |evt| {
                        if evt.key() != Key::Escape {
                            evt.stop_propagation();
                        }
                    }
                }
                hr {}
                label {
                    style: "display: flex; flex-direction: row; align-items: center; gap: 6px; font-size: 11px; cursor: pointer;",
                    input {
                        r#type: "checkbox",
                        checked: remote_emb_val,
                        onchange: move |evt| SidebarMessage::RemoteEmbeddingsChanged(evt.checked()).dispatch()
                    }
                    span { "{remote_emb_lbl}" }
                }
                span { style: "font-size: 11px;", "{emb_model_lbl}" }
                input {
                    r#type: "text",
                    placeholder: "{emb_model_ph}",
                    value: "{emb_model_val}",
                    style: "font-size: 11px; padding: 6px; width: 100%;",
                    oninput: move |evt| SidebarMessage::EmbeddingModelChanged(evt.value()).dispatch(),
                    onkeydown: move |evt| {
                        if evt.key() != Key::Escape {
                            evt.stop_propagation();
                        }
                    }
                }
                span { style: "font-size: 10px; color: {muted_color};", "{https_notice}" }
                {save}
            }
        });

        let storage_dir_str = self.storage_dir.display().to_string();
        let storage_notice = tr.storage_notice();
        let storage_card = card(rsx! {
            div {
                style: "display: flex; flex-direction: column; gap: 6px;",
                span { style: "font-size: 10px; word-break: break-all;", "{storage_dir_str}" }
                span { style: "font-size: 10px; color: {muted_color};", "{storage_notice}" }
                {storage}
            }
        });

        let tile_style = accent_tile_style();
        let brand_sub = tr.brand_subtitle();
        let hide_sb_btn = small_button(tr.btn_hide(), Some(SidebarMessage::ToggleSidebar));

        let lang_lbl = tr.language_label();
        let fr_class = if self.config.language == crate::i18n::Language::French {
            "btn-primary"
        } else {
            "btn-secondary"
        };
        let en_class = if self.config.language == crate::i18n::Language::English {
            "btn-primary"
        } else {
            "btn-secondary"
        };
        let fr_btn = constrain_button(rsx! {
            button {
                class: "{fr_class}",
                style: "font-size: 10px; padding: 2px 8px;",
                onclick: move |_| SidebarMessage::LanguageChanged(crate::i18n::Language::French).dispatch(),
                "Français"
            }
        });
        let en_btn = constrain_button(rsx! {
            button {
                class: "{en_class}",
                style: "font-size: 10px; padding: 2px 8px;",
                onclick: move |_| SidebarMessage::LanguageChanged(crate::i18n::Language::English).dispatch(),
                "English"
            }
        });

        let mut sections = Vec::new();
        for (section, label, body) in [
            (
                SidebarSection::Mailboxes,
                tr.section_mailboxes(),
                mailboxes_card,
            ),
            (SidebarSection::Labels, tr.section_labels(), labels_card),
            (SidebarSection::LayaModel, tr.section_laya(), laya_card),
            (SidebarSection::AiModels, tr.section_ai(), llm_card),
            (SidebarSection::Storage, tr.section_storage(), storage_card),
        ] {
            let collapsed = self.collapsed_sections[section.index()];
            let header_el =
                collapsible_section_label(label, collapsed, SidebarMessage::ToggleSection(section));
            sections.push(rsx! {
                {header_el}
                if !collapsed {
                    {body}
                }
            });
        }

        rsx! {
            div {
                style: "width: 285px; height: 100%; flex-shrink: 0; overflow-y: auto; {sb_style}",
                div {
                    style: "padding: 14px; display: flex; flex-direction: column; gap: 10px;",
                    div {
                        style: "display: flex; flex-direction: row; align-items: center; gap: 10px;",
                        div {
                            style: "width: 34px; height: 34px; display: flex; align-items: center; justify-content: center; flex-shrink: 0; {tile_style}",
                            span { style: "font-size: 16px; color: {on_prim}; font-weight: 600;", "P" }
                        }
                        div {
                            style: "flex: 1; display: flex; flex-direction: column; gap: 1px;",
                            span { style: "font-size: 14px; color: {bright}; font-weight: 600;", "PSTbeetle" }
                            span { style: "font-size: 10px; color: {muted_color};", "{brand_sub}" }
                        }
                        {hide_sb_btn}
                    }
                    div {
                        style: "display: flex; flex-direction: row; align-items: center; gap: 6px;",
                        span { style: "font-size: 11px; color: {muted_color}; flex: 1;", "{lang_lbl}" }
                        {fr_btn}
                        {en_btn}
                    }
                    hr {}
                    for sec in sections {
                        {sec}
                    }
                }
            }
        }
    }
}
