mod parser;

use iced::{
    Alignment, Border, Color, Element, Fill, FillPortion, Font, Padding, font,
    widget::{Space, column, container, horizontal_rule, rich_text, row, scrollable, span, text},
};

pub(crate) use self::parser::{MdBlock, MdSpan, parse_inline, parse_markdown};

use super::{
    Message,
    theme::{
        accent_text, blockquote_style, bright_text, code_block_style, md_table_header_style,
        md_table_row_style, muted, on_primary,
    },
};

pub(super) fn render_markdown<'a>(content: &'a str, is_user: bool) -> Element<'a, Message> {
    let blocks = parse_markdown(content);
    if blocks.is_empty() {
        return Space::new(0, 0).into();
    }

    let base_color = if is_user { on_primary() } else { bright_text() };
    let mut col = column![].spacing(8).width(Fill);

    for block in blocks {
        match block {
            MdBlock::Heading { level, text: title } => {
                let size = match level {
                    1 => 18.0,
                    2 => 16.0,
                    3 => 14.5,
                    4 => 13.5,
                    _ => 13.0,
                };
                let heading_color = if is_user {
                    on_primary()
                } else if level <= 2 {
                    bright_text()
                } else {
                    accent_text()
                };
                col = col.push(render_inline_text(&title, size, heading_color, true, false));
            }
            MdBlock::Paragraph(paragraph) => {
                col = col.push(render_inline_text(
                    &paragraph, 13.0, base_color, false, false,
                ));
            }
            MdBlock::CodeBlock { lang, code } => {
                let mut block_col = column![].spacing(4).width(Fill);
                if !lang.is_empty() {
                    block_col = block_col.push(
                        row![
                            text(lang.to_ascii_uppercase())
                                .size(9)
                                .font(Font::MONOSPACE)
                                .color(accent_text()),
                            Space::new(Fill, 0)
                        ]
                        .align_y(Alignment::Center),
                    );
                }
                let code_content = scrollable(
                    container(
                        text(code)
                            .size(11.5)
                            .line_height(1.45)
                            .font(Font::MONOSPACE)
                            .color(bright_text()),
                    )
                    .padding([4, 2]),
                )
                .direction(scrollable::Direction::Horizontal(
                    scrollable::Scrollbar::new().width(4).scroller_width(4),
                ));
                block_col = block_col.push(code_content);
                col = col.push(
                    container(block_col)
                        .width(Fill)
                        .padding([8, 10])
                        .style(code_block_style),
                );
            }
            MdBlock::BlockQuote(lines) => {
                let mut quote_col = column![].spacing(4).width(Fill);
                for line in lines {
                    quote_col =
                        quote_col.push(render_inline_text(&line, 12.5, muted(), false, true));
                }
                col = col.push(
                    container(quote_col)
                        .width(Fill)
                        .padding([7, 11])
                        .style(blockquote_style),
                );
            }
            MdBlock::List(items) => {
                let mut list_col = column![].spacing(4).width(Fill);
                for item in items {
                    let indent_px = (item.indent.min(6) as u16) * 14;
                    let bullet_label = match item.task_checked {
                        Some(true) => "Done:".to_owned(),
                        Some(false) => "Todo:".to_owned(),
                        None => item.marker,
                    };
                    let bullet_color = match item.task_checked {
                        Some(true) => accent_text(),
                        _ => muted(),
                    };
                    let item_row = row![
                        Space::new(indent_px, 0),
                        text(bullet_label).size(12.5).color(bullet_color),
                        render_inline_text(&item.text, 13.0, base_color, false, false),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Start);
                    list_col = list_col.push(item_row);
                }
                col = col.push(list_col);
            }
            MdBlock::Table { headers, rows } => {
                let num_cols = headers
                    .len()
                    .max(rows.iter().map(|r| r.len()).max().unwrap_or(0))
                    .max(1);
                let mut table_col = column![].spacing(0).width(Fill);

                if !headers.is_empty() {
                    let mut header_row = row![].spacing(8).align_y(Alignment::Center);
                    for col_idx in 0..num_cols {
                        let cell_text = headers.get(col_idx).map(String::as_str).unwrap_or("");
                        header_row = header_row.push(
                            container(render_inline_text(
                                cell_text,
                                11.5,
                                bright_text(),
                                true,
                                false,
                            ))
                            .width(FillPortion(1)),
                        );
                    }
                    table_col = table_col.push(
                        container(header_row)
                            .width(Fill)
                            .padding([6, 8])
                            .style(md_table_header_style),
                    );
                }

                for data_row in rows {
                    let mut r = row![].spacing(8).align_y(Alignment::Start);
                    for col_idx in 0..num_cols {
                        let cell_text = data_row.get(col_idx).map(String::as_str).unwrap_or("");
                        r = r.push(
                            container(render_inline_text(
                                cell_text, 11.5, base_color, false, false,
                            ))
                            .width(FillPortion(1)),
                        );
                    }
                    table_col = table_col.push(
                        container(r)
                            .width(Fill)
                            .padding([5, 8])
                            .style(md_table_row_style),
                    );
                }

                col = col.push(table_col);
            }
            MdBlock::Rule => {
                col = col.push(horizontal_rule(1));
            }
        }
    }

    col.into()
}

fn render_inline_text<'a>(
    input: &str,
    font_size: f32,
    default_color: Color,
    force_bold: bool,
    force_italic: bool,
) -> Element<'a, Message> {
    let parsed = parse_inline(input);
    let spans: Vec<_> = parsed
        .into_iter()
        .map(|item| match item {
            MdSpan::Text(content) => {
                let mut s = span(content).size(font_size).color(default_color);
                if force_bold || force_italic {
                    s = s.font(styled_font(force_bold, force_italic));
                }
                s
            }
            MdSpan::Styled {
                text: content,
                bold,
                italic,
                strike,
            } => {
                let is_bold = bold || force_bold;
                let is_italic = italic || force_italic;
                span(content)
                    .size(font_size)
                    .color(default_color)
                    .font(styled_font(is_bold, is_italic))
                    .strikethrough(strike)
            }
            MdSpan::Code(code) => span(code)
                .size((font_size - 1.0).max(10.0))
                .font(Font::MONOSPACE)
                .color(Color::from_rgb(0.95, 0.85, 0.58))
                .background(Color {
                    r: 0.04,
                    g: 0.05,
                    b: 0.08,
                    a: 0.92,
                })
                .border(
                    Border::default()
                        .rounded(4.0)
                        .color(Color {
                            r: 0.25,
                            g: 0.30,
                            b: 0.40,
                            a: 0.7,
                        })
                        .width(1),
                )
                .padding(Padding {
                    top: 0.0,
                    right: 4.0,
                    bottom: 0.0,
                    left: 4.0,
                }),
            MdSpan::Link { text: label, url } => {
                let link_msg = parse_link_message(&url);
                span(label)
                    .size(font_size)
                    .color(accent_text())
                    .underline(true)
                    .font(styled_font(force_bold, force_italic))
                    .link(link_msg)
            }
        })
        .collect();

    rich_text(spans).line_height(1.42).width(Fill).into()
}

fn styled_font(bold: bool, italic: bool) -> Font {
    Font {
        weight: if bold {
            font::Weight::Bold
        } else {
            font::Weight::Normal
        },
        style: if italic {
            font::Style::Italic
        } else {
            font::Style::Normal
        },
        ..Font::default()
    }
}

fn parse_link_message(url: &str) -> Message {
    let trimmed = url.trim();
    if let Some(rest) = trimmed
        .strip_prefix("mail://")
        .or_else(|| trimmed.strip_prefix("#mail-"))
        .or_else(|| trimmed.strip_prefix('#'))
        && let Ok(id) = rest.trim().parse::<i64>()
    {
        return Message::SelectEmail(id);
    }
    Message::MarkdownLinkClicked(trimmed.to_owned())
}
