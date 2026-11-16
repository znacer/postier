mod parser;

use dioxus::prelude::*;

pub(crate) use self::parser::{MdBlock, MdSpan, parse_inline, parse_markdown};

use crate::components::separator::Separator;

use super::{
    DispatchMessage,
    theme::{
        Color, accent_text, blockquote_style, bright_text, code_block_style, md_table_header_style,
        md_table_row_style, muted, on_primary,
    },
};

pub(super) fn render_markdown_blocks<M: DispatchMessage>(
    blocks: &[MdBlock],
    is_user: bool,
    on_link: impl Fn(&str) -> M + Copy + 'static,
) -> Element {
    if blocks.is_empty() {
        return rsx! {};
    }

    let base_color = if is_user { on_primary() } else { bright_text() };
    let mut rendered_blocks = Vec::with_capacity(blocks.len());

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
                } else if *level <= 2 {
                    bright_text()
                } else {
                    accent_text()
                };
                let inline = render_inline_text(title, size, heading_color, true, false, on_link);
                rendered_blocks.push(rsx! {
                    div { style: "width: 100%;", {inline} }
                });
            }
            MdBlock::Paragraph(paragraph) => {
                let inline = render_inline_text(paragraph, 13.0, base_color, false, false, on_link);
                rendered_blocks.push(rsx! {
                    div { style: "width: 100%;", {inline} }
                });
            }
            MdBlock::CodeBlock { lang, code } => {
                let lang_upper = lang.to_ascii_uppercase();
                let has_lang = !lang.is_empty();
                let code_str = code.clone();
                let accent = accent_text();
                let bright = bright_text();
                let block_style = code_block_style();
                rendered_blocks.push(rsx! {
                    div { style: "width: 100%; padding: 8px 10px; display: flex; flex-direction: column; gap: 4px; {block_style}",
                        if has_lang {
                            div { style: "display: flex; flex-direction: row; align-items: center;",
                                span { style: "font-size: 9px; font-family: monospace; color: {accent};",
                                    "{lang_upper}"
                                }
                            }
                        }
                        div { style: "overflow-x: auto; padding: 4px 2px;",
                            pre {
                                class: "selectable-text",
                                style: "margin: 0; font-size: 11.5px; line-height: 1.45; font-family: monospace; color: {bright};",
                                "{code_str}"
                            }
                        }
                    }
                });
            }
            MdBlock::BlockQuote(lines) => {
                let quote_style = blockquote_style();
                let quote_lines: Vec<Element> = lines
                    .iter()
                    .map(|line| render_inline_text(line, 12.5, muted(), false, true, on_link))
                    .collect();
                rendered_blocks.push(rsx! {
                    div { style: "width: 100%; padding: 7px 11px; display: flex; flex-direction: column; gap: 4px; {quote_style}",
                        for ql in quote_lines {
                            {ql}
                        }
                    }
                });
            }
            MdBlock::List(items) => {
                let mut list_rows = Vec::with_capacity(items.len());
                for item in items {
                    let indent_px = (item.indent.min(6) as u16) * 14;
                    let bullet_label = match item.task_checked {
                        Some(true) => "Done:".to_owned(),
                        Some(false) => "Todo:".to_owned(),
                        None => item.marker.clone(),
                    };
                    let bullet_color = match item.task_checked {
                        Some(true) => accent_text(),
                        _ => muted(),
                    };
                    let item_inline =
                        render_inline_text(&item.text, 13.0, base_color, false, false, on_link);
                    list_rows.push(rsx! {
                        div { style: "display: flex; flex-direction: row; align-items: flex-start; gap: 6px; padding-left: {indent_px}px;",
                            span { style: "font-size: 12.5px; color: {bullet_color}; flex-shrink: 0; line-height: 1.42;",
                                "{bullet_label}"
                            }
                            div { style: "flex: 1;", {item_inline} }
                        }
                    });
                }
                rendered_blocks.push(rsx! {
                    div { style: "width: 100%; display: flex; flex-direction: column; gap: 4px;",
                        for row_el in list_rows {
                            {row_el}
                        }
                    }
                });
            }
            MdBlock::Table { headers, rows } => {
                let num_cols = headers
                    .len()
                    .max(rows.iter().map(|r| r.len()).max().unwrap_or(0))
                    .max(1);
                let header_style = md_table_header_style();
                let row_style = md_table_row_style();

                let header_cells: Vec<Element> = if !headers.is_empty() {
                    (0..num_cols)
                        .map(|col_idx| {
                            let cell_text = headers.get(col_idx).map(String::as_str).unwrap_or("");
                            let inline = render_inline_text(
                                cell_text,
                                11.5,
                                bright_text(),
                                true,
                                false,
                                on_link,
                            );
                            rsx! {
                                div { style: "flex: 1;", {inline} }
                            }
                        })
                        .collect()
                } else {
                    Vec::new()
                };

                let mut body_rows = Vec::with_capacity(rows.len());
                for data_row in rows {
                    let cells: Vec<Element> = (0..num_cols)
                        .map(|col_idx| {
                            let cell_text = data_row.get(col_idx).map(String::as_str).unwrap_or("");
                            let inline = render_inline_text(
                                cell_text, 11.5, base_color, false, false, on_link,
                            );
                            rsx! {
                                div { style: "flex: 1;", {inline} }
                            }
                        })
                        .collect();
                    body_rows.push(rsx! {
                        div { style: "width: 100%; padding: 5px 8px; display: flex; flex-direction: row; align-items: flex-start; gap: 8px; {row_style}",
                            for c in cells {
                                {c}
                            }
                        }
                    });
                }

                rendered_blocks.push(rsx! {
                    div { style: "width: 100%; display: flex; flex-direction: column;",
                        if !header_cells.is_empty() {
                            div { style: "width: 100%; padding: 6px 8px; display: flex; flex-direction: row; align-items: center; gap: 8px; {header_style}",
                                for hc in header_cells {
                                    {hc}
                                }
                            }
                        }
                        for br in body_rows {
                            {br}
                        }
                    }
                });
            }
            MdBlock::Rule => {
                rendered_blocks.push(rsx! {
                    Separator {}
                });
            }
        }
    }

    rsx! {
        div {
            class: "selectable-text",
            style: "width: 100%; display: flex; flex-direction: column; gap: 8px;",
            for block_el in rendered_blocks {
                {block_el}
            }
        }
    }
}

fn render_inline_text<M: DispatchMessage>(
    input: &str,
    font_size: f32,
    default_color: Color,
    force_bold: bool,
    force_italic: bool,
    on_link: impl Fn(&str) -> M + Copy + 'static,
) -> Element {
    let parsed = parse_inline(input);
    let mut spans = Vec::with_capacity(parsed.len());
    for item in parsed {
        match item {
            MdSpan::Text(content) => {
                let weight = if force_bold { "bold" } else { "normal" };
                let fstyle = if force_italic { "italic" } else { "normal" };
                spans.push(rsx! {
                    span { style: "font-size: {font_size}px; color: {default_color}; font-weight: {weight}; font-style: {fstyle};",
                        "{content}"
                    }
                });
            }
            MdSpan::Styled {
                text: content,
                bold,
                italic,
                strike,
            } => {
                let weight = if bold || force_bold { "bold" } else { "normal" };
                let fstyle = if italic || force_italic {
                    "italic"
                } else {
                    "normal"
                };
                let deco = if strike { "line-through" } else { "none" };
                spans.push(rsx! {
                    span { style: "font-size: {font_size}px; color: {default_color}; font-weight: {weight}; font-style: {fstyle}; text-decoration: {deco};",
                        "{content}"
                    }
                });
            }
            MdSpan::Code(code) => {
                let code_size = (font_size - 1.0).max(10.0);
                let code_color = Color::from_rgb(0.95, 0.85, 0.58);
                let code_bg = Color::from_rgba(0.04, 0.05, 0.08, 0.92);
                let code_border = Color::from_rgba(0.25, 0.30, 0.40, 0.7);
                spans.push(rsx! {
                    code { style: "font-size: {code_size}px; font-family: monospace; color: {code_color}; background-color: {code_bg}; border: 1px solid {code_border}; border-radius: 4px; padding: 0px 4px;",
                        "{code}"
                    }
                });
            }
            MdSpan::Link { text: label, url } => {
                let link_msg = on_link(&url);
                let accent = accent_text();
                let weight = if force_bold { "bold" } else { "normal" };
                let fstyle = if force_italic { "italic" } else { "normal" };
                spans.push(rsx! {
                    a {
                        href: "#",
                        style: "font-size: {font_size}px; color: {accent}; text-decoration: underline; font-weight: {weight}; font-style: {fstyle}; cursor: pointer;",
                        onclick: move |evt| {
                            evt.prevent_default();
                            link_msg.clone().dispatch();
                        },
                        "{label}"
                    }
                });
            }
        }
    }

    rsx! {
        div { style: "line-height: 1.42; width: 100%; word-break: break-word;",
            for s in spans {
                {s}
            }
        }
    }
}
