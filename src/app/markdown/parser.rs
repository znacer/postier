#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MdBlock {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph(String),
    CodeBlock {
        lang: String,
        code: String,
    },
    BlockQuote(Vec<String>),
    List(Vec<MdListItem>),
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Rule,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdListItem {
    pub indent: usize,
    pub marker: String,
    pub task_checked: Option<bool>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MdSpan {
    Text(String),
    Styled {
        text: String,
        bold: bool,
        italic: bool,
        strike: bool,
    },
    Code(String),
    Link {
        text: String,
        url: String,
    },
}

pub fn parse_markdown(input: &str) -> Vec<MdBlock> {
    let lines: Vec<&str> = input.lines().collect();
    let mut blocks = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        // Fenced code block: ```lang or ~~~lang
        if let Some((fence_char, fence_len, lang)) = parse_code_fence(trimmed) {
            i += 1;
            let mut code_lines = Vec::new();
            while i < lines.len() {
                let candidate = lines[i].trim();
                if is_closing_fence(candidate, fence_char, fence_len) {
                    i += 1;
                    break;
                }
                code_lines.push(lines[i]);
                i += 1;
            }
            blocks.push(MdBlock::CodeBlock {
                lang,
                code: code_lines.join("\n"),
            });
            continue;
        }

        // Horizontal rule: ---, ***, ___
        if is_horizontal_rule(trimmed) {
            blocks.push(MdBlock::Rule);
            i += 1;
            continue;
        }

        // ATX Heading: # .. ######
        if let Some((level, heading_text)) = parse_atx_heading(trimmed) {
            blocks.push(MdBlock::Heading {
                level,
                text: heading_text.to_owned(),
            });
            i += 1;
            continue;
        }

        // Table: header row followed by |---|---| separator row
        if trimmed.contains('|') && i + 1 < lines.len() && is_table_separator(lines[i + 1].trim()) {
            let headers = split_table_row(trimmed);
            i += 2;
            let mut rows = Vec::new();
            while i < lines.len() {
                let row_trimmed = lines[i].trim();
                if row_trimmed.is_empty() || !row_trimmed.contains('|') {
                    break;
                }
                if is_table_separator(row_trimmed) {
                    i += 1;
                    continue;
                }
                rows.push(split_table_row(row_trimmed));
                i += 1;
            }
            blocks.push(MdBlock::Table { headers, rows });
            continue;
        }

        // Blockquote: > ...
        if trimmed.starts_with('>') {
            let mut quote_lines = Vec::new();
            while i < lines.len() {
                let qt = lines[i].trim();
                if let Some(stripped) = qt.strip_prefix('>') {
                    quote_lines.push(stripped.trim_start().to_owned());
                    i += 1;
                } else {
                    break;
                }
            }
            blocks.push(MdBlock::BlockQuote(quote_lines));
            continue;
        }

        // List: unordered (-, *, +) or ordered (1., 2))
        if let Some(first_item) = parse_list_line(line) {
            let mut items = vec![first_item];
            i += 1;
            while i < lines.len() {
                let next_line = lines[i];
                let next_trimmed = next_line.trim();
                if next_trimmed.is_empty() {
                    break;
                }
                if let Some(item) = parse_list_line(next_line) {
                    items.push(item);
                    i += 1;
                } else if next_line.starts_with("  ") || next_line.starts_with('\t') {
                    if let Some(last) = items.last_mut() {
                        last.text.push(' ');
                        last.text.push_str(next_trimmed);
                    }
                    i += 1;
                } else {
                    break;
                }
            }
            blocks.push(MdBlock::List(items));
            continue;
        }

        // Paragraph: collect contiguous normal lines
        let mut para_lines = vec![trimmed];
        i += 1;
        while i < lines.len() {
            let next_line = lines[i];
            let next_trimmed = next_line.trim();
            if next_trimmed.is_empty()
                || parse_code_fence(next_trimmed).is_some()
                || is_horizontal_rule(next_trimmed)
                || parse_atx_heading(next_trimmed).is_some()
                || next_trimmed.starts_with('>')
                || parse_list_line(next_line).is_some()
                || (next_trimmed.contains('|')
                    && i + 1 < lines.len()
                    && is_table_separator(lines[i + 1].trim()))
            {
                break;
            }
            para_lines.push(next_trimmed);
            i += 1;
        }
        blocks.push(MdBlock::Paragraph(join_paragraph_lines(&para_lines)));
    }

    blocks
}

fn join_paragraph_lines(lines: &[&str]) -> String {
    let mut out = String::new();
    for (idx, &line) in lines.iter().enumerate() {
        if idx > 0 {
            let prev = lines[idx - 1];
            if prev.ends_with('\\') {
                out.pop();
                out.push('\n');
            } else {
                out.push(' ');
            }
        }
        out.push_str(line);
    }
    out
}

fn parse_code_fence(trimmed: &str) -> Option<(char, usize, String)> {
    let first = trimmed.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let fence_len = trimmed.chars().take_while(|&c| c == first).count();
    if fence_len < 3 {
        return None;
    }
    let rest = trimmed[fence_len..].trim();
    if first == '`' && rest.contains('`') {
        return None;
    }
    let lang = rest.split_whitespace().next().unwrap_or("").to_owned();
    Some((first, fence_len, lang))
}

fn is_closing_fence(trimmed: &str, fence_char: char, min_len: usize) -> bool {
    let count = trimmed.chars().take_while(|&c| c == fence_char).count();
    count >= min_len && trimmed[count..].trim().is_empty()
}

fn is_horizontal_rule(trimmed: &str) -> bool {
    if trimmed.len() < 3 {
        return false;
    }
    let mut marker = None;
    let mut count = 0usize;
    for ch in trimmed.chars() {
        if ch == ' ' {
            continue;
        }
        if ch == '-' || ch == '*' || ch == '_' {
            if *marker.get_or_insert(ch) == ch {
                count += 1;
            } else {
                return false;
            }
        } else {
            return false;
        }
    }
    count >= 3
}

fn parse_atx_heading(trimmed: &str) -> Option<(u8, &str)> {
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let after = &trimmed[hashes..];
    if !after.starts_with(' ') && !after.starts_with('\t') {
        return None;
    }
    let text = after.trim().trim_end_matches('#').trim();
    if text.is_empty() {
        return None;
    }
    Some((hashes as u8, text))
}

fn is_table_separator(trimmed: &str) -> bool {
    if !trimmed.contains('|') || !trimmed.contains('-') {
        return false;
    }
    let cells = split_table_row(trimmed);
    !cells.is_empty()
        && cells.iter().all(|cell| {
            let c = cell.trim();
            !c.is_empty()
                && c.chars().any(|ch| ch == '-')
                && c.chars().all(|ch| ch == '-' || ch == ':')
        })
}

fn split_table_row(trimmed: &str) -> Vec<String> {
    let without_prefix = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let inner = without_prefix.strip_suffix('|').unwrap_or(without_prefix);
    inner.split('|').map(|s| s.trim().to_owned()).collect()
}

fn parse_list_line(line: &str) -> Option<MdListItem> {
    let leading_spaces = line
        .chars()
        .take_while(|&c| c == ' ' || c == '\t')
        .map(|c| if c == '\t' { 2 } else { 1 })
        .sum::<usize>();
    let indent = leading_spaces / 2;
    let trimmed = line.trim_start();

    // Unordered list: -, *, +
    for prefix in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let (task_checked, item_text) = parse_task_prefix(rest.trim_start());
            return Some(MdListItem {
                indent,
                marker: "-".to_owned(),
                task_checked,
                text: item_text.to_owned(),
            });
        }
    }

    // Ordered list: 1. or 1)
    let digit_bytes = trimmed.bytes().take_while(|b| b.is_ascii_digit()).count();
    if (1..=6).contains(&digit_bytes) {
        let number = &trimmed[..digit_bytes];
        let after_digits = &trimmed[digit_bytes..];
        if let Some(rest) = after_digits
            .strip_prefix(". ")
            .or_else(|| after_digits.strip_prefix(") "))
        {
            let (task_checked, item_text) = parse_task_prefix(rest.trim_start());
            return Some(MdListItem {
                indent,
                marker: format!("{number}."),
                task_checked,
                text: item_text.to_owned(),
            });
        }
    }

    None
}

fn parse_task_prefix(rest: &str) -> (Option<bool>, &str) {
    if let Some(after) = rest.strip_prefix("[ ] ") {
        (Some(false), after)
    } else if let Some(after) = rest
        .strip_prefix("[x] ")
        .or_else(|| rest.strip_prefix("[X] "))
    {
        (Some(true), after)
    } else {
        (None, rest)
    }
}

pub fn parse_inline(input: &str) -> Vec<MdSpan> {
    let mut spans = Vec::new();
    let mut plain = String::new();
    let mut idx = 0;

    let flush_plain = |plain: &mut String, spans: &mut Vec<MdSpan>| {
        if !plain.is_empty() {
            spans.push(MdSpan::Text(std::mem::take(plain)));
        }
    };

    while idx < input.len() {
        let rest = &input[idx..];

        // Escaped character: \* \_ \` \[ etc.
        if rest.starts_with('\\') && rest.len() >= 2 {
            let mut chars = rest.chars();
            let _ = chars.next();
            if let Some(escaped) = chars.next()
                && matches!(
                    escaped,
                    '\\' | '`' | '*' | '_' | '~' | '[' | ']' | '(' | ')' | '#' | '+' | '-' | '!'
                )
            {
                plain.push(escaped);
                idx += 1 + escaped.len_utf8();
                continue;
            }
        }

        // Inline code: ``code`` or `code`
        if rest.starts_with("``")
            && let Some(end) = rest[2..].find("``")
        {
            flush_plain(&mut plain, &mut spans);
            let code = &rest[2..2 + end];
            spans.push(MdSpan::Code(code.trim().to_owned()));
            idx += 4 + end;
            continue;
        }
        if rest.starts_with('`')
            && let Some(end) = rest[1..].find('`')
            && end > 0
        {
            flush_plain(&mut plain, &mut spans);
            let code = &rest[1..1 + end];
            spans.push(MdSpan::Code(code.to_owned()));
            idx += 2 + end;
            continue;
        }

        // Markdown link: [text](url)
        if rest.starts_with('[')
            && let Some((label, url, consumed)) = parse_markdown_link(rest)
        {
            flush_plain(&mut plain, &mut spans);
            spans.push(MdSpan::Link { text: label, url });
            idx += consumed;
            continue;
        }

        // Bold + Italic: ***text*** or ___text___
        if let Some((_delim, inner, consumed)) = match_delimited(rest, &["***", "___"]) {
            flush_plain(&mut plain, &mut spans);
            spans.push(MdSpan::Styled {
                text: inner.to_owned(),
                bold: true,
                italic: true,
                strike: false,
            });
            idx += consumed;
            continue;
        }

        // Bold: **text** or __text__
        if let Some((_delim, inner, consumed)) = match_delimited(rest, &["**", "__"]) {
            flush_plain(&mut plain, &mut spans);
            spans.push(MdSpan::Styled {
                text: inner.to_owned(),
                bold: true,
                italic: false,
                strike: false,
            });
            idx += consumed;
            continue;
        }

        // Strikethrough: ~~text~~
        if let Some((_delim, inner, consumed)) = match_delimited(rest, &["~~"]) {
            flush_plain(&mut plain, &mut spans);
            spans.push(MdSpan::Styled {
                text: inner.to_owned(),
                bold: false,
                italic: false,
                strike: true,
            });
            idx += consumed;
            continue;
        }

        // Italic: *text* or _text_
        if let Some((_delim, inner, consumed)) = match_single_italic(rest) {
            flush_plain(&mut plain, &mut spans);
            spans.push(MdSpan::Styled {
                text: inner.to_owned(),
                bold: false,
                italic: true,
                strike: false,
            });
            idx += consumed;
            continue;
        }

        if let Some(ch) = rest.chars().next() {
            plain.push(ch);
            idx += ch.len_utf8();
        } else {
            break;
        }
    }

    flush_plain(&mut plain, &mut spans);
    if spans.is_empty() {
        spans.push(MdSpan::Text(String::new()));
    }
    spans
}

fn match_delimited<'a>(
    rest: &'a str,
    delims: &[&'static str],
) -> Option<(&'static str, &'a str, usize)> {
    for &delim in delims {
        if let Some(after) = rest.strip_prefix(delim)
            && let Some(end) = after.find(delim)
            && end > 0
        {
            let inner = &after[..end];
            if !inner.trim().is_empty() {
                return Some((delim, inner, delim.len() * 2 + end));
            }
        }
    }
    None
}

fn match_single_italic(rest: &str) -> Option<(char, &str, usize)> {
    for delim in ['*', '_'] {
        if rest.starts_with(delim) {
            let after = &rest[1..];
            if after.starts_with(delim) || after.starts_with(' ') {
                continue;
            }
            if let Some(end) = after.find(delim)
                && end > 0
            {
                let inner = &after[..end];
                if !inner.ends_with(' ') && !inner.contains('\n') {
                    return Some((delim, inner, 2 + end));
                }
            }
        }
    }
    None
}

fn parse_markdown_link(rest: &str) -> Option<(String, String, usize)> {
    let after_open = rest.strip_prefix('[')?;
    let close_bracket = after_open.find("](")?;
    let label = &after_open[..close_bracket];
    if label.is_empty() || label.contains('\n') {
        return None;
    }
    let after_paren = &after_open[close_bracket + 2..];
    let close_paren = after_paren.find(')')?;
    let url = after_paren[..close_paren].trim();
    if url.is_empty() {
        return None;
    }
    let consumed = 1 + close_bracket + 2 + close_paren + 1;
    Some((label.to_owned(), url.to_owned(), consumed))
}
