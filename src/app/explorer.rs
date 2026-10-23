use iced::{
    Alignment, Element, Fill, FillPortion, Length,
    widget::{
        Space, button, checkbox, column, container, horizontal_rule, row, scrollable, text,
        text_input,
    },
};

use crate::model::{EmailRecord, LabelFilter, LabelerBackend, MailScope, MailSort};

use super::{PstRagApp, theme::*};

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

pub(super) fn explorer_filter_input_id() -> text_input::Id {
    text_input::Id::new("explorer-quick-filter")
}

pub(super) fn reader_header_id() -> container::Id {
    container::Id::new("reader-mail-header")
}

impl PstRagApp {
    pub(super) fn explorer(&self) -> Element<'_, ExplorerMessage> {
        let toolbar = self.explorer_toolbar();
        let cancel_btn: Element<'_, ExplorerMessage> = if self.busy {
            small_button("Stop Task", Some(ExplorerMessage::CancelWorker)).into()
        } else {
            Space::new(0, 0).into()
        };
        let status_bar = container(
            row![
                text(&self.status).size(11).color(muted()).width(Fill),
                cancel_btn,
                text(format!(
                    "Showing {} of {} indexed, {} selected, Labeler: {}",
                    self.explorer_messages.len(),
                    self.count,
                    self.checked_message_ids.len(),
                    self.config.labeler_backend.display_name()
                ))
                .size(11)
                .color(muted())
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .padding([5, 12])
        .width(Fill)
        .style(status_style);

        let message_table = self.message_table();
        let reading_pane = self.reading_pane();

        let mut col = column![toolbar].spacing(8).padding(10).height(Fill);
        if !self.checked_message_ids.is_empty() {
            col = col.push(self.selection_action_bar());
        }
        let split_area = column![
            message_table,
            splitter(
                SplitAxis::Horizontal,
                self.is_resizing(super::Splitter::TableHeight),
                ExplorerMessage::ReaderResizeStart,
            ),
            reading_pane
        ]
        .height(Fill);
        col = col.push(status_bar).push(split_area);

        container(col).width(Fill).height(Fill).into()
    }

    fn explorer_toolbar(&self) -> Element<'_, ExplorerMessage> {
        let scope_title = match &self.mail_scope {
            MailScope::AllMessages => "All Mailboxes".to_owned(),
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

        let label_filter_badge: Element<'_, ExplorerMessage> = match &self.label_filter {
            LabelFilter::Any => Space::new(0, 0).into(),
            LabelFilter::Labeled(label) => row![
                label_badge(label, self.label_color(label), None),
                small_button(
                    "All labels",
                    Some(ExplorerMessage::SelectLabelFilter(LabelFilter::Any))
                )
            ]
            .spacing(4)
            .align_y(Alignment::Center)
            .into(),
            LabelFilter::Unlabeled => row![
                label_badge("Unlabeled", "#7F8C8D", None),
                small_button(
                    "All labels",
                    Some(ExplorerMessage::SelectLabelFilter(LabelFilter::Any))
                )
            ]
            .spacing(4)
            .align_y(Alignment::Center)
            .into(),
        };

        let filter_input = text_input(
            "Quick filter (subject, sender, body)... [/]",
            &self.filter_query,
        )
        .id(explorer_filter_input_id())
        .on_input(ExplorerMessage::FilterChanged)
        .padding([6, 10])
        .size(12)
        .width(250);

        let clear_filter: Element<'_, ExplorerMessage> = if self.filter_query.is_empty() {
            Space::new(0, 0).into()
        } else {
            small_button("Clear", Some(ExplorerMessage::FilterChanged(String::new()))).into()
        };

        let laya_btn = button(text("Laya (GGUF)").size(11))
            .padding([5, 10])
            .style(if self.config.labeler_backend == LabelerBackend::Laya {
                button::primary
            } else {
                button::secondary
            })
            .on_press(ExplorerMessage::SetLabelerBackend(LabelerBackend::Laya));

        let llm_btn = button(text("LLM Judge").size(11))
            .padding([5, 10])
            .style(if self.config.labeler_backend == LabelerBackend::LlmJudge {
                button::primary
            } else {
                button::secondary
            })
            .on_press(ExplorerMessage::SetLabelerBackend(LabelerBackend::LlmJudge));

        let checked_count = self.checked_message_ids.len();
        let (label_action_title, label_action_msg) = if checked_count > 0 {
            (
                format!("Label Selection ({checked_count})"),
                (!self.busy).then_some(ExplorerMessage::LabelCheckedWith(
                    self.config.labeler_backend,
                )),
            )
        } else {
            (
                "Label Focused Mail".to_owned(),
                (!self.busy && self.selected_email().is_some())
                    .then_some(ExplorerMessage::LabelFocusedEmail),
            )
        };
        let can_label_view = !self.busy && !self.explorer_messages.is_empty();

        let sidebar_toggle_btn: Element<'_, ExplorerMessage> = if !self.sidebar_open {
            small_button("Show Sidebar", Some(ExplorerMessage::ToggleSidebar)).into()
        } else {
            Space::new(0, 0).into()
        };

        let chat_toggle_label = if self.chat_open {
            "Hide Chat"
        } else {
            "Open Chat"
        };

        let top_row = row![
            sidebar_toggle_btn,
            column![
                text(scope_title).size(15).color(bright_text()),
                text(format!(
                    "{} in view, {} checked",
                    self.explorer_messages.len(),
                    checked_count
                ))
                .size(10)
                .color(muted())
            ]
            .spacing(1),
            label_filter_badge,
            Space::new(Fill, 0),
            filter_input,
            clear_filter,
            primary_button(chat_toggle_label, Some(ExplorerMessage::ToggleChatPane)),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        let bottom_row = row![
            text("Decision engine:").size(11).color(muted()),
            row![laya_btn, llm_btn].spacing(4),
            Space::new(8, 0),
            enabled_button(label_action_title, label_action_msg),
            enabled_button(
                "Label View",
                can_label_view.then_some(ExplorerMessage::LabelVisibleMessages)
            ),
            small_button(
                "Select Unlabeled",
                Some(ExplorerMessage::SelectUnlabeledVisible)
            ),
            Space::new(Fill, 0),
        ]
        .spacing(8)
        .align_y(Alignment::Center);

        container(column![top_row, bottom_row].spacing(6))
            .padding([8, 12])
            .width(Fill)
            .style(toolbar_style)
            .into()
    }

    fn selection_action_bar(&self) -> Element<'_, ExplorerMessage> {
        let count = self.checked_message_ids.len();
        let mut manual_chips = row![text("Batch label:").size(10).color(muted())]
            .spacing(5)
            .align_y(Alignment::Center);

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
                format!("Remove {}", def.name)
            } else {
                format!("Add {}", def.name)
            };
            let label_name = def.name.clone();
            manual_chips = manual_chips.push(
                button(text(chip_label).size(10))
                    .padding([3, 8])
                    .style(if all_have_label {
                        button::primary
                    } else {
                        button::secondary
                    })
                    .on_press(ExplorerMessage::ToggleManualLabelOnChecked(label_name)),
            );
        }

        container(
            row![
                text(format!(
                    "{count} mail{} selected",
                    if count == 1 { "" } else { "s" }
                ))
                .size(12)
                .color(on_primary()),
                primary_button(
                    format!("Label {count} with Laya"),
                    (!self.busy).then_some(ExplorerMessage::LabelCheckedWith(LabelerBackend::Laya))
                ),
                small_button(
                    format!("Label {count} with LLM Judge"),
                    (!self.busy)
                        .then_some(ExplorerMessage::LabelCheckedWith(LabelerBackend::LlmJudge))
                ),
                Space::new(8, 0),
                container(manual_chips.wrap()).width(Fill),
                small_button("Clear Selection", Some(ExplorerMessage::ClearCheckedEmails)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding([7, 12])
        .width(Fill)
        .style(selection_bar_style)
        .into()
    }

    fn message_table(&self) -> Element<'_, ExplorerMessage> {
        let sort_date_label = match self.mail_sort {
            MailSort::DateDesc => "DATE (Newest)",
            MailSort::DateAsc => "DATE (Oldest)",
            _ => "DATE",
        };
        let sort_subj_label = if self.mail_sort == MailSort::SubjectAsc {
            "SUBJECT (A-Z)"
        } else {
            "SUBJECT"
        };
        let sort_from_label = if self.mail_sort == MailSort::SenderAsc {
            "CORRESPONDENT (A-Z)"
        } else {
            "CORRESPONDENT"
        };

        let all_visible_checked = !self.explorer_messages.is_empty()
            && self
                .explorer_messages
                .iter()
                .all(|m| self.checked_message_ids.contains(&m.id));

        let header = container(
            row![
                checkbox("", all_visible_checked)
                    .on_toggle(ExplorerMessage::ToggleCheckAllVisible)
                    .size(14),
                small_button("Unlabeled", Some(ExplorerMessage::SelectUnlabeledVisible)),
                button(text(sort_subj_label).size(10).color(muted()))
                    .padding(0)
                    .style(button::text)
                    .width(FillPortion(4))
                    .on_press(ExplorerMessage::SetMailSort(MailSort::SubjectAsc)),
                text("LABELS").size(10).color(muted()).width(FillPortion(3)),
                button(text(sort_from_label).size(10).color(muted()))
                    .padding(0)
                    .style(button::text)
                    .width(FillPortion(3))
                    .on_press(ExplorerMessage::SetMailSort(MailSort::SenderAsc)),
                text("FOLDER").size(10).color(muted()).width(FillPortion(2)),
                button(text(sort_date_label).size(10).color(muted()))
                    .padding(0)
                    .style(button::text)
                    .width(FillPortion(2))
                    .on_press(ExplorerMessage::SetMailSort(
                        if self.mail_sort == MailSort::DateDesc {
                            MailSort::DateAsc
                        } else {
                            MailSort::DateDesc
                        }
                    )),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding([6, 12])
        .width(Fill)
        .style(table_header_style);

        let mut list = column![].spacing(1).width(Fill);
        if self.explorer_messages.is_empty() {
            let empty_msg = if self.count == 0 {
                "No emails indexed yet. Choose a .pst archive or .jsonl fixture in the left pane and click 'Index now'."
            } else {
                "No messages match the current folder, label, or quick-filter selection."
            };
            list = list.push(
                container(text(empty_msg).size(12).color(muted()))
                    .padding(24)
                    .center_x(Fill),
            );
        } else {
            for email in &self.explorer_messages {
                list = list.push(self.message_row(email));
            }
        }

        container(
            column![header, scrollable(list).height(Fill)]
                .spacing(0)
                .height(Fill),
        )
        .width(Fill)
        .height(self.table_height)
        .style(reader_body_style)
        .into()
    }

    fn message_row<'a>(&'a self, email: &'a EmailRecord) -> Element<'a, ExplorerMessage> {
        let is_selected = self.selected_message_id == Some(email.id);
        let is_checked = self.checked_message_ids.contains(&email.id);
        let email_id = email.id;
        let subject = if email.subject.trim().is_empty() {
            "(No subject)"
        } else {
            email.subject.as_str()
        };
        let sender = if email.sender.trim().is_empty() {
            "(Unknown sender)"
        } else {
            email.sender.as_str()
        };
        let folder = if email.folder.trim().is_empty() {
            "Root"
        } else {
            email.folder.as_str()
        };
        let date_short = email
            .sent_at
            .as_deref()
            .map(short_date)
            .unwrap_or("Unknown");

        let mut label_row = row![].spacing(4).align_y(Alignment::Center);
        if email.labels.is_empty() {
            label_row = label_row.push(text("None").size(10).color(muted()));
        } else {
            for assigned in email.labels.iter().take(2) {
                let color = self.label_color(&assigned.label);
                let pct = assigned.confidence.map(|c| format!("{:.0}%", c * 100.0));
                label_row = label_row.push(label_badge(&assigned.label, color, pct));
            }
        }

        let range_btn: Element<'a, ExplorerMessage> = button(text("Range").size(9).color(muted()))
            .padding([2, 5])
            .style(button::text)
            .on_press(ExplorerMessage::SelectRangeTo(email_id))
            .into();

        let clickable_columns = button(
            row![
                text(truncate_str(subject, 54))
                    .size(12)
                    .color(if is_selected || is_checked {
                        on_primary()
                    } else {
                        bright_text()
                    })
                    .width(FillPortion(4)),
                container(label_row).width(FillPortion(3)),
                text(truncate_str(sender, 32))
                    .size(11)
                    .color(if is_selected { on_primary() } else { muted() })
                    .width(FillPortion(3)),
                text(truncate_str(folder, 20))
                    .size(11)
                    .color(muted())
                    .width(FillPortion(2)),
                text(date_short)
                    .size(11)
                    .color(muted())
                    .width(FillPortion(2)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .width(Fill)
        .padding(0)
        .style(button::text)
        .on_press(ExplorerMessage::SelectEmail(email_id));

        container(
            row![
                checkbox("", is_checked)
                    .on_toggle(move |v| ExplorerMessage::ToggleCheckEmail(email_id, v))
                    .size(14),
                range_btn,
                clickable_columns,
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .width(Fill)
        .padding([6, 12])
        .style(mail_row_style(is_selected, is_checked))
        .into()
    }

    fn reading_pane(&self) -> Element<'_, ExplorerMessage> {
        let Some(email) = self.selected_email() else {
            return container(
                column![
                    text("No message selected").size(16).color(bright_text()),
                    text(
                        "Select a message in the table above to read its content, or check multiple messages to label a selection."
                    )
                    .size(12)
                    .color(muted())
                ]
                .spacing(6)
                .align_x(Alignment::Center),
            )
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .style(reader_body_style)
            .into();
        };

        let subject = if email.subject.trim().is_empty() {
            "(No subject)"
        } else {
            email.subject.as_str()
        };
        let source_name = std::path::Path::new(&email.source_key)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&email.source_key);

        let mut assigned_row = row![text("Labels:").size(11).color(muted())]
            .spacing(6)
            .align_y(Alignment::Center);
        if email.labels.is_empty() {
            assigned_row = assigned_row.push(text("None assigned").size(11).color(muted()));
        } else {
            for assigned in &email.labels {
                let detail = match assigned.confidence {
                    Some(c) => format!("{} {:.0}%", assigned.source, c * 100.0),
                    None => assigned.source.clone(),
                };
                assigned_row = assigned_row.push(label_badge(
                    &assigned.label,
                    self.label_color(&assigned.label),
                    Some(detail),
                ));
            }
        }

        let mut quick_toggle_row = row![text("Toggle:").size(10).color(muted())]
            .spacing(5)
            .align_y(Alignment::Center);
        for (def, _) in &self.labels_with_counts {
            let active = email.labels.iter().any(|l| l.label == def.name);
            let btn_label = if active {
                format!("Remove {}", def.name)
            } else {
                format!("Add {}", def.name)
            };
            let msg_id = email.id;
            let label_name = def.name.clone();
            quick_toggle_row = quick_toggle_row.push(
                button(text(btn_label).size(10))
                    .padding([3, 8])
                    .style(if active {
                        button::primary
                    } else {
                        button::secondary
                    })
                    .on_press(ExplorerMessage::ToggleManualLabel(msg_id, label_name)),
            );
        }

        let header_content = column![
            row![
                text(subject).size(17).color(bright_text()).width(Fill),
                small_button(
                    "Label with Laya",
                    (!self.busy)
                        .then_some(ExplorerMessage::LabelSelectedWith(LabelerBackend::Laya))
                ),
                small_button(
                    "Label with LLM Judge",
                    (!self.busy)
                        .then_some(ExplorerMessage::LabelSelectedWith(LabelerBackend::LlmJudge))
                ),
                small_button(
                    "Ask Chat about this mail",
                    Some(ExplorerMessage::AskAboutEmail(email.id))
                ),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
            row![
                text(format!("From: {}", email.sender))
                    .size(12)
                    .color(bright_text()),
                Space::new(16, 0),
                text(format!("To: {}", email.recipients))
                    .size(11)
                    .color(muted()),
                Space::new(Fill, 0),
                text(format!(
                    "{} - {} / {}",
                    email.sent_at.as_deref().unwrap_or("Unknown date"),
                    source_name,
                    email.folder
                ))
                .size(11)
                .color(muted()),
            ]
            .align_y(Alignment::Center),
            horizontal_rule(1),
            row![
                container(assigned_row.wrap()).width(FillPortion(1)),
                container(quick_toggle_row.wrap()).width(FillPortion(1)),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        ]
        .spacing(6);

        // When the user has shrunk the header, or it exceeds the max height,
        // its contents scroll inside it.
        let (outer_height, inner_height) = match self.reader_header_height {
            Some(height) => (Length::Fixed(height.min(super::HEADER_MAX_HEIGHT)), Fill),
            None => (Length::Shrink, Length::Shrink),
        };
        let header = container(
            scrollable(container(header_content).padding([10, 14]).width(Fill))
                .height(inner_height),
        )
        .id(reader_header_id())
        .width(Fill)
        .height(outer_height)
        .max_height(super::HEADER_MAX_HEIGHT)
        .style(reader_header_style);

        let body_text = if email.body.trim().is_empty() {
            "(Empty message body)"
        } else {
            email.body.as_str()
        };

        let body = container(
            scrollable(
                container(
                    text(body_text)
                        .size(13)
                        .line_height(1.5)
                        .color(bright_text()),
                )
                .padding([12, 16])
                .width(Fill),
            )
            .height(Fill),
        )
        .width(Fill)
        .height(Fill)
        .style(reader_body_style);

        column![
            header,
            splitter(
                SplitAxis::Horizontal,
                self.is_resizing(super::Splitter::HeaderHeight),
                ExplorerMessage::ReaderHeaderResizeStart,
            ),
            body
        ]
        .width(Fill)
        .height(Fill)
        .into()
    }
}

fn short_date(iso: &str) -> &str {
    iso.get(0..16).unwrap_or(iso)
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
