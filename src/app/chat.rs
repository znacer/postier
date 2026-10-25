use dioxus::prelude::*;
use dioxus_primitives::checkbox::CheckboxState;

use crate::{
    components::{
        avatar::{Avatar, AvatarFallback, AvatarImageSize},
        badge::{Badge, BadgeVariant},
        button::{Button, ButtonVariant},
        checkbox::Checkbox,
        collapsible::{Collapsible, CollapsibleContent, CollapsibleTrigger},
        input::Input,
        item::{
            Item, ItemActions, ItemContent, ItemDescription, ItemGroup, ItemHeader, ItemMedia,
            ItemSize, ItemTitle, ItemVariant,
        },
        label::Label,
        scroll_area::ScrollArea,
        separator::Separator,
    },
    model::SearchHit,
};

use super::{
    ChatMessage, ChatRole, DispatchMessage, PstRagApp,
    email_html::{format_email_datetime, parse_contact},
    markdown::render_markdown_blocks,
    theme::*,
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
                aside {
                    style: "width: 42px; height: 100%; flex-shrink: 0; {cp_style}",
                    Button {
                        variant: ButtonVariant::Ghost,
                        title: "{open_lbl}",
                        style: "width: 100%; height: 100%; padding: 14px 4px; display: flex; flex-direction: column; align-items: center; justify-content: flex-start; gap: 10px; border-radius: 0;",
                        onclick: move |_| ChatPaneMessage::ToggleChatPane.dispatch(),
                        svg {
                            width: "16",
                            height: "16",
                            view_box: "0 0 24 24",
                            fill: "none",
                            stroke: "{accent}",
                            stroke_width: "2",
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                            path { d: "M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" }
                        }
                        span {
                            style: "writing-mode: vertical-rl; text-orientation: mixed; font-size: 10px; font-weight: 600; letter-spacing: 0.12em; color: {muted_color}; text-transform: uppercase;",
                            "CHAT"
                        }
                    }
                }
            };
        }

        let chat_title = tr.chat_title();
        let chat_sub = tr.chat_subtitle();
        let new_btn = small_button(tr.btn_new(), Some(ChatPaneMessage::NewChat));
        let hide_btn = small_button(tr.btn_hide(), Some(ChatPaneMessage::ToggleChatPane));

        let header = rsx! {
            header {
                style: "display: flex; flex-direction: row; align-items: center; gap: 8px; flex-shrink: 0;",
                div {
                    style: "flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px;",
                    span { style: "font-size: 15px; color: {bright}; font-weight: 700; letter-spacing: -0.01em;", "{chat_title}" }
                    span { style: "font-size: 11px; color: {muted_color}; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;", "{chat_sub}" }
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
                    style: "padding: 20px 8px; display: flex; flex-direction: column; gap: 12px;",
                    div {
                        style: "width: 36px; height: 36px; border-radius: 10px; background-color: rgba(59, 130, 246, 0.14); border: 1px solid rgba(96, 165, 250, 0.3); display: flex; align-items: center; justify-content: center;",
                        svg {
                            width: "18",
                            height: "18",
                            view_box: "0 0 24 24",
                            fill: "none",
                            stroke: "{accent}",
                            stroke_width: "2",
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                            path { d: "M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" }
                        }
                    }
                    div {
                        style: "display: flex; flex-direction: column; gap: 4px;",
                        span { style: "font-size: 15px; color: {bright}; font-weight: 700;", "{w_title}" }
                        span { style: "font-size: 12px; color: {muted_color}; line-height: 1.45;", "{w_desc}" }
                    }
                    div {
                        style: "width: 100%; display: flex; flex-direction: column; gap: 8px; margin-top: 4px;",
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
        let search_only_btn = small_button(
            tr.btn_search_only(),
            enabled.then_some(ChatPaneMessage::SearchLocally),
        );
        let send_btn = primary_button(tr.btn_send(), enabled.then_some(ChatPaneMessage::Ask));
        let comp_style = composer_style();
        let chat_width = self.chat_width;

        rsx! {
            aside {
                style: "width: {chat_width}px; height: 100%; padding: 14px; display: flex; flex-direction: column; gap: 10px; flex-shrink: 0; overflow: hidden; {cp_style}",
                {header}
                Separator {}
                ScrollArea {
                    style: "flex: 1; min-height: 0; width: 100%;",
                    div {
                        style: "padding: 6px 2px; display: flex; flex-direction: column; gap: 12px;",
                        if let Some(welcome) = welcome_section {
                            {welcome}
                        }
                        for me in message_elements {
                            {me}
                        }
                        {sources_section}
                    }
                }
                div {
                    style: "padding: 12px; display: flex; flex-direction: column; gap: 10px; flex-shrink: 0; {comp_style}",
                    Input {
                        id: "{q_id}",
                        r#type: "text",
                        placeholder: "{q_ph}",
                        value: "{q_val}",
                        style: "width: 100%; padding: 9px 12px; font-size: 13px;",
                        oninput: move |evt: FormEvent| ChatPaneMessage::QuestionChanged(evt.value()).dispatch(),
                        onkeydown: move |evt: KeyboardEvent| {
                            if evt.key() != Key::Escape {
                                evt.stop_propagation();
                            }
                            if evt.key() == Key::Enter && enabled {
                                ChatPaneMessage::Ask.dispatch();
                            }
                        }
                    }
                    div {
                        style: "display: flex; flex-direction: row; align-items: center; gap: 8px;",
                        span {
                            style: "flex: 1; font-size: 11px; color: {muted_color}; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;",
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
        let label = if self.show_sources {
            tr.btn_hide_evidence()
        } else {
            tr.btn_show_evidence()
        };
        let ev_heading = tr.evidence_heading();
        let hits_len = self.hits.len();
        let show_sources = self.show_sources;
        let hit_elements: Vec<Element> = self
            .hits
            .iter()
            .enumerate()
            .map(|(idx, hit)| self.hit_view(idx, hit))
            .collect();

        rsx! {
            Collapsible {
                open: show_sources,
                on_open_change: move |_| ChatPaneMessage::ToggleSources.dispatch(),
                div {
                    style: "width: 100%; display: flex; flex-direction: column; gap: 8px; padding-top: 4px;",
                    CollapsibleTrigger {
                        as: move |_attrs: Vec<Attribute>| {
                            rsx! {
                                Button {
                                    variant: ButtonVariant::Outline,
                                    style: "width: 100%; height: auto; padding: 8px 12px; display: flex; flex-direction: row; align-items: center; gap: 8px; border-radius: var(--radius-md);",
                                    onclick: move |_| ChatPaneMessage::ToggleSources.dispatch(),
                                    span { style: "font-size: 11px; font-weight: 700; letter-spacing: 0.04em; color: {accent};", "{ev_heading}" }
                                    Badge {
                                        variant: BadgeVariant::Secondary,
                                        "{hits_len}"
                                    }
                                    div { style: "flex: 1;" }
                                    span { style: "font-size: 11px; color: {muted_color};", "{label}" }
                                }
                            }
                        },
                    }
                    CollapsibleContent {
                        ItemGroup {
                            style: "gap: 8px;",
                            for he in hit_elements {
                                {he}
                            }
                        }
                    }
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
        let av_letter = if is_user { "Y" } else { "AI" };
        let avatar = rsx! {
            Avatar {
                size: AvatarImageSize::Small,
                style: "width: 28px; height: 28px; flex-shrink: 0;",
                AvatarFallback {
                    style: "font-size: 10px; font-weight: 700; {av_style}",
                    "{av_letter}"
                }
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
        let show_ctx_state = if show_ctx {
            CheckboxState::Checked
        } else {
            CheckboxState::Unchecked
        };
        let ctx_preview = message.context_preview.clone();
        let muted_color = muted();
        let cb_style = code_block_style();
        let bub_style = if is_user {
            user_bubble_style()
        } else {
            assistant_bubble_style()
        };
        let ctx_checkbox_id = format!("chat-ctx-check-{index}");

        let bubble = rsx! {
            div {
                style: "flex: 1; min-width: 0; padding: 11px 14px; display: flex; flex-direction: column; gap: 6px; {bub_style}",
                span {
                    style: "font-size: 10px; color: {badge_color}; font-weight: 700; letter-spacing: 0.04em; text-transform: uppercase;",
                    "{badge_text}"
                }
                {md_blocks}
                if has_context {
                    Separator {}
                    div {
                        style: "display: flex; flex-direction: row; align-items: center; gap: 8px; padding-top: 2px;",
                        Checkbox {
                            id: "{ctx_checkbox_id}",
                            checked: show_ctx_state,
                            on_checked_change: move |state: CheckboxState| {
                                ChatPaneMessage::ToggleContext(index, matches!(state, CheckboxState::Checked)).dispatch();
                            }
                        }
                        Label {
                            html_for: "{ctx_checkbox_id}",
                            style: "font-size: 11px; cursor: pointer;",
                            "{show_ctx_lbl}"
                        }
                    }
                    if show_ctx {
                        div {
                            class: "selectable-text",
                            style: "width: 100%; padding: 8px 10px; font-size: 10.5px; font-family: var(--font-mono); color: {muted_color}; white-space: pre-wrap; word-break: break-word; max-height: 220px; overflow-y: auto; {cb_style}",
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
        let sender_display = if hit.sender.trim().is_empty() {
            tr.unknown_sender().to_owned()
        } else {
            parse_contact(&hit.sender).display_primary().to_owned()
        };
        let formatted_date = format_email_datetime(hit.sent_at.as_deref(), tr)
            .map(|dt| dt.full)
            .unwrap_or_else(|| tr.unknown_date_full().to_owned());
        let sender_date = format!("{sender_display} · {formatted_date}");
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
                "PST: {} · Folder: {} · Score: {:.3} · ID: {}",
                source_name, hit.folder, hit.score, hit.id
            );
            if !hit.conversation_id.trim().is_empty() {
                meta_text.push_str(&format!(" · Thread: {}", hit.conversation_id.trim()));
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
                span { style: "font-size: 10.5px; color: {muted_color}; font-family: var(--font-mono);", "{meta_text}" }
                if let Some(att) = att_text {
                    span { style: "font-size: 11px; color: {accent};", "{att}" }
                }
                if let Some(lbl) = lbl_text {
                    span { style: "font-size: 11px; color: {accent};", "{lbl}" }
                }
                Separator {}
                span {
                    class: "selectable-text",
                    style: "font-size: 11.5px; line-height: 1.5; white-space: pre-wrap; word-break: break-word;",
                    "{long_excerpt}"
                }
            })
        } else {
            None
        };

        let src_style = source_style();

        rsx! {
            Item {
                variant: ItemVariant::Outline,
                size: ItemSize::Sm,
                style: "width: 100%; padding: 12px; display: flex; flex-direction: column; align-items: stretch; gap: 8px; {src_style}",
                ItemHeader {
                    style: "display: flex; flex-direction: row; align-items: center; gap: 8px;",
                    ItemMedia {
                        Badge {
                            variant: BadgeVariant::Secondary,
                            "{idx_str}"
                        }
                    }
                    ItemContent {
                        style: "flex: 1; min-width: 0; gap: 2px;",
                        ItemTitle {
                            style: "width: 100%; font-size: 12.5px; font-weight: 600; color: {bright}; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;",
                            "{subject}"
                        }
                        ItemDescription {
                            style: "font-size: 11px; color: {muted_color}; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; -webkit-line-clamp: 1;",
                            "{sender_date}"
                        }
                    }
                    ItemActions {
                        {open_mail_btn}
                    }
                }
                span {
                    class: "selectable-text",
                    style: "font-size: 11.5px; line-height: 1.45; white-space: pre-wrap; word-break: break-word;",
                    "{short_excerpt}"
                }
                div {
                    style: "display: flex; flex-direction: row;",
                    {details_btn}
                }
                if let Some(details) = expanded_details {
                    {details}
                }
            }
        }
    }
}
