use iced::{
    Alignment, Element, Fill,
    widget::{checkbox, column, container, horizontal_rule, row, scrollable, text, text_input},
};

use super::{Message, PstRagApp, theme::*};

impl PstRagApp {
    pub(super) fn sidebar(&self) -> Element<'_, Message> {
        let name = self
            .archive
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("No archive selected");
        let choose = enabled_button("Choose PST", (!self.busy).then_some(Message::ChooseArchive));
        let index = enabled_button(
            "Index now",
            (!self.busy && self.archive.is_some()).then_some(Message::Index),
        );
        let storage = enabled_button(
            "Choose directory",
            (!self.busy).then_some(Message::ChooseStorage),
        );
        let save = enabled_button(
            "Save AI settings",
            (!self.busy).then_some(Message::SaveCredentials),
        );

        let archive = card(
            column![
                row![
                    text(self.count).size(30).color(primary()),
                    column![
                        text("MESSAGES").size(10).color(muted()),
                        text("indexed locally").size(12).color(muted())
                    ]
                    .spacing(1)
                ]
                .spacing(10)
                .align_y(Alignment::Center),
                horizontal_rule(1),
                text(name).size(15),
                text(
                    self.archive
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                )
                .size(11),
                row![choose, index].spacing(8)
            ]
            .spacing(8),
        );

        let storage_card = card(
            column![
                text(self.storage_dir.display().to_string()).size(11),
                text("Contains BM25, optional vectors, and AI credentials").size(11),
                storage
            ]
            .spacing(8),
        );

        let mut indexed_sources = column![].spacing(10);
        if self.sources.is_empty() {
            indexed_sources = indexed_sources.push(text("No PST files indexed yet").size(11));
        }
        for source in &self.sources {
            let source_key = source.key.clone();
            let mut source_view = column![
                checkbox(
                    format!("{} ({} messages)", source.name, source.message_count),
                    source.enabled
                )
                .on_toggle(move |enabled| Message::SourceEnabledChanged(
                    source_key.clone(),
                    enabled
                ))
            ]
            .spacing(5);
            for folder in &source.folders {
                let source_key = source.key.clone();
                let folder_path = folder.path.clone();
                source_view = source_view.push(
                    checkbox(
                        format!("  {} ({})", folder.path, folder.message_count),
                        source.enabled && folder.enabled,
                    )
                    .on_toggle(move |enabled| {
                        Message::FolderEnabledChanged(
                            source_key.clone(),
                            folder_path.clone(),
                            enabled,
                        )
                    }),
                );
            }
            if self.pending_remove_source.as_deref() == Some(&source.key) {
                source_view = source_view.push(
                    column![
                        text("Remove this PST's messages and embeddings?").size(10),
                        row![
                            enabled_button("Cancel", Some(Message::CancelRemoveSource)),
                            enabled_button("Remove", Some(Message::ConfirmRemoveSource))
                        ]
                        .spacing(6)
                    ]
                    .spacing(5),
                );
            } else {
                source_view = source_view.push(enabled_button(
                    "Remove index",
                    (!self.busy).then_some(Message::RemoveSourceRequested(source.key.clone())),
                ));
            }
            indexed_sources = indexed_sources.push(container(source_view).padding([6, 0]));
        }
        let source_manager = card(indexed_sources);

        let llm = card(
            column![
                text("Endpoint").size(12),
                text_input("https://…", &self.config.endpoint).on_input(Message::EndpointChanged),
                text("Model").size(12),
                text_input("Model name", &self.config.model).on_input(Message::ModelChanged),
                text("API key").size(12),
                text_input("Optional bearer token", &self.config.bearer_token)
                    .secure(true)
                    .on_input(Message::TokenChanged),
                horizontal_rule(1),
                checkbox(
                    "Add remote embeddings during indexing",
                    self.config.use_remote_embeddings
                )
                .on_toggle(Message::RemoteEmbeddingsChanged),
                text("Embedding model").size(12),
                text_input("Embedding model name", &self.config.embedding_model)
                    .on_input(Message::EmbeddingModelChanged),
                text("BM25 is always indexed locally. Remote mode sends email text to the configured embeddings endpoint.").size(10),
                text("HTTPS required except for 127.0.0.1").size(10),
                save
            ]
            .spacing(7),
        );

        let brand = row![
            container(text("P").size(17).color(background()))
                .width(38)
                .height(38)
                .center_x(38)
                .center_y(38)
                .style(accent_tile_style),
            column![
                text("PSTbeetle").size(14),
                text("Private archive search").size(11).color(muted())
            ]
            .spacing(2)
        ]
        .spacing(11)
        .align_y(Alignment::Center);

        container(scrollable(
            column![
                brand,
                horizontal_rule(1),
                section_label("ARCHIVE"),
                archive,
                section_label("STORAGE"),
                storage_card,
                section_label("INDEXED PST FILES"),
                source_manager,
                section_label("AI MODELS"),
                llm
            ]
            .spacing(12)
            .padding(18),
        ))
        .width(310)
        .height(Fill)
        .style(sidebar_style)
        .into()
    }
}
