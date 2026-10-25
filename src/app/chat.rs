use dioxus::prelude::*;

use crate::model::SearchHit;

use super::{
    ChatMessage, ChatRole, DispatchMessage, PstRagApp, markdown::render_markdown_blocks, theme::*,
};

#[derive(Debug, Clone)]
pub(super) enum ChatPaneMessage {
    ToggleChatPane,
    NewChat,
    QuestionChanged(String),
    Ask,
    SearchLocally,
    UsePrompt(&'static str),
    ToggleContext(usize, bool),
    ToggleSources,
    ToggleHit(usize),
    SelectEmail(i64),
    MarkdownLinkClicked(String),
}

pub(super) const fn chat_question_input_id() -> &'static str {
    "chat-question-input"
}

fn parse_link_message(url: &str) -> ChatPaneMessage {
    let trimmed = url.trim();
    if let Some(rest) = trimmed
        .strip_prefix("mail://")
        .or_else(|| trimmed.strip_prefix("#mail-"))
        .or_else(|| trimmed.strip_prefix('#'))
        && let Ok(id) = rest.trim().parse::<i64>()
    {
        return ChatPaneMessage::SelectEmail(id);
    }
    ChatPaneMessage::MarkdownLinkClicked(trimmed.to_owned())
}

impl PstRagApp {
    pub(super) fn chat_pane(&self) -> Element {
        let tr = self.config.language;
        let accent = accent_text();
        let muted_color = muted();
        let bright = bright_text();
        let cp_style = chat_pane_style();

        if !self.chat_open {
            let open_lbl = tr.btn_open();
            return rsx! {
                div {
                    style: "width: 36px; height: 100%; flex-shrink: 0; {cp_style}",
                    button {
                        class: "btn-text",
                        style: "width: 100%; height: 100%; padding: 16px 4px; display: flex; flex-direction: column; align-items: center; justify-content: flex-start; gap: 2px;",
                        onclick: move |_| ChatPaneMessage::ToggleChatPane.dispatch(),
                        span { style: "font-size: 10px; color: {accent};", "{open_lbl}" }
                        div { style: "height: 8px;" }
                        span { style: "font-size: 10px; color: {muted_color};", "C" }
                        span { style: "font-size: 10px; color: {muted_color};", "H" }
                        span { style: "font-size: 10px; color: {muted_color};", "A" }
                        span { style: "font-size: 10px; color: {muted_color};", "T" }
                    }
                }
            };
        }

        let chat_title = tr.chat_title();
        let chat_sub = tr.chat_subtitle();
        let new_btn = small_button(tr.btn_new(), Some(ChatPaneMessage::NewChat));
        let hide_btn = small_button(tr.btn_hide(), Some(ChatPaneMessage::ToggleChatPane));

        let header = rsx! {
            div {
                style: "display: flex; flex-direction: row; align-items: center; gap: 5px; flex-shrink: 0;",
                div {
                    style: "flex: 1; display: flex; flex-direction: column; gap: 2px;",
                    span { style: "font-size: 17px; color: {bright}; font-weight: 600;", "{chat_title}" }
                    span { style: "font-size: 11px; color: {muted_color};", "{chat_sub}" }
                }
                {new_btn}
                {hide_btn}
            }
        };

        let welcome_section = if self.messages.is_empty() {
            let w_title = tr.chat_welcome_title();
            let w_desc = tr.chat_welcome_desc();
            let p1 = prompt_button(
                tr.prompt_summary_label(),
                ChatPaneMessage::UsePrompt(tr.prompt_summary_query()),
            );
            let p2 = prompt_button(
                tr.prompt_project_label(),
                ChatPaneMessage::UsePrompt(tr.prompt_project_query()),
            );
            let p3 = prompt_button(
                tr.prompt_followup_label(),
                ChatPaneMessage::UsePrompt(tr.prompt_followup_query()),
            );
            Some(rsx! {
                div {
                    style: "padding: 20px 6px; display: flex; flex-direction: column; gap: 10px;",
                    span { style: "font-size: 16px; color: {bright}; font-weight: 600;", "{w_title}" }
                    span { style: "font-size: 11px; color: {muted_color};", "{w_desc}" }
                    div {
                        style: "width: 100%; display: flex; flex-direction: column; gap: 6px;",
                        {p1}
                        {p2}
                        {p3}
                    }
                }
            })
        } else {
            None
        };

        let message_elements: Vec<Element> = self
            .messages
            .iter()
            .enumerate()
            .map(|(idx, msg)| self.message_view(idx, msg))
            .collect();

        let sources_section = self.sources_view();

        let enabled = !self.busy && self.count > 0 && !self.question.trim().is_empty();
        let q_id = chat_question_input_id();
        let q_ph = tr.chat_input_placeholder();
        let q_val = self.question.clone();
        let helper = if self.count == 0 {
            tr.chat_helper_need_index()
        } else if self.busy {
            tr.chat_helper_busy()
        } else {
            tr.chat_helper_ready()
        };
        let search_only_btn = enabled_button(
            tr.btn_search_only(),
            enabled.then_some(ChatPaneMessage::SearchLocally),
        );
        let send_btn = primary_button(tr.btn_send(), enabled.then_some(ChatPaneMessage::Ask));
        let comp_style = composer_style();
        let chat_width = self.chat_width;

        rsx! {
            div {
                style: "width: {chat_width}px; height: 100%; padding: 14px; display: flex; flex-direction: column; gap: 10px; flex-shrink: 0; overflow: hidden; {cp_style}",
                {header}
                hr {}
                div {
                    style: "flex: 1; min-height: 0; width: 100%; overflow-y: auto; padding: 6px 2px; display: flex; flex-direction: column; gap: 12px;",
                    if let Some(welcome) = welcome_section {
                        {welcome}
                    }
                    for me in message_elements {
                        {me}
                    }
                    {sources_section}
                }
                div {
                    style: "padding: 10px; display: flex; flex-direction: column; gap: 8px; flex-shrink: 0; {comp_style}",
                    input {
                        id: "{q_id}",
                        r#type: "text",
                        placeholder: "{q_ph}",
                        value: "{q_val}",
                        style: "width: 100%; padding: 10px; font-size: 13px;",
                        oninput: move |evt| ChatPaneMessage::QuestionChanged(evt.value()).dispatch(),
                        onkeydown: move |evt| {
                            if evt.key() != Key::Escape {
                                evt.stop_propagation();
                            }
                            if evt.key() == Key::Enter && enabled {
                                ChatPaneMessage::Ask.dispatch();
                            }
                        }
                    }
                    div {
                        style: "display: flex; flex-direction: row; align-items: center; gap: 6px;",
                        span {
                            style: "flex: 1; font-size: 10px; color: {muted_color};",
                            "{helper}"
                        }
                        {search_only_btn}
                        {send_btn}
                    }
                }
            }
        }
    }

    fn sources_view(&self) -> Element {
        if self.hits.is_empty() {
            return rsx! {};
        }
        let tr = self.config.language;
        let accent = accent_text();
        let muted_color = muted();
        let b_style = badge_style();
        let label = if self.show_sources {
            tr.btn_hide_evidence()
        } else {
            tr.btn_show_evidence()
        };
        let ev_heading = tr.evidence_heading();
        let hits_len = self.hits.len();
        let show_sources = self.show_sources;
        let hit_elements: Vec<Element> = if show_sources {
            self.hits
                .iter()
                .enumerate()
                .map(|(idx, hit)| self.hit_view(idx, hit))
                .collect()
        } else {
            Vec::new()
        };

        rsx! {
            div {
                style: "width: 100%; display: flex; flex-direction: column; gap: 8px; padding-top: 4px;",
                button {
                    class: "btn-secondary",
                    style: "width: 100%; padding: 8px 10px; display: flex; flex-direction: row; align-items: center; gap: 8px;",
                    onclick: move |_| ChatPaneMessage::ToggleSources.dispatch(),
                    span { style: "font-size: 10px; color: {accent};", "{ev_heading}" }
                    span {
                        style: "padding: 2px 6px; font-size: 10px; {b_style}",
                        "{hits_len}"
                    }
                    div { style: "flex: 1;" }
                    span { style: "font-size: 10px; color: {muted_color};", "{label}" }
                }
                for he in hit_elements {
                    {he}
                }
            }
        }
    }

    fn message_view(&self, index: usize, message: &ChatMessage) -> Element {
        let tr = self.config.language;
        let is_user = matches!(message.role, ChatRole::User);
        let av_style = if is_user {
            user_avatar_style()
        } else {
            assistant_avatar_style()
        };
        let av_letter = if is_user { "Y" } else { "D" };
        let avatar = rsx! {
            div {
                style: "width: 26px; height: 26px; display: flex; align-items: center; justify-content: center; flex-shrink: 0; {av_style}",
                span { style: "font-size: 10px; font-weight: 600;", "{av_letter}" }
            }
        };

        let badge_text = if is_user {
            tr.user_badge()
        } else {
            tr.assistant_badge()
        };
        let badge_color = if is_user { on_primary() } else { accent_text() };
        let md_blocks = render_markdown_blocks(&message.blocks, is_user, parse_link_message);

        let has_context = !message.context_preview.is_empty();
        let show_ctx_lbl = tr.show_context_checkbox();
        let show_ctx = message.show_context;
        let ctx_preview = message.context_preview.clone();
        let muted_color = muted();
        let cb_style = code_block_style();
        let bub_style = if is_user {
            user_bubble_style()
        } else {
            assistant_bubble_style()
        };

        let bubble = rsx! {
            div {
                style: "flex: 1; min-width: 0; padding: 10px 12px; display: flex; flex-direction: column; gap: 6px; {bub_style}",
                span {
                    style: "font-size: 9px; color: {badge_color}; font-weight: 600;",
                    "{badge_text}"
                }
                {md_blocks}
                if has_context {
                    hr {}
                    label {
                        style: "display: flex; flex-direction: row; align-items: center; gap: 6px; font-size: 11px; cursor: pointer;",
                        input {
                            r#type: "checkbox",
                            checked: show_ctx,
                            onchange: move |evt| ChatPaneMessage::ToggleContext(index, evt.checked()).dispatch()
                        }
                        span { "{show_ctx_lbl}" }
                    }
                    if show_ctx {
                        div {
                            class: "selectable-text",
                            style: "width: 100%; padding: 6px 8px; font-size: 10px; color: {muted_color}; white-space: pre-wrap; word-break: break-word; {cb_style}",
                            "{ctx_preview}"
                        }
                    }
                }
            }
        };

        match message.role {
            ChatRole::User => rsx! {
                div {
                    style: "width: 100%; display: flex; flex-direction: row; align-items: flex-end; gap: 8px;",
                    {bubble}
                    {avatar}
                }
            },
            ChatRole::Assistant => rsx! {
                div {
                    style: "width: 100%; display: flex; flex-direction: row; align-items: flex-start; gap: 8px;",
                    {avatar}
                    {bubble}
                }
            },
        }
    }

    fn hit_view(&self, index: usize, hit: &SearchHit) -> Element {
        let tr = self.config.language;
        let accent = accent_text();
        let bright = bright_text();
        let muted_color = muted();

        let subject = if hit.subject.is_empty() {
            tr.no_subject().to_owned()
        } else {
            hit.subject.clone()
        };
        let source_name = std::path::Path::new(&hit.source_key)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&hit.source_key);
        let idx_str = format!("{:02}", index + 1);
        let sender_date = format!(
            "{} - {}",
            hit.sender,
            hit.sent_at
                .as_deref()
                .unwrap_or_else(|| tr.unknown_date_full())
        );
        let open_mail_btn = small_button(
            tr.btn_open_mail(),
            Some(ChatPaneMessage::SelectEmail(hit.id)),
        );
        let short_excerpt = hit.excerpt(240);
        let is_expanded = self.expanded_hit == Some(index);
        let details_btn = small_button(
            if is_expanded {
                tr.btn_hide_details()
            } else {
                tr.btn_metadata_and_excerpt()
            },
            Some(ChatPaneMessage::ToggleHit(index)),
        );

        let expanded_details = if is_expanded {
            let mut meta_text = format!(
                "PST: {} - Folder: {} - Score: {:.3} - ID: {}",
                source_name, hit.folder, hit.score, hit.id
            );
            if !hit.conversation_id.trim().is_empty() {
                meta_text.push_str(&format!(" - Thread: {}", hit.conversation_id.trim()));
            }
            let att_text = if !hit.attachment_names.trim().is_empty() {
                Some(tr.attachments_prefix(hit.attachment_names.trim()))
            } else {
                None
            };
            let lbl_text = if !hit.labels.is_empty() {
                let label_str = hit
                    .labels
                    .iter()
                    .map(|l| {
                        if let Some(conf) = l.confidence {
                            format!("{} ({:.0}%)", l.label, conf * 100.0)
                        } else {
                            l.label.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                Some(tr.labels_prefix(&label_str))
            } else {
                None
            };
            let long_excerpt = hit.excerpt(900);
            Some(rsx! {
                span { style: "font-size: 10px; color: {muted_color};", "{meta_text}" }
                if let Some(att) = att_text {
                    span { style: "font-size: 10px; color: {accent};", "{att}" }
                }
                if let Some(lbl) = lbl_text {
                    span { style: "font-size: 10px; color: {accent};", "{lbl}" }
                }
                hr {}
                span {
                    class: "selectable-text",
                    style: "font-size: 11px; white-space: pre-wrap; word-break: break-word;",
                    "{long_excerpt}"
                }
            })
        } else {
            None
        };

        let src_style = source_style();

        rsx! {
            div {
                style: "width: 100%; padding: 10px; display: flex; flex-direction: column; gap: 6px; {src_style}",
                div {
                    style: "display: flex; flex-direction: row; align-items: center; gap: 8px;",
                    span { style: "font-size: 11px; color: {accent};", "{idx_str}" }
                    div {
                        style: "flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px;",
                        span {
                            style: "font-size: 13px; color: {bright}; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;",
                            "{subject}"
                        }
                        span {
                            style: "font-size: 10px; color: {muted_color};",
                            "{sender_date}"
                        }
                    }
                    {open_mail_btn}
                }
                span {
                    class: "selectable-text",
                    style: "font-size: 11px; white-space: pre-wrap; word-break: break-word;",
                    "{short_excerpt}"
                }
                {details_btn}
                if let Some(details) = expanded_details {
                    {details}
                }
            }
        }
    }
}
