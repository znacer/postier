use iced::{
    Alignment, Element, Fill,
    widget::{
        Space, button, checkbox, column, container, horizontal_rule, row, scrollable, text,
        text_input,
    },
};
use std::collections::{BTreeMap, HashSet};

use super::{PstRagApp, SidebarSection, theme::*};
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
    pub(super) fn sidebar(&self) -> Element<'_, SidebarMessage> {
        if !self.sidebar_open {
            let expand_rail = button(
                column![
                    text("Open").size(10).color(accent_text()),
                    Space::new(0, 8),
                    text("M").size(10).color(muted()),
                    text("A").size(10).color(muted()),
                    text("I").size(10).color(muted()),
                    text("L").size(10).color(muted()),
                ]
                .spacing(2)
                .align_x(Alignment::Center),
            )
            .width(Fill)
            .height(Fill)
            .padding([16, 4])
            .style(button::text)
            .on_press(SidebarMessage::ToggleSidebar);

            return container(expand_rail)
                .width(36)
                .height(Fill)
                .style(sidebar_style)
                .into();
        }

        let name = self
            .archive
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("No archive selected");
        let choose = small_button(
            "Choose PST",
            (!self.busy).then_some(SidebarMessage::ChooseArchive),
        );
        let index = small_button(
            "Index now",
            (!self.busy && self.archive.is_some()).then_some(SidebarMessage::Index),
        );
        let storage = small_button(
            "Choose directory",
            (!self.busy).then_some(SidebarMessage::ChooseStorage),
        );
        let save = small_button("Save AI settings", Some(SidebarMessage::SaveCredentials));

        let all_selected = self.mail_scope == MailScope::AllMessages;
        let all_mail_btn = button(
            row![
                text("All Messages").size(12).width(Fill),
                text(self.count).size(11).color(accent_text())
            ]
            .align_y(Alignment::Center),
        )
        .width(Fill)
        .padding([6, 10])
        .style(if all_selected {
            button::primary
        } else {
            button::secondary
        })
        .on_press(SidebarMessage::SelectMailScope(MailScope::AllMessages));

        let mut mailbox_col = column![
            all_mail_btn,
            horizontal_rule(1),
            text(format!("Import: {name}")).size(11).color(muted()),
            row![choose, index].spacing(6),
            horizontal_rule(1),
        ]
        .spacing(7);

        if self.sources.is_empty() {
            mailbox_col =
                mailbox_col.push(text("No PST files indexed yet").size(11).color(muted()));
        }
        for source in &self.sources {
            let source_key = source.key.clone();
            let source_key_select = source.key.clone();
            let is_source_selected =
                matches!(&self.mail_scope, MailScope::Source(k) if k == &source.key);
            let mut source_view = column![
                row![
                    checkbox("", source.enabled)
                        .size(14)
                        .on_toggle(move |enabled| {
                            SidebarMessage::SourceEnabledChanged(source_key.clone(), enabled)
                        }),
                    button(text(format!("{} ({})", source.name, source.message_count)).size(12))
                        .padding([3, 6])
                        .width(Fill)
                        .style(if is_source_selected {
                            button::primary
                        } else {
                            button::text
                        })
                        .on_press(SidebarMessage::SelectMailScope(MailScope::Source(
                            source_key_select
                        ))),
                ]
                .spacing(4)
                .align_y(Alignment::Center)
            ]
            .spacing(3);

            let collapsed_paths: HashSet<_> = self
                .collapsed_source_folders
                .iter()
                .filter(|(sk, _)| sk == &source.key)
                .map(|(_, path)| path.clone())
                .collect();
            for tree_row in folder_tree_rows(&source.folders, &collapsed_paths) {
                let source_key = source.key.clone();
                let source_key_folder = source.key.clone();
                let folder_paths = tree_row.folder_paths;
                let row_path = tree_row.path.clone();
                let is_folder_selected = matches!(
                    &self.mail_scope,
                    MailScope::Folder { source_key: sk, folder_prefix: fp }
                    if sk == &source.key && fp == &row_path
                );

                let indent_px = ((tree_row.depth + 1).min(6) as u16) * 12;
                let check = checkbox("", source.enabled && tree_row.enabled)
                    .size(13)
                    .on_toggle(move |enabled| {
                        SidebarMessage::FolderTreeEnabledChanged(
                            source_key.clone(),
                            folder_paths.clone(),
                            enabled,
                        )
                    });
                let folder_btn = button(
                    text(format!("{} ({})", tree_row.name, tree_row.message_count)).size(11),
                )
                .padding([2, 5])
                .width(Fill)
                .style(if is_folder_selected {
                    button::primary
                } else {
                    button::text
                })
                .on_press(SidebarMessage::SelectMailScope(MailScope::Folder {
                    source_key: source_key_folder,
                    folder_prefix: row_path,
                }));

                if tree_row.has_children {
                    let source_key = source.key.clone();
                    let folder_path = tree_row.path;
                    let toggle_label = if tree_row.collapsed { "Show" } else { "Hide" };
                    source_view = source_view.push(
                        row![
                            Space::new(indent_px, 0),
                            check,
                            folder_btn,
                            button(text(toggle_label).size(10).color(muted()))
                                .padding([2, 4])
                                .style(button::text)
                                .on_press(SidebarMessage::ToggleFolderTreeNode(
                                    source_key,
                                    folder_path
                                )),
                        ]
                        .spacing(4)
                        .align_y(Alignment::Center),
                    );
                } else {
                    source_view = source_view.push(
                        row![Space::new(indent_px, 0), check, folder_btn]
                            .spacing(4)
                            .align_y(Alignment::Center),
                    );
                }
            }
            if self.pending_remove_source.as_deref() == Some(&source.key) {
                source_view = source_view.push(
                    column![
                        text("Remove this PST's messages and embeddings?").size(10),
                        row![
                            small_button("Cancel", Some(SidebarMessage::CancelRemoveSource)),
                            small_button("Remove", Some(SidebarMessage::ConfirmRemoveSource))
                        ]
                        .spacing(6)
                    ]
                    .spacing(4),
                );
            } else {
                source_view = source_view.push(small_button(
                    "Remove index",
                    (!self.busy)
                        .then_some(SidebarMessage::RemoveSourceRequested(source.key.clone())),
                ));
            }
            mailbox_col = mailbox_col.push(container(source_view).padding([4, 0]));
        }
        let mailboxes_card = card(mailbox_col);

        let mut labels_col = column![
            row![
                small_button(
                    "All",
                    Some(SidebarMessage::SelectLabelFilter(LabelFilter::Any))
                ),
                small_button(
                    format!("Unlabeled ({})", self.unlabeled_count),
                    Some(SidebarMessage::SelectLabelFilter(LabelFilter::Unlabeled))
                ),
            ]
            .spacing(6)
        ]
        .spacing(6);

        for (def, count) in &self.labels_with_counts {
            let label_name = def.name.clone();
            let label_del = def.name.clone();
            let is_active = matches!(&self.label_filter, LabelFilter::Labeled(l) if l == &def.name);
            labels_col = labels_col.push(
                row![
                    button(
                        row![
                            label_badge(&def.name, &def.color, Some(count.to_string())),
                            Space::new(Fill, 0),
                        ]
                        .align_y(Alignment::Center),
                    )
                    .width(Fill)
                    .padding([3, 6])
                    .style(if is_active {
                        button::primary
                    } else {
                        button::text
                    })
                    .on_press(SidebarMessage::SelectLabelFilter(LabelFilter::Labeled(
                        label_name
                    ))),
                    small_button(
                        "Delete",
                        Some(SidebarMessage::RemoveLabelDefinition(label_del))
                    ),
                ]
                .spacing(4)
                .align_y(Alignment::Center),
            );
        }

        let can_add_label = !self.new_label_name.trim().is_empty();
        labels_col = labels_col
            .push(horizontal_rule(1))
            .push(text("Add or update label").size(11).color(muted()))
            .push(
                text_input("Label name (e.g. Billing)", &self.new_label_name)
                    .on_input(SidebarMessage::NewLabelNameChanged)
                    .on_submit_maybe(can_add_label.then_some(SidebarMessage::SaveLabelDefinition))
                    .size(11)
                    .padding(6),
            )
            .push(
                text_input(
                    "Criteria description for Laya or LLM Judge",
                    &self.new_label_description,
                )
                .on_input(SidebarMessage::NewLabelDescriptionChanged)
                .on_submit_maybe(can_add_label.then_some(SidebarMessage::SaveLabelDefinition))
                .size(11)
                .padding(6),
            )
            .push(small_button(
                "Save Label",
                can_add_label.then_some(SidebarMessage::SaveLabelDefinition),
            ));
        let labels_card = card(labels_col);

        let external_gguf = self.active_laya_gguf_path();
        let laya_status = match &external_gguf {
            Some(path) => format!("External GGUF ready: {}", path.display()),
            None => "Active: Built-in packaged Laya-GGUF decision model".to_owned(),
        };
        let laya_card = card(
            column![
                text("ggml-org/Laya-GGUF").size(12).color(bright_text()),
                text(laya_status).size(10).color(muted()),
                text_input(
                    "Optional path to Laya-Q8_0.gguf",
                    &self.config.laya_gguf_path
                )
                .on_input(SidebarMessage::LayaGgufPathChanged)
                .size(11)
                .padding(6),
                row![
                    small_button(
                        "Choose GGUF...",
                        (!self.busy).then_some(SidebarMessage::ChooseLayaGguf)
                    ),
                    small_button(
                        "Download Q8_0",
                        (!self.busy).then_some(SidebarMessage::DownloadLayaGguf)
                    ),
                ]
                .spacing(6),
                text("Packaged Laya-GGUF runs in-process offline. Downloading Laya-Q8_0.gguf switches to the full 421M checkpoint.")
                    .size(10)
                    .color(muted()),
            ]
            .spacing(6),
        );

        let llm = card(
            column![
                text("Endpoint").size(11),
                text_input("https://...", &self.config.endpoint)
                    .on_input(SidebarMessage::EndpointChanged)
                    .size(11)
                    .padding(6),
                text("Model").size(11),
                text_input("Model name", &self.config.model)
                    .on_input(SidebarMessage::ModelChanged)
                    .size(11)
                    .padding(6),
                text("API key").size(11),
                text_input("Optional bearer token", &self.config.bearer_token)
                    .secure(true)
                    .on_input(SidebarMessage::TokenChanged)
                    .size(11)
                    .padding(6),
                horizontal_rule(1),
                checkbox(
                    "Remote embeddings on index",
                    self.config.use_remote_embeddings
                )
                .size(13)
                .text_size(11)
                .on_toggle(SidebarMessage::RemoteEmbeddingsChanged),
                text("Embedding model").size(11),
                text_input("Embedding model name", &self.config.embedding_model)
                    .on_input(SidebarMessage::EmbeddingModelChanged)
                    .size(11)
                    .padding(6),
                text("HTTPS required except for 127.0.0.1")
                    .size(10)
                    .color(muted()),
                save
            ]
            .spacing(6),
        );

        let storage_card = card(
            column![
                text(self.storage_dir.display().to_string()).size(10),
                text("Contains SQLite index, labels, models, and credentials")
                    .size(10)
                    .color(muted()),
                storage
            ]
            .spacing(6),
        );

        let brand = row![
            container(text("P").size(16).color(on_primary()))
                .width(34)
                .height(34)
                .center_x(34)
                .center_y(34)
                .style(accent_tile_style),
            column![
                text("PSTbeetle").size(14).color(bright_text()),
                text("Thunderbird Mail Explorer").size(10).color(muted())
            ]
            .spacing(1)
            .width(Fill),
            small_button("Hide", Some(SidebarMessage::ToggleSidebar)),
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        let mut contents = column![brand, horizontal_rule(1)].spacing(10).padding(14);
        for (section, label, body) in [
            (
                SidebarSection::Mailboxes,
                "MAILBOXES AND FOLDERS",
                mailboxes_card,
            ),
            (SidebarSection::Labels, "MAIL LABELS", labels_card),
            (SidebarSection::LayaModel, "LAYA DECISION MODEL", laya_card),
            (SidebarSection::AiModels, "LLM JUDGE AND RAG", llm),
            (SidebarSection::Storage, "STORAGE", storage_card),
        ] {
            let collapsed = self.collapsed_sections[section.index()];
            contents = contents.push(collapsible_section_label(
                label,
                collapsed,
                SidebarMessage::ToggleSection(section),
            ));
            if !collapsed {
                contents = contents.push(body);
            }
        }

        container(scrollable(contents))
            .width(285)
            .height(Fill)
            .style(sidebar_style)
            .into()
    }
}
