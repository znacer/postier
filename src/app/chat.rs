use crate::model::SearchHit;
use iced::{
    Alignment, Element, Fill,
    widget::{
        Space, button, checkbox, column, container, horizontal_rule, row, scrollable, slider, text,
        text_input,
    },
};

use super::{ChatMessage, ChatRole, Message, PstRagApp, markdown::render_markdown, theme::*};

impl PstRagApp {
    pub(super) fn chat_pane(&self) -> Element<'_, Message> {
        if !self.chat_open {
            let expand_rail = button(
                column![
                    text("Open").size(10).color(accent_text()),
                    Space::new(0, 8),
                    text("C").size(10).color(muted()),
                    text("H").size(10).color(muted()),
                    text("A").size(10).color(muted()),
                    text("T").size(10).color(muted()),
                ]
                .spacing(2)
                .align_x(Alignment::Center),
            )
            .width(Fill)
            .height(Fill)
            .padding([16, 4])
            .style(button::text)
            .on_press(Message::ToggleChatPane);

            return container(expand_rail)
                .width(36)
                .height(Fill)
                .style(chat_pane_style)
                .into();
        }

        let header = column![
            row![
                column![
                    text("Archive Chat").size(17).color(bright_text()),
                    text("Grounded RAG assistant - Markdown enabled")
                        .size(11)
                        .color(muted())
                ]
                .spacing(2)
                .width(Fill),
                small_button(
                    "Wider",
                    (self.chat_width < 740.0).then_some(Message::ChatWidthChanged(
                        (self.chat_width + 80.0).min(760.0)
                    ))
                ),
                small_button(
                    "Narrower",
                    (self.chat_width > 330.0).then_some(Message::ChatWidthChanged(
                        (self.chat_width - 80.0).max(310.0)
                    ))
                ),
                small_button("New", Some(Message::NewChat)),
                small_button("Hide", Some(Message::ToggleChatPane)),
            ]
            .spacing(5)
            .align_y(Alignment::Center),
            row![
                text("Pane width").size(10).color(muted()),
                slider(310.0..=760.0, self.chat_width, Message::ChatWidthChanged).height(14),
                text(format!("{:.0}px", self.chat_width))
                    .size(10)
                    .color(muted())
            ]
            .spacing(8)
            .align_y(Alignment::Center)
        ]
        .spacing(6);

        let mut conversation = column![].spacing(12).width(Fill);
        if self.messages.is_empty() {
            let prompts = column![
                prompt_button(
                    "Summarize the latest decisions",
                    "Summarize the latest decisions in my archive"
                ),
                prompt_button(
                    "Find messages about a project",
                    "Find the latest messages about my current projects"
                ),
                prompt_button(
                    "What needs a follow-up?",
                    "Which recent messages need a follow-up?"
                )
            ]
            .spacing(6)
            .width(Fill);
            conversation = conversation.push(
                container(
                    column![
                        text("Ask your archive").size(16).color(bright_text()),
                        text("Ask about decisions, people, dates, or projects. Only selected evidence is sent to your configured LLM.")
                            .size(11)
                            .color(muted()),
                        prompts
                    ]
                    .spacing(10),
                )
                .padding([20, 6]),
            );
        }
        for (index, message) in self.messages.iter().enumerate() {
            conversation = conversation.push(self.message_view(index, message));
        }
        conversation = self.sources_view(conversation);

        let enabled = !self.busy && self.count > 0 && !self.question.trim().is_empty();
        let mut question_input = text_input("Ask your archive...", &self.question)
            .on_input(Message::QuestionChanged)
            .padding(10)
            .size(13);
        if enabled {
            question_input = question_input.on_submit(Message::Ask);
        }
        let helper = if self.count == 0 {
            "Index an archive to start chatting"
        } else if self.busy {
            "Working with your local archive..."
        } else {
            "Enter to send - Search only stays local"
        };
        let composer = container(
            column![
                question_input,
                row![
                    text(helper).size(10).color(muted()).width(Fill),
                    enabled_button("Search only", enabled.then_some(Message::SearchLocally)),
                    primary_button("Send", enabled.then_some(Message::Ask))
                ]
                .spacing(6)
                .align_y(Alignment::Center)
            ]
            .spacing(8),
        )
        .padding(10)
        .style(composer_style);

        container(
            column![
                header,
                horizontal_rule(1),
                scrollable(container(conversation).padding([6, 2])).height(Fill),
                composer
            ]
            .spacing(10)
            .padding(14),
        )
        .width(self.chat_width)
        .height(Fill)
        .style(chat_pane_style)
        .into()
    }

    fn sources_view<'a>(
        &'a self,
        mut conversation: iced::widget::Column<'a, Message>,
    ) -> iced::widget::Column<'a, Message> {
        if self.hits.is_empty() {
            return conversation;
        }
        let label = if self.show_sources {
            "Hide evidence"
        } else {
            "Show evidence"
        };
        conversation = conversation.push(Space::new(0, 4)).push(
            button(
                row![
                    text("EVIDENCE").size(10).color(accent_text()),
                    container(text(self.hits.len()).size(10))
                        .padding([2, 6])
                        .style(badge_style),
                    Space::new(Fill, 0),
                    text(label).size(10).color(muted())
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .width(Fill)
            .padding([8, 10])
            .style(button::secondary)
            .on_press(Message::ToggleSources),
        );
        if self.show_sources {
            for (index, hit) in self.hits.iter().enumerate() {
                conversation = conversation.push(self.hit_view(index, hit));
            }
        }
        conversation
    }

    fn message_view<'a>(&'a self, index: usize, message: &'a ChatMessage) -> Element<'a, Message> {
        let is_user = matches!(message.role, ChatRole::User);
        let avatar = container(text(if is_user { "Y" } else { "D" }).size(10))
            .width(26)
            .height(26)
            .center_x(26)
            .center_y(26)
            .style(if is_user {
                user_avatar_style
            } else {
                assistant_avatar_style
            });
        let mut contents = column![
            text(if is_user { "YOU" } else { "DUNGBEETLE" })
                .size(9)
                .color(if is_user { on_primary() } else { accent_text() }),
            render_markdown(&message.content, is_user)
        ]
        .spacing(6);
        if !message.context_preview.is_empty() {
            contents = contents.push(horizontal_rule(1)).push(
                checkbox("Show context sent to LLM", message.show_context)
                    .size(12)
                    .text_size(11)
                    .on_toggle(move |value| Message::ToggleContext(index, value)),
            );
            if message.show_context {
                contents = contents.push(
                    container(text(&message.context_preview).size(10).color(muted()))
                        .width(Fill)
                        .padding([6, 8])
                        .style(code_block_style),
                );
            }
        }
        let bubble = container(contents)
            .width(Fill)
            .padding([10, 12])
            .style(if is_user {
                user_bubble_style
            } else {
                assistant_bubble_style
            });
        match message.role {
            ChatRole::User => row![bubble, avatar]
                .spacing(8)
                .align_y(Alignment::End)
                .into(),
            ChatRole::Assistant => row![avatar, bubble]
                .spacing(8)
                .align_y(Alignment::Start)
                .into(),
        }
    }

    fn hit_view<'a>(&'a self, index: usize, hit: &'a SearchHit) -> Element<'a, Message> {
        let subject = if hit.subject.is_empty() {
            "(No subject)"
        } else {
            &hit.subject
        };
        let source_name = std::path::Path::new(&hit.source_key)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&hit.source_key);
        let mut contents = column![
            row![
                text(format!("{:02}", index + 1))
                    .size(11)
                    .color(accent_text()),
                column![
                    text(subject).size(13).color(bright_text()),
                    text(format!(
                        "{} - {}",
                        hit.sender,
                        hit.sent_at.as_deref().unwrap_or("Unknown date")
                    ))
                    .size(10)
                    .color(muted())
                ]
                .spacing(2)
                .width(Fill),
                small_button("Open Mail", Some(Message::SelectEmail(hit.id))),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            text(hit.excerpt(240)).size(11),
            small_button(
                if self.expanded_hit == Some(index) {
                    "Hide details"
                } else {
                    "Metadata and full excerpt"
                },
                Some(Message::ToggleHit(index))
            )
        ]
        .spacing(6);
        if self.expanded_hit == Some(index) {
            let mut meta_text = format!(
                "PST: {} - Folder: {} - Score: {:.3} - ID: {}",
                source_name, hit.folder, hit.score, hit.id
            );
            if !hit.conversation_id.trim().is_empty() {
                meta_text.push_str(&format!(" - Thread: {}", hit.conversation_id.trim()));
            }
            contents = contents.push(text(meta_text).size(10).color(muted()));
            if !hit.attachment_names.trim().is_empty() {
                contents = contents.push(
                    text(format!("Attachments: {}", hit.attachment_names.trim()))
                        .size(10)
                        .color(accent_text()),
                );
            }
            if !hit.labels.is_empty() {
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
                contents = contents.push(
                    text(format!("Labels: {}", label_str))
                        .size(10)
                        .color(accent_text()),
                );
            }
            contents = contents
                .push(horizontal_rule(1))
                .push(text(hit.excerpt(900)).size(11));
        }
        container(contents)
            .width(Fill)
            .padding(10)
            .style(source_style)
            .into()
    }
}
