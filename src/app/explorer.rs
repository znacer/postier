use std::{borrow::Cow, cell::Cell, collections::HashSet};

use dioxus::prelude::*;
use dioxus_primitives::checkbox::CheckboxState;

use crate::{
    components::{
        avatar::{Avatar, AvatarFallback, AvatarImageSize},
        badge::{Badge, BadgeVariant},
        button::{Button, ButtonSize, ButtonVariant},
        checkbox::Checkbox,
        input::Input,
        scroll_area::ScrollArea,
        separator::Separator,
        toggle_group::{ToggleGroup, ToggleItem},
        toolbar::{Toolbar, ToolbarGroup, ToolbarSeparator},
    },
    email_html::{
        ContactEntry, FormattedEmailDate, format_email_body_html, format_email_datetime,
        parse_contact, parse_email_participants, repair_text_encoding,
    },
    i18n::Language,
    model::{EmailRecord, LabelFilter, LabelerBackend, MailScope, MailSort},
};

use super::{DispatchMessage, PstRagApp, theme::*};

thread_local! {
    static CHECKBOX_SHIFT_HELD: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, Debug)]
pub(super) struct CachedExplorerRow {
    pub(super) subject: String,
    pub(super) sender: String,
    pub(super) folder: String,
    pub(super) date_short: String,
}

impl CachedExplorerRow {
    pub(super) fn from_record(email: &EmailRecord, tr: Language) -> Self {
        let repaired_subject = repair_text_encoding(&email.subject);
        let subject = if repaired_subject.trim().is_empty() {
            tr.no_subject()
        } else {
            repaired_subject.as_str()
        };
        let repaired_sender = repair_text_encoding(&email.sender);
        let sender_contact = if repaired_sender.trim().is_empty() {
            None
        } else {
            Some(parse_contact(&repaired_sender))
        };
        let sender = sender_contact
            .as_ref()
            .map(ContactEntry::display_primary)
            .unwrap_or_else(|| tr.unknown_sender());
        let repaired_folder = repair_text_encoding(&email.folder);
        let folder = if repaired_folder.trim().is_empty() {
            tr.root_folder()
        } else {
            repaired_folder.as_str()
        };
        let date_short = format_email_datetime(email.sent_at.as_deref(), tr)
            .map(|dt| dt.table_short)
            .unwrap_or_else(|| tr.unknown_date().to_owned());

        Self {
            subject: truncate_str(subject, 64),
            sender: truncate_str(sender, 36),
            folder: truncate_str(folder, 24),
            date_short,
        }
    }
}

#[derive(Clone)]
pub(super) struct CachedReaderView {
    pub(super) subject: String,
    pub(super) archive_folder_meta: String,
    pub(super) avatar_initials: String,
    pub(super) from: Option<ContactEntry>,
    pub(super) to: Vec<ContactEntry>,
    pub(super) cc: Vec<ContactEntry>,
    pub(super) formatted_dt: Option<FormattedEmailDate>,
    pub(super) attachment_chips: Vec<String>,
    pub(super) is_body_empty: bool,
    pub(super) formatted_html: String,
}

impl CachedReaderView {
    pub(super) fn from_email(email: &EmailRecord, tr: Language) -> Self {
        let repaired_subject = repair_text_encoding(&email.subject);
        let subject = if repaired_subject.trim().is_empty() {
            tr.no_subject().to_owned()
        } else {
            repaired_subject
        };
        let source_name = std::path::Path::new(&email.source_key)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&email.source_key);
        let repaired_folder = repair_text_encoding(&email.folder);
        let folder_display = if repaired_folder.trim().is_empty() {
            tr.root_folder()
        } else {
            repaired_folder.as_str()
        };
        let archive_folder_meta = format!("{source_name} / {folder_display}");

        let participants = parse_email_participants(&email.sender, &email.recipients, &email.body);
        let avatar_initials = participants
            .from
            .as_ref()
            .map(ContactEntry::initials)
            .unwrap_or_else(|| "?".to_string());
        let formatted_dt = format_email_datetime(email.sent_at.as_deref(), tr);

        let repaired_attachments = repair_text_encoding(&email.attachment_names);
        let attachment_chips: Vec<String> = repaired_attachments
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();

        let is_body_empty = email.body.trim().is_empty();
        let formatted_html = if is_body_empty {
            String::new()
        } else {
            format_email_body_html(&email.body)
        };

        Self {
            subject,
            archive_folder_meta,
            avatar_initials,
            from: participants.from,
            to: participants.to,
            cc: participants.cc,
            formatted_dt,
            attachment_chips,
            is_body_empty,
            formatted_html,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) enum ExplorerMessage {
    ToggleSidebar,
    ToggleChatPane,
    SelectEmail(i64),
    ToggleCheckEmail(i64, bool),
    ToggleCheckAllVisible(bool),
    SelectUnlabeledVisible,
    SelectRangeTo(i64),
    ClearCheckedEmails,
    SelectLabelFilter(LabelFilter),
    FilterChanged(String),
    SetMailSort(MailSort),
    SetLabelerBackend(LabelerBackend),
    LabelFocusedEmail,
    LabelSelectedWith(LabelerBackend),
    LabelCheckedWith(LabelerBackend),
    LabelVisibleMessages,
    ToggleManualLabel(i64, String),
    ToggleManualLabelOnChecked(String),
    AskAboutEmail(i64),
    ReaderResizeStart,
    ReaderHeaderResizeStart,
    CancelWorker,
}

pub(super) const fn explorer_filter_input_id() -> &'static str {
    "explorer-quick-filter"
}

pub(super) const fn reader_header_id() -> &'static str {
    "reader-mail-header"
}

impl PstRagApp {
    pub(super) fn explorer(&self) -> Element {
        let tr = self.config.language;
        let muted_color = muted();
        let toolbar = self.explorer_toolbar();
        let has_cancel = self.busy;
        let dot_class = if self.busy {
            "status-dot busy"
        } else {
            "status-dot"
        };
        let stop_lbl = tr.btn_stop_task();
        let status_text = self.status.clone();
        let showing_text = tr.status_showing(
            self.explorer_messages.len(),
            self.count,
            self.checked_message_ids.len(),
            tr.backend_name(self.config.labeler_backend),
        );
        let st_style = status_style();

        let status_bar = rsx! {
            div { style: "width: 100%; padding: 6px 14px; display: flex; flex-direction: row; align-items: center; gap: 10px; flex-shrink: 0; {st_style}",
                span { class: "{dot_class}" }
                span { style: "font-size: 11.5px; color: {muted_color}; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;",
                    "{status_text}"
                }
                if has_cancel {
                    Button {
                        variant: ButtonVariant::Destructive,
                        size: ButtonSize::Xs,
                        style: "font-size: 11px; padding: 3px 10px; border-radius: var(--radius-full);",
                        onclick: move |_| ExplorerMessage::CancelWorker.dispatch(),
                        "{stop_lbl}"
                    }
                }
                span { style: "font-size: 11px; color: {muted_color}; white-space: nowrap; font-variant-numeric: tabular-nums;",
                    "{showing_text}"
                }
            }
        };

        let has_checked = !self.checked_message_ids.is_empty();
        let selection_bar = if has_checked {
            Some(self.selection_action_bar())
        } else {
            None
        };

        let message_table = self.message_table();
        let table_splitter = splitter(
            SplitAxis::Horizontal,
            self.is_resizing(super::Splitter::TableHeight),
            ExplorerMessage::ReaderResizeStart,
        );
        let reading_pane = self.reading_pane();

        rsx! {
            main { style: "flex: 1; min-width: 0; height: 100%; padding: 12px; display: flex; flex-direction: column; gap: 8px; overflow: hidden;",
                {toolbar}
                if let Some(bar) = selection_bar {
                    {bar}
                }
                {status_bar}
                div { style: "flex: 1; min-height: 0; width: 100%; display: flex; flex-direction: column; overflow: hidden;",
                    {message_table}
                    {table_splitter}
                    {reading_pane}
                }
            }
        }
    }

    fn explorer_toolbar(&self) -> Element {
        let tr = self.config.language;
        let bright = bright_text();
        let muted_color = muted();

        let scope_title = match &self.mail_scope {
            MailScope::AllMessages => tr.all_mailboxes().to_owned(),
            MailScope::Source(source) => std::path::Path::new(source)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(source)
                .to_owned(),
            MailScope::Folder {
                source_key,
                folder_prefix,
            } => {
                let archive = std::path::Path::new(source_key)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(source_key);
                format!("{archive} / {folder_prefix}")
            }
        };

        let label_filter_badge = match &self.label_filter {
            LabelFilter::Any => None,
            LabelFilter::Labeled(label) => {
                let badge = label_badge(label, self.label_color(label), None);
                let all_btn = small_button(
                    tr.all_labels(),
                    Some(ExplorerMessage::SelectLabelFilter(LabelFilter::Any)),
                );
                Some(rsx! {
                    div { style: "display: flex; flex-direction: row; align-items: center; gap: 6px;",
                        {badge}
                        {all_btn}
                    }
                })
            }
            LabelFilter::Unlabeled => {
                let badge = label_badge(tr.unlabeled(), "#7F8C8D", None);
                let all_btn = small_button(
                    tr.all_labels(),
                    Some(ExplorerMessage::SelectLabelFilter(LabelFilter::Any)),
                );
                Some(rsx! {
                    div { style: "display: flex; flex-direction: row; align-items: center; gap: 6px;",
                        {badge}
                        {all_btn}
                    }
                })
            }
        };

        let filter_id = explorer_filter_input_id();
        let filter_ph = tr.filter_placeholder();
        let filter_val = self.filter_query.clone();
        let has_filter = !self.filter_query.is_empty();
        let clear_lbl = tr.btn_clear();

        let is_laya = self.config.labeler_backend == LabelerBackend::Laya;
        let laya_name = tr.backend_name(LabelerBackend::Laya);
        let llm_name = tr.backend_name(LabelerBackend::LlmJudge);
        let mut backend_pressed = HashSet::new();
        backend_pressed.insert(if is_laya { 0usize } else { 1usize });

        let checked_count = self.checked_message_ids.len();
        let (label_action_title, label_action_msg) = if checked_count > 0 {
            (
                tr.label_selection_count(checked_count),
                (!self.busy).then_some(ExplorerMessage::LabelCheckedWith(
                    self.config.labeler_backend,
                )),
            )
        } else {
            (
                tr.label_focused_mail().to_owned(),
                (!self.busy && self.selected_email().is_some())
                    .then_some(ExplorerMessage::LabelFocusedEmail),
            )
        };
        let can_label_view = !self.busy && !self.explorer_messages.is_empty();

        let show_sb_btn = if !self.sidebar_open {
            Some(small_button(
                tr.btn_show_sidebar(),
                Some(ExplorerMessage::ToggleSidebar),
            ))
        } else {
            None
        };

        let chat_toggle_label = if self.chat_open {
            tr.btn_hide_chat()
        } else {
            tr.btn_open_chat()
        };
        let chat_toggle_btn =
            primary_button(chat_toggle_label, Some(ExplorerMessage::ToggleChatPane));

        let in_view_text = tr.in_view_checked(self.explorer_messages.len(), checked_count);
        let engine_lbl = tr.decision_engine_label();
        let label_action_btn = enabled_button(label_action_title, label_action_msg);
        let label_view_btn = small_button(
            tr.btn_label_view(),
            can_label_view.then_some(ExplorerMessage::LabelVisibleMessages),
        );
        let select_unlabeled_btn = small_button(
            tr.btn_select_unlabeled(),
            Some(ExplorerMessage::SelectUnlabeledVisible),
        );
        let tb_style = toolbar_style();

        rsx! {
            header { style: "width: 100%; padding: 10px 14px; display: flex; flex-direction: column; gap: 8px; flex-shrink: 0; {tb_style}",
                div { style: "display: flex; flex-direction: row; align-items: center; gap: 10px; flex-wrap: wrap;",
                    if let Some(sb_btn) = show_sb_btn {
                        {sb_btn}
                    }
                    div { style: "display: flex; flex-direction: column; gap: 1px;",
                        span { style: "font-size: 15px; color: {bright}; font-weight: 700; letter-spacing: -0.01em;",
                            "{scope_title}"
                        }
                        span { style: "font-size: 11px; color: {muted_color}; font-variant-numeric: tabular-nums;",
                            "{in_view_text}"
                        }
                    }
                    if let Some(badge) = label_filter_badge {
                        {badge}
                    }
                    div { style: "flex: 1;" }
                    div { style: "position: relative; display: flex; align-items: center;",
                        svg {
                            width: "14",
                            height: "14",
                            view_box: "0 0 24 24",
                            fill: "none",
                            stroke: "{muted_color}",
                            stroke_width: "2",
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                            style: "position: absolute; left: 10px; z-index: 2; pointer-events: none;",
                            circle { cx: "11", cy: "11", r: "8" }
                            line {
                                x1: "21",
                                y1: "21",
                                x2: "16.65",
                                y2: "16.65",
                            }
                        }
                        Input {
                            id: "{filter_id}",
                            r#type: "text",
                            placeholder: "{filter_ph}",
                            value: "{filter_val}",
                            style: "width: 265px; padding: 6px 28px 6px 30px; font-size: 12px;",
                            oninput: move |evt: FormEvent| ExplorerMessage::FilterChanged(evt.value()).dispatch(),
                            onkeydown: move |evt: KeyboardEvent| {
                                if evt.key() != Key::Escape {
                                    evt.stop_propagation();
                                }
                            },
                        }
                        if has_filter {
                            Button {
                                variant: ButtonVariant::Ghost,
                                size: ButtonSize::IconXs,
                                title: "{clear_lbl}",
                                style: "position: absolute; right: 4px; padding: 2px 6px; font-size: 11px; color: {muted_color};",
                                onclick: move |_| ExplorerMessage::FilterChanged(String::new()).dispatch(),
                                "×"
                            }
                        } else {
                            Badge {
                                variant: BadgeVariant::Outline,
                                style: "position: absolute; right: 8px; padding: 1px 5px; height: 18px; min-width: 18px; font-size: 10px; font-family: var(--font-mono); color: {muted_color}; pointer-events: none;",
                                "/"
                            }
                        }
                    }
                    {chat_toggle_btn}
                }
                Separator {}
                Toolbar { style: "width: 100%; padding: 0; box-shadow: none; justify-content: flex-start; gap: 10px;",
                    ToolbarGroup { style: "align-items: center; gap: 8px;",
                        span { style: "font-size: 11.5px; color: {muted_color}; font-weight: 500;",
                            "{engine_lbl}"
                        }
                        ToggleGroup {
                            horizontal: true,
                            allow_multiple_pressed: false,
                            pressed: backend_pressed,
                            on_pressed_change: move |pressed: HashSet<usize>| {
                                if pressed.contains(&0) {
                                    ExplorerMessage::SetLabelerBackend(LabelerBackend::Laya).dispatch();
                                } else if pressed.contains(&1) {
                                    ExplorerMessage::SetLabelerBackend(LabelerBackend::LlmJudge).dispatch();
                                }
                            },
                            ToggleItem {
                                index: 0usize,
                                style: "padding: 4px 10px; font-size: 11px;",
                                "{laya_name}"
                            }
                            ToggleItem {
                                index: 1usize,
                                style: "padding: 4px 10px; font-size: 11px;",
                                "{llm_name}"
                            }
                        }
                    }
                    ToolbarSeparator { horizontal: false }
                    ToolbarGroup { style: "align-items: center; gap: 8px; flex-wrap: wrap;",
                        {label_action_btn}
                        {label_view_btn}
                        {select_unlabeled_btn}
                    }
                }
            }
        }
    }

    fn selection_action_bar(&self) -> Element {
        let tr = self.config.language;
        let muted_color = muted();
        let bright = bright_text();
        let count = self.checked_message_ids.len();

        let mut manual_chips = Vec::new();
        for (def, _) in &self.labels_with_counts {
            let all_have_label = self
                .explorer_messages
                .iter()
                .filter(|m| self.checked_message_ids.contains(&m.id))
                .all(|m| m.labels.iter().any(|l| l.label == def.name))
                && self
                    .explorer_messages
                    .iter()
                    .any(|m| self.checked_message_ids.contains(&m.id));
            let chip_label = if all_have_label {
                tr.remove_label_chip(&def.name)
            } else {
                tr.add_label_chip(&def.name)
            };
            let chip_variant = if all_have_label {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Outline
            };
            let label_name = def.name.clone();
            let chip_key = def.name.clone();
            manual_chips.push(rsx! {
                div {
                    key: "chip-{chip_key}",
                    style: "max-width: {BUTTON_MAX_WIDTH}px; max-height: {BUTTON_MAX_HEIGHT}px; overflow: hidden; display: inline-flex; flex-shrink: 0;",
                    Button {
                        variant: chip_variant,
                        size: ButtonSize::Xs,
                        style: "font-size: 10.5px; padding: 3px 9px; border-radius: var(--radius-full);",
                        onclick: move |_| ExplorerMessage::ToggleManualLabelOnChecked(label_name.clone()).dispatch(),
                        "{chip_label}"
                    }
                }
            });
        }

        let batch_lbl = tr.batch_label_label();
        let selected_count_lbl = tr.mails_selected_count(count);
        let laya_batch_btn = primary_button(
            tr.label_count_with_laya(count),
            (!self.busy).then_some(ExplorerMessage::LabelCheckedWith(LabelerBackend::Laya)),
        );
        let llm_batch_btn = small_button(
            tr.label_count_with_llm(count),
            (!self.busy).then_some(ExplorerMessage::LabelCheckedWith(LabelerBackend::LlmJudge)),
        );
        let clear_sel_btn = small_button(
            tr.btn_clear_selection(),
            Some(ExplorerMessage::ClearCheckedEmails),
        );
        let sb_style = selection_bar_style();

        rsx! {
            div { style: "width: 100%; padding: 8px 14px; display: flex; flex-direction: row; align-items: center; gap: 10px; flex-shrink: 0; {sb_style}",
                span { style: "font-size: 12px; font-weight: 600; color: {bright}; white-space: nowrap;",
                    "{selected_count_lbl}"
                }
                {laya_batch_btn}
                {llm_batch_btn}
                Separator { horizontal: false, style: "height: 18px;" }
                div { style: "flex: 1; display: flex; flex-direction: row; align-items: center; gap: 6px; flex-wrap: wrap;",
                    span { style: "font-size: 11px; color: {muted_color}; font-weight: 500;",
                        "{batch_lbl}"
                    }
                    for chip in manual_chips {
                        {chip}
                    }
                }
                {clear_sel_btn}
            }
        }
    }

    fn message_table(&self) -> Element {
        let tr = self.config.language;
        let muted_color = muted();
        let sort_date_label = match self.mail_sort {
            MailSort::DateDesc => tr.table_header_date_newest(),
            MailSort::DateAsc => tr.table_header_date_oldest(),
            _ => tr.table_header_date(),
        };
        let sort_subj_label = if self.mail_sort == MailSort::SubjectAsc {
            tr.table_header_subj_az()
        } else {
            tr.table_header_subj()
        };
        let sort_from_label = if self.mail_sort == MailSort::SenderAsc {
            tr.table_header_from_az()
        } else {
            tr.table_header_from()
        };

        let all_visible_checked = if !self.explorer_messages.is_empty()
            && self
                .explorer_messages
                .iter()
                .all(|m| self.checked_message_ids.contains(&m.id))
        {
            CheckboxState::Checked
        } else {
            CheckboxState::Unchecked
        };
        let next_date_sort = if self.mail_sort == MailSort::DateDesc {
            MailSort::DateAsc
        } else {
            MailSort::DateDesc
        };

        let labels_hdr = tr.table_header_labels();
        let folder_hdr = tr.table_header_folder();

        let header = rsx! {
            div { class: "mail-table-header",
                Checkbox {
                    checked: all_visible_checked,
                    on_checked_change: move |state: CheckboxState| {
                        ExplorerMessage::ToggleCheckAllVisible(matches!(state, CheckboxState::Checked))
                            .dispatch();
                    },
                }
                Button {
                    variant: ButtonVariant::Ghost,
                    size: ButtonSize::Xs,
                    style: "flex: 4; padding: 2px 4px; font-size: 11px; font-weight: 600; letter-spacing: 0.04em; color: {muted_color}; justify-content: flex-start;",
                    onclick: move |_| ExplorerMessage::SetMailSort(MailSort::SubjectAsc).dispatch(),
                    "{sort_subj_label}"
                }
                span { style: "flex: 3;", "{labels_hdr}" }
                Button {
                    variant: ButtonVariant::Ghost,
                    size: ButtonSize::Xs,
                    style: "flex: 3; padding: 2px 4px; font-size: 11px; font-weight: 600; letter-spacing: 0.04em; color: {muted_color}; justify-content: flex-start;",
                    onclick: move |_| ExplorerMessage::SetMailSort(MailSort::SenderAsc).dispatch(),
                    "{sort_from_label}"
                }
                span { style: "flex: 2;", "{folder_hdr}" }
                Button {
                    variant: ButtonVariant::Ghost,
                    size: ButtonSize::Xs,
                    style: "flex: 2; padding: 2px 4px; font-size: 11px; font-weight: 600; letter-spacing: 0.04em; color: {muted_color}; justify-content: flex-start;",
                    onclick: move |_| ExplorerMessage::SetMailSort(next_date_sort).dispatch(),
                    "{sort_date_label}"
                }
            }
        };

        let is_empty = self.explorer_messages.is_empty();
        let empty_msg = if self.count == 0 {
            tr.empty_table_no_indexed()
        } else {
            tr.empty_table_no_match()
        };

        let rows: Vec<Element> = self
            .explorer_messages
            .iter()
            .enumerate()
            .map(|(idx, email)| self.message_row(email, self.cached_explorer_rows.get(idx)))
            .collect();

        let table_height = self.table_height;
        let rb_style = reader_body_style();

        rsx! {
            section { style: "width: 100%; height: {table_height}px; flex-shrink: 0; display: flex; flex-direction: column; overflow: hidden; {rb_style}",
                {header}
                ScrollArea { style: "flex: 1; min-height: 0; width: 100%; display: flex; flex-direction: column;",
                    if is_empty {
                        div { style: "width: 100%; height: 100%; padding: 32px 24px; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 10px;",
                            svg {
                                width: "28",
                                height: "28",
                                view_box: "0 0 24 24",
                                fill: "none",
                                stroke: "{muted_color}",
                                stroke_width: "1.6",
                                stroke_linecap: "round",
                                stroke_linejoin: "round",
                                style: "opacity: 0.55;",
                                polyline { points: "22 12 16 12 14 15 10 15 8 12 2 12" }
                                path { d: "M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z" }
                            }
                            span { style: "font-size: 12.5px; color: {muted_color};",
                                "{empty_msg}"
                            }
                        }
                    } else {
                        for r in rows {
                            {r}
                        }
                    }
                }
            }
        }
    }

    fn message_row(&self, email: &EmailRecord, cached: Option<&CachedExplorerRow>) -> Element {
        let tr = self.config.language;
        let muted_color = muted();
        let is_selected = self.selected_message_id == Some(email.id);
        let is_checked = self.checked_message_ids.contains(&email.id);
        let email_id = email.id;
        let cached = match cached {
            Some(row) => Cow::Borrowed(row),
            None => Cow::Owned(CachedExplorerRow::from_record(email, tr)),
        };
        let subj_trunc = &cached.subject;
        let sender_trunc = &cached.sender;
        let folder_trunc = &cached.folder;
        let date_short = &cached.date_short;

        let mut label_badges = Vec::new();
        let no_labels = email.labels.is_empty();
        let none_lbl = tr.none_label();
        if !no_labels {
            for assigned in email.labels.iter().take(2) {
                let color = self.label_color(&assigned.label);
                let pct = assigned.confidence.map(|c| format!("{:.0}%", c * 100.0));
                label_badges.push(label_badge(&assigned.label, color, pct));
            }
        }

        let subj_color = bright_text();
        let subj_weight = if is_selected { "600" } else { "500" };
        let sender_color = if is_selected {
            bright_text()
        } else {
            muted_color
        };
        let row_class = match (is_selected, is_checked) {
            (true, _) => "mail-row selected",
            (false, true) => "mail-row checked",
            (false, false) => "mail-row",
        };
        let check_state = if is_checked {
            CheckboxState::Checked
        } else {
            CheckboxState::Unchecked
        };

        rsx! {
            div { key: "{email_id}", class: "{row_class}",
                div {
                    style: "display: inline-flex; align-items: center;",
                    onmousedown: move |evt: MouseEvent| {
                        CHECKBOX_SHIFT_HELD.with(|cell| cell.set(evt.modifiers().shift()));
                    },
                    Checkbox {
                        checked: check_state,
                        on_checked_change: move |state: CheckboxState| {
                            let shift = CHECKBOX_SHIFT_HELD.with(|cell| cell.replace(false));
                            if shift {
                                ExplorerMessage::SelectRangeTo(email_id).dispatch();
                            } else {
                                ExplorerMessage::ToggleCheckEmail(
                                        email_id,
                                        matches!(state, CheckboxState::Checked),
                                    )
                                    .dispatch();
                            }
                        },
                    }
                }
                Button {
                    variant: ButtonVariant::Ghost,
                    style: "flex: 1; min-width: 0; height: auto; padding: 0; display: flex; flex-direction: row; align-items: center; gap: 10px;",
                    onclick: move |evt: MouseEvent| {
                        if evt.modifiers().shift() {
                            ExplorerMessage::SelectRangeTo(email_id).dispatch();
                        } else {
                            ExplorerMessage::SelectEmail(email_id).dispatch();
                        }
                    },
                    span { style: "flex: 4; text-align: left; font-size: 12.5px; font-weight: {subj_weight}; color: {subj_color}; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;",
                        "{subj_trunc}"
                    }
                    div { style: "flex: 3; display: flex; flex-direction: row; align-items: center; gap: 4px; overflow: hidden;",
                        if no_labels {
                            span { style: "font-size: 10.5px; color: {muted_color}; opacity: 0.6;",
                                "{none_lbl}"
                            }
                        } else {
                            for b in label_badges {
                                {b}
                            }
                        }
                    }
                    span { style: "flex: 3; text-align: left; font-size: 11.5px; color: {sender_color}; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;",
                        "{sender_trunc}"
                    }
                    span { style: "flex: 2; text-align: left; font-size: 11px; color: {muted_color}; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;",
                        "{folder_trunc}"
                    }
                    span { style: "flex: 2; text-align: left; font-size: 11px; color: {muted_color}; font-variant-numeric: tabular-nums; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;",
                        "{date_short}"
                    }
                }
            }
        }
    }

    fn reading_pane(&self) -> Element {
        let tr = self.config.language;
        let bright = bright_text();
        let muted_color = muted();
        let rb_style = reader_body_style();

        let Some(email) = self.selected_email() else {
            let no_msg = tr.no_message_selected();
            let no_msg_hint = tr.no_message_selected_hint();
            return rsx! {
                div { style: "flex: 1; min-height: 0; width: 100%; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; {rb_style}",
                    svg {
                        width: "32",
                        height: "32",
                        view_box: "0 0 24 24",
                        fill: "none",
                        stroke: "{muted_color}",
                        stroke_width: "1.5",
                        stroke_linecap: "round",
                        stroke_linejoin: "round",
                        style: "opacity: 0.45;",
                        rect {
                            x: "2",
                            y: "4",
                            width: "20",
                            height: "16",
                            rx: "2",
                        }
                        path { d: "m22 7-8.97 5.7a1.94 1.94 0 0 1-2.06 0L2 7" }
                    }
                    span { style: "font-size: 15px; font-weight: 600; color: {bright};",
                        "{no_msg}"
                    }
                    span { style: "font-size: 12px; color: {muted_color};", "{no_msg_hint}" }
                }
            };
        };

        let cached = match &self.cached_reader {
            Some(view) => Cow::Borrowed(view),
            None => Cow::Owned(CachedReaderView::from_email(email, tr)),
        };

        let subject = &cached.subject;
        let archive_folder_meta = &cached.archive_folder_meta;

        let labels_hdr = tr.labels_heading();
        let none_assigned_lbl = tr.none_assigned();
        let is_labels_empty = email.labels.is_empty();
        let mut assigned_badges = Vec::new();
        for assigned in &email.labels {
            let detail = match assigned.confidence {
                Some(c) => format!("{} {:.0}%", assigned.source, c * 100.0),
                None => assigned.source.clone(),
            };
            assigned_badges.push(label_badge(
                &assigned.label,
                self.label_color(&assigned.label),
                Some(detail),
            ));
        }

        let toggle_hdr = tr.toggle_heading();
        let mut quick_toggle_buttons = Vec::new();
        for (def, _) in &self.labels_with_counts {
            let active = email.labels.iter().any(|l| l.label == def.name);
            let btn_label = if active {
                tr.remove_label_chip(&def.name)
            } else {
                tr.add_label_chip(&def.name)
            };
            let btn_variant = if active {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Outline
            };
            let msg_id = email.id;
            let label_name = def.name.clone();
            let toggle_key = def.name.clone();
            quick_toggle_buttons.push(rsx! {
                Button {
                    key: "toggle-{toggle_key}",
                    variant: btn_variant,
                    size: ButtonSize::Xs,
                    style: "font-size: 10.5px; padding: 3px 9px; border-radius: var(--radius-full);",
                    onclick: move |_| ExplorerMessage::ToggleManualLabel(msg_id, label_name.clone()).dispatch(),
                    "{btn_label}"
                }
            });
        }

        let btn_laya = small_button(
            tr.btn_label_with_laya(),
            (!self.busy).then_some(ExplorerMessage::LabelSelectedWith(LabelerBackend::Laya)),
        );
        let btn_llm = small_button(
            tr.btn_label_with_llm(),
            (!self.busy).then_some(ExplorerMessage::LabelSelectedWith(LabelerBackend::LlmJudge)),
        );
        let btn_ask = primary_button(
            tr.btn_ask_about_mail(),
            Some(ExplorerMessage::AskAboutEmail(email.id)),
        );

        let avatar_initials = &cached.avatar_initials;

        let (from_lbl, to_lbl, cc_lbl, unknown_rcpt_lbl) = match tr {
            Language::French => ("De", "À", "Cc", "(Destinataire inconnu)"),
            Language::English => ("From", "To", "Cc", "(Unknown recipient)"),
        };
        let unknown_sender_lbl = tr.unknown_sender();

        let from_pill = match &cached.from {
            Some(c) => render_contact_pill("from-0", c, true),
            None => rsx! {
                span { style: "font-size: 11.5px; color: {muted_color}; font-style: italic;",
                    "{unknown_sender_lbl}"
                }
            },
        };

        let to_pills: Vec<Element> = cached
            .to
            .iter()
            .enumerate()
            .map(|(idx, c)| render_contact_pill(&format!("to-{idx}"), c, false))
            .collect();
        let has_to = !to_pills.is_empty();

        let cc_pills: Vec<Element> = cached
            .cc
            .iter()
            .enumerate()
            .map(|(idx, c)| render_contact_pill(&format!("cc-{idx}"), c, false))
            .collect();
        let has_cc = !cc_pills.is_empty();

        let formatted_dt = &cached.formatted_dt;
        let unknown_date_lbl = tr.unknown_date_full();
        let attachment_chips = &cached.attachment_chips;

        let rh_id = reader_header_id();
        let rh_style = reader_header_style();
        let max_hdr = super::HEADER_MAX_HEIGHT;
        let header_height_style = match self.reader_header_height {
            Some(height) => {
                let h = height.min(max_hdr);
                format!("height: {h}px; max-height: {max_hdr}px;")
            }
            None => format!("height: auto; max-height: {max_hdr}px;"),
        };

        let is_body_empty = cached.is_body_empty;
        let empty_body_text = tr.empty_body();
        let formatted_html = &cached.formatted_html;

        let header_splitter = splitter(
            SplitAxis::Horizontal,
            self.is_resizing(super::Splitter::HeaderHeight),
            ExplorerMessage::ReaderHeaderResizeStart,
        );

        rsx! {
            article { style: "flex: 1; min-height: 0; width: 100%; display: flex; flex-direction: column; overflow: hidden;",
                div {
                    id: "{rh_id}",
                    class: "selectable-text",
                    style: "width: 100%; flex-shrink: 0; overflow-y: auto; {header_height_style} {rh_style}",
                    div { style: "padding: 12px 16px; width: 100%; display: flex; flex-direction: column; gap: 9px;",
                        div { style: "display: flex; flex-direction: row; align-items: center; gap: 8px; flex-wrap: wrap;",
                            span { style: "flex: 1; font-size: 16px; color: {bright}; font-weight: 700; letter-spacing: -0.01em;",
                                "{subject}"
                            }
                            {btn_laya}
                            {btn_llm}
                            {btn_ask}
                        }
                        div { class: "email-header-envelope",
                            Avatar {
                                size: AvatarImageSize::Small,
                                style: "width: 36px; height: 36px;",
                                AvatarFallback {
                                    class: "email-sender-avatar",
                                    style: "font-size: 12px;",
                                    "{avatar_initials}"
                                }
                            }
                            div { class: "email-envelope-fields",
                                div { class: "email-field-row",
                                    span { class: "email-field-label", "{from_lbl}" }
                                    div { class: "email-field-values", {from_pill} }
                                }
                                div { class: "email-field-row",
                                    span { class: "email-field-label", "{to_lbl}" }
                                    div { class: "email-field-values",
                                        if has_to {
                                            for tp in to_pills {
                                                {tp}
                                            }
                                        } else {
                                            span { style: "font-size: 11.5px; color: {muted_color}; font-style: italic;",
                                                "{unknown_rcpt_lbl}"
                                            }
                                        }
                                    }
                                }
                                if has_cc {
                                    div { class: "email-field-row",
                                        span { class: "email-field-label", "{cc_lbl}" }
                                        div { class: "email-field-values",
                                            for cp in cc_pills {
                                                {cp}
                                            }
                                        }
                                    }
                                }
                            }
                            div { class: "email-datetime-card",
                                if let Some(dt) = formatted_dt {
                                    div { class: "email-datetime-primary",
                                        svg {
                                            width: "13",
                                            height: "13",
                                            view_box: "0 0 24 24",
                                            fill: "none",
                                            stroke: "{muted_color}",
                                            stroke_width: "2",
                                            stroke_linecap: "round",
                                            stroke_linejoin: "round",
                                            rect {
                                                x: "3",
                                                y: "4",
                                                width: "18",
                                                height: "18",
                                                rx: "2",
                                                ry: "2",
                                            }
                                            line {
                                                x1: "16",
                                                y1: "2",
                                                x2: "16",
                                                y2: "6",
                                            }
                                            line {
                                                x1: "8",
                                                y1: "2",
                                                x2: "8",
                                                y2: "6",
                                            }
                                            line {
                                                x1: "3",
                                                y1: "10",
                                                x2: "21",
                                                y2: "10",
                                            }
                                        }
                                        span { "{dt.date_part}" }
                                        if !dt.time_part.is_empty() {
                                            span { class: "email-time-badge", "{dt.time_part}" }
                                        }
                                    }
                                } else {
                                    div { class: "email-datetime-primary",
                                        span { style: "color: {muted_color};", "{unknown_date_lbl}" }
                                    }
                                }
                                span { class: "email-folder-pill", "📁 {archive_folder_meta}" }
                            }
                        }
                        if !attachment_chips.is_empty() {
                            div { style: "display: flex; flex-direction: row; align-items: center; flex-wrap: wrap; gap: 6px;",
                                for (idx, att) in attachment_chips.iter().enumerate() {
                                    Badge {
                                        key: "att-{idx}-{att}",
                                        variant: BadgeVariant::Secondary,
                                        "📎 {att}"
                                    }
                                }
                            }
                        }
                        Separator {}
                        div { style: "display: flex; flex-direction: row; align-items: center; gap: 12px; flex-wrap: wrap;",
                            div { style: "flex: 1; display: flex; flex-direction: row; align-items: center; gap: 6px; flex-wrap: wrap;",
                                span { style: "font-size: 11px; font-weight: 600; color: {muted_color};",
                                    "{labels_hdr}"
                                }
                                if is_labels_empty {
                                    span { style: "font-size: 11px; color: {muted_color}; opacity: 0.7;",
                                        "{none_assigned_lbl}"
                                    }
                                } else {
                                    for ab in assigned_badges {
                                        {ab}
                                    }
                                }
                            }
                            div { style: "flex: 1; display: flex; flex-direction: row; align-items: center; gap: 5px; flex-wrap: wrap;",
                                span { style: "font-size: 10.5px; font-weight: 600; color: {muted_color};",
                                    "{toggle_hdr}"
                                }
                                for qtb in quick_toggle_buttons {
                                    {qtb}
                                }
                            }
                        }
                    }
                }
                {header_splitter}
                ScrollArea {
                    class: "selectable-text",
                    style: "flex: 1; min-height: 0; width: 100%; {rb_style}",
                    if is_body_empty {
                        div { style: "padding: 18px 22px; width: 100%; font-size: 13px; font-style: italic; color: {muted_color};",
                            "{empty_body_text}"
                        }
                    } else {
                        div {
                            class: "email-html-body",
                            style: "padding: 18px 22px; width: 100%;",
                            dangerous_inner_html: "{formatted_html}",
                        }
                    }
                }
            }
        }
    }
}

fn render_contact_pill(key_id: &str, contact: &ContactEntry, is_sender: bool) -> Element {
    let pill_class = if is_sender {
        "email-contact-pill sender"
    } else {
        "email-contact-pill"
    };
    let inner = match (contact.name.as_deref(), contact.email.as_deref()) {
        (Some(name), Some(addr)) => {
            let name_s = name.to_owned();
            let addr_bracket = format!("<{addr}>");
            rsx! {
                span { class: "email-contact-name", "{name_s}" }
                span { class: "email-contact-addr", "{addr_bracket}" }
            }
        }
        (Some(name), None) => {
            let name_s = name.to_owned();
            rsx! {
                span { class: "email-contact-name", "{name_s}" }
            }
        }
        (None, Some(addr)) => {
            let addr_s = addr.to_owned();
            rsx! {
                span { class: "email-contact-addr", "{addr_s}" }
            }
        }
        (None, None) => {
            let raw_s = contact.raw.clone();
            rsx! {
                span { class: "email-contact-name", "{raw_s}" }
            }
        }
    };

    rsx! {
        span { key: "{key_id}", class: "{pill_class}", {inner} }
    }
}

fn truncate_str(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in value.chars().take(max_chars) {
        out.push(if ch == '\n' || ch == '\r' { ' ' } else { ch });
    }
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}
