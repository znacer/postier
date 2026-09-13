use crate::model::SearchHit;
use iced::{
    Alignment, Element, Fill,
    widget::{
        Space, button, checkbox, column, container, horizontal_rule, row, scrollable, text,
        text_input,
    },
};

use super::{ChatMessage, ChatRole, Message, PstRagApp, theme::*};

impl PstRagApp {
    pub(super) fn content(&self) -> Element<'_, Message> {
        let header = row![
            column![
                text("Chat with your archive").size(27),
                text("Answers grounded in your indexed email.")
                    .size(13)
                    .color(muted())
            ]
            .spacing(4)
            .width(Fill),
            enabled_button("New chat", (!self.busy).then_some(Message::NewChat))
        ]
        .align_y(Alignment::Center);
        let status = container(
            row![text(&self.status).size(11).color(muted())]
                .spacing(7)
                .align_y(Alignment::Center),
        )
        .padding([6, 10])
        .style(status_style);

        let mut conversation = column![].spacing(14).width(Fill);
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
            .spacing(8)
            .width(420);
            conversation = conversation.push(container(column![
                text("What would you like to find?").size(22),
                text("Ask about decisions, people, dates, or projects. Only selected evidence is sent to your configured LLM.").size(13).color(muted()).width(480),
                prompts
            ].spacing(12).align_x(Alignment::Center)).padding([54, 10]).center_x(Fill));
        }
        for (index, message) in self.messages.iter().enumerate() {
            conversation = conversation.push(self.message_view(index, message));
        }
        conversation = self.sources_view(conversation);

        let enabled = !self.busy && self.count > 0 && !self.question.trim().is_empty();
        let mut question_input = text_input("Message your archive…", &self.question)
            .on_input(Message::QuestionChanged)
            .padding(12);
        if enabled {
            question_input = question_input.on_submit(Message::Ask);
        }
        let helper = if self.count == 0 {
            "Index an archive to start chatting"
        } else if self.busy {
            "Working with your local archive…"
        } else {
            "Enter to send  ·  Search only never contacts the LLM"
        };
        let composer = container(
            column![
                question_input,
                row![
                    text(helper).size(10).color(muted()).width(Fill),
                    enabled_button("Search only", enabled.then_some(Message::SearchLocally)),
                    primary_button("Send", enabled.then_some(Message::Ask))
                ]
                .spacing(8)
                .align_y(Alignment::Center)
            ]
            .spacing(9),
        )
        .padding(12)
        .style(composer_style);

        container(
            column![
                header,
                status,
                scrollable(container(conversation).padding([8, 4])).height(Fill),
                composer
            ]
            .spacing(14)
            .padding([24, 28]),
        )
        .width(Fill)
        .height(Fill)
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
            "Hide evidence  ▴"
        } else {
            "Show evidence  ▾"
        };
        conversation = conversation.push(Space::new(0, 4)).push(
            button(
                row![
                    text("EVIDENCE").size(11).color(primary()),
                    container(text(self.hits.len()).size(10))
                        .padding([2, 7])
                        .style(badge_style),
                    Space::new(Fill, 0),
                    text(label).size(11).color(muted())
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .width(Fill)
            .padding([10, 12])
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
        let avatar = container(text(if is_user { "Y" } else { "D" }).size(11))
            .width(30)
            .height(30)
            .center_x(30)
            .center_y(30)
            .style(if is_user {
                user_avatar_style
            } else {
                assistant_avatar_style
            });
        let mut contents = column![
            text(if is_user { "YOU" } else { "DUNGBEETLE" })
                .size(10)
                .color(if is_user { on_primary() } else { primary() }),
            text(&message.content).size(14).line_height(1.45)
        ]
        .spacing(7);
        if !message.context_preview.is_empty() {
            contents = contents.push(horizontal_rule(1)).push(
                checkbox("Show context sent to the LLM", message.show_context)
                    .on_toggle(move |value| Message::ToggleContext(index, value)),
            );
            if message.show_context {
                contents = contents.push(text(&message.context_preview).size(11));
            }
        }
        let bubble = container(contents)
            .max_width(680)
            .padding([13, 16])
            .style(if is_user {
                user_bubble_style
            } else {
                assistant_bubble_style
            });
        match message.role {
            ChatRole::User => row![Space::new(Fill, 0), bubble, avatar]
                .spacing(10)
                .align_y(Alignment::End)
                .into(),
            ChatRole::Assistant => row![avatar, bubble, Space::new(Fill, 0)]
                .spacing(10)
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
                text(format!("{:02}", index + 1)).size(13),
                column![
                    text(subject).size(15),
                    text(format!(
                        "{}  •  {}",
                        hit.sender,
                        hit.sent_at.as_deref().unwrap_or("Unknown date")
                    ))
                    .size(11)
                ]
                .spacing(3)
            ]
            .spacing(10),
            text(hit.excerpt(420)).size(13),
            button(if self.expanded_hit == Some(index) {
                "Hide details"
            } else {
                "View full excerpt and metadata"
            })
            .on_press(Message::ToggleHit(index))
        ]
        .spacing(8);
        if self.expanded_hit == Some(index) {
            contents = contents
                .push(
                    text(format!(
                        "PST: {}  •  Folder: {}  •  Retrieval score: {:.3}  •  Local ID: {}",
                        source_name, hit.folder, hit.score, hit.id
                    ))
                    .size(11),
                )
                .push(horizontal_rule(1))
                .push(text(hit.excerpt(1200)).size(12));
        }
        container(contents)
            .width(Fill)
            .padding(14)
            .style(source_style)
            .into()
    }
}
