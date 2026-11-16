//! HTML email sanitization, plain-text-to-HTML formatting, HTML-to-text extraction,
//! participant (`From`, `To`, `Cc`) parsing, and localized date/time formatting.

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, Timelike};

use crate::i18n::Language;

/// A parsed email participant (sender or recipient).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactEntry {
    pub name: Option<String>,
    pub email: Option<String>,
    pub raw: String,
}

impl ContactEntry {
    /// Returns the human-friendly primary label (display name if available, otherwise email or raw string).
    pub fn display_primary(&self) -> &str {
        self.name
            .as_deref()
            .or(self.email.as_deref())
            .unwrap_or(&self.raw)
    }

    /// Returns 1–2 uppercase initials for avatar badges.
    pub fn initials(&self) -> String {
        let source = self
            .name
            .as_deref()
            .or_else(|| {
                self.email
                    .as_deref()
                    .map(|e| e.split('@').next().unwrap_or(e))
            })
            .unwrap_or(&self.raw);

        let parts: Vec<&str> = source
            .split(|c: char| c.is_whitespace() || matches!(c, '.' | '_' | '-' | ','))
            .filter(|s| s.chars().any(char::is_alphanumeric))
            .collect();

        match parts.as_slice() {
            [first, second, ..] => {
                let mut out = String::new();
                if let Some(c1) = first.chars().find(|c| c.is_alphanumeric()) {
                    out.extend(c1.to_uppercase());
                }
                if let Some(c2) = second.chars().find(|c| c.is_alphanumeric()) {
                    out.extend(c2.to_uppercase());
                }
                out
            }
            [only] => only
                .chars()
                .filter(|c| c.is_alphanumeric())
                .take(2)
                .flat_map(char::to_uppercase)
                .collect(),
            [] => "?".to_string(),
        }
    }
}

/// Structured envelope participants (`From`, `To`, `Cc`) for an email.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedParticipants {
    pub from: Option<ContactEntry>,
    pub to: Vec<ContactEntry>,
    pub cc: Vec<ContactEntry>,
}

/// Parses a single contact string such as `"Alice Martin <alice@example.com>"` or `"alice@example.com"`.
pub fn parse_contact(raw: &str) -> ContactEntry {
    let trimmed = raw
        .trim()
        .trim_matches(|c: char| c == ',' || c == ';' || c.is_whitespace());
    if trimmed.is_empty() {
        return ContactEntry {
            name: None,
            email: None,
            raw: String::new(),
        };
    }

    // Case 1: `Display Name <email@domain>` or `<email@domain>`
    if let Some(lt_idx) = trimmed.rfind('<')
        && let Some(gt_rel) = trimmed[lt_idx + 1..].find('>')
    {
        let inside = trimmed[lt_idx + 1..lt_idx + 1 + gt_rel].trim();
        let before = trimmed[..lt_idx]
            .trim()
            .trim_matches(|c: char| c == '"' || c == '\'')
            .trim();
        let email = (!inside.is_empty()).then(|| inside.to_string());
        let name =
            (!before.is_empty() && Some(before) != email.as_deref()).then(|| before.to_string());
        return ContactEntry {
            name,
            email,
            raw: trimmed.to_string(),
        };
    }

    // Case 2: `Display Name (email@domain)`
    if let Some(lp_idx) = trimmed.rfind('(')
        && let Some(rp_rel) = trimmed[lp_idx + 1..].find(')')
    {
        let inside = trimmed[lp_idx + 1..lp_idx + 1 + rp_rel].trim();
        if inside.contains('@') && !inside.contains(' ') {
            let before = trimmed[..lp_idx]
                .trim()
                .trim_matches(|c: char| c == '"' || c == '\'')
                .trim();
            return ContactEntry {
                name: (!before.is_empty()).then(|| before.to_string()),
                email: Some(inside.to_string()),
                raw: trimmed.to_string(),
            };
        }
    }

    // Case 3: bare email address `user@domain`
    let unquoted = trimmed.trim_matches(|c: char| c == '"' || c == '\'').trim();
    if unquoted.contains('@') && !unquoted.contains(char::is_whitespace) {
        return ContactEntry {
            name: None,
            email: Some(unquoted.to_string()),
            raw: unquoted.to_string(),
        };
    }

    ContactEntry {
        name: Some(unquoted.to_string()),
        email: None,
        raw: trimmed.to_string(),
    }
}

/// Splits a comma- or semicolon-delimited contact list while respecting quotes and `<...>`.
pub fn parse_contact_list(raw: &str) -> Vec<ContactEntry> {
    let mut entries = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut angle_depth = 0usize;

    for ch in raw.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
            }
            '<' if !in_quotes => {
                angle_depth = angle_depth.saturating_add(1);
                current.push(ch);
            }
            '>' if !in_quotes => {
                angle_depth = angle_depth.saturating_sub(1);
                current.push(ch);
            }
            ',' | ';' if !in_quotes && angle_depth == 0 => {
                let candidate = current.trim();
                if !candidate.is_empty() {
                    let parsed = parse_contact(candidate);
                    if !parsed.raw.is_empty() {
                        entries.push(parsed);
                    }
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    let tail = current.trim();
    if !tail.is_empty() {
        let parsed = parse_contact(tail);
        if !parsed.raw.is_empty() {
            entries.push(parsed);
        }
    }
    entries
}

/// Parses `From`, `To`, and `Cc` participants from `sender`, `recipients`, and optional top-of-body headers.
pub fn parse_email_participants(sender: &str, recipients: &str, body: &str) -> ParsedParticipants {
    let sender = repair_text_encoding(sender);
    let sender = sender.as_str();
    let recipients = repair_text_encoding(recipients);
    let recipients = recipients.as_str();
    let body = repair_text_encoding(body);
    let body = body.as_str();
    let from = {
        let s = sender.trim();
        if s.is_empty() {
            None
        } else {
            Some(parse_contact(s))
        }
    };

    let mut to_segments = Vec::new();
    let mut cc_segments = Vec::new();

    for line in recipients.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Check if line contains an inline `| Cc:` or `; Cc:` separator
        if let Some((to_part, cc_part)) = split_inline_cc(trimmed) {
            if !to_part.is_empty() {
                to_segments.push(strip_to_prefix(to_part));
            }
            if !cc_part.is_empty() {
                cc_segments.push(cc_part);
            }
        } else if let Some(cc_rest) = strip_cc_prefix(trimmed) {
            cc_segments.push(cc_rest);
        } else {
            to_segments.push(strip_to_prefix(trimmed));
        }
    }

    let to: Vec<ContactEntry> = to_segments
        .into_iter()
        .flat_map(parse_contact_list)
        .collect();
    let mut cc: Vec<ContactEntry> = cc_segments
        .into_iter()
        .flat_map(parse_contact_list)
        .collect();

    // If `cc` was not present in `recipients`, check if the leading header block of `body` has `Cc:`
    if cc.is_empty() && !body.trim().is_empty() && !is_html_email(body) {
        for line in body.lines().take(10) {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                break;
            }
            if let Some(cc_rest) = strip_cc_prefix(trimmed) {
                cc.extend(parse_contact_list(cc_rest));
                break;
            }
            // Only scan while lines look like mail headers (`Key: Value`)
            if parse_reply_header_line(trimmed).is_none() && !is_reply_banner(trimmed) {
                break;
            }
        }
    }

    ParsedParticipants { from, to, cc }
}

fn strip_to_prefix(s: &str) -> &str {
    let trimmed = s.trim();
    for prefix in ["to:", "To:", "TO:", "à :", "À :", "à:", "À:"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return rest.trim();
        }
    }
    trimmed
}

fn strip_cc_prefix(s: &str) -> Option<&str> {
    let trimmed = s.trim();
    for prefix in ["cc:", "Cc:", "CC:", "cc :", "Cc :", "CC :"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let clean = rest.trim();
            if !clean.is_empty() {
                return Some(clean);
            }
        }
    }
    None
}

fn split_inline_cc(s: &str) -> Option<(&str, &str)> {
    for sep in [" | Cc:", " | cc:", " | CC:", "; Cc:", "; cc:", "; CC:"] {
        if let Some(idx) = s.find(sep) {
            let to_part = s[..idx].trim();
            let cc_part = s[idx + sep.len()..].trim();
            return Some((to_part, cc_part));
        }
    }
    None
}

/// Localized, human-readable date and time representations for an email timestamp.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormattedEmailDate {
    /// Full localized date + time (e.g. `"Lun. 10 mars 2025 à 14:32"` or `"Mon, Mar 10, 2025 at 14:32"`).
    pub full: String,
    /// Localized date portion (e.g. `"Lun. 10 mars 2025"` or `"Mon, Mar 10, 2025"`).
    pub date_part: String,
    /// Localized time portion (e.g. `"14:32"`), or empty if only a calendar date was present.
    pub time_part: String,
    /// Compact localized representation for table rows (e.g. `"10 mars 2025 · 14:32"`).
    pub table_short: String,
}

struct ParsedTimestamp {
    date: NaiveDate,
    hour_minute: Option<(u32, u32)>,
    tz_offset_secs: Option<i32>,
}

/// Parses an ISO-8601 / RFC-3339 / RFC-2822 / SQL timestamp string and formats it
/// into localized date and time strings.
pub fn format_email_datetime(raw: Option<&str>, lang: Language) -> Option<FormattedEmailDate> {
    let s = raw?.trim();
    if s.is_empty() {
        return None;
    }

    if let Some(parsed) = parse_any_datetime(s) {
        let year = parsed.date.year();
        let month_idx = (parsed.date.month0() as usize).min(11);
        let day = parsed.date.day();
        let wd_idx = parsed.date.weekday().num_days_from_monday() as usize;

        const FR_WEEKDAYS: [&str; 7] = ["Lun.", "Mar.", "Mer.", "Jeu.", "Ven.", "Sam.", "Dim."];
        const EN_WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

        const FR_MONTHS: [&str; 12] = [
            "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.",
            "nov.", "déc.",
        ];
        const EN_MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];

        let date_part = match lang {
            Language::French => {
                format!(
                    "{} {} {} {}",
                    FR_WEEKDAYS[wd_idx], day, FR_MONTHS[month_idx], year
                )
            }
            Language::English => {
                format!(
                    "{}, {} {}, {}",
                    EN_WEEKDAYS[wd_idx], EN_MONTHS[month_idx], day, year
                )
            }
        };

        let short_date_only = match lang {
            Language::French => format!("{day:02} {} {year}", FR_MONTHS[month_idx]),
            Language::English => format!("{} {day:02}, {year}", EN_MONTHS[month_idx]),
        };

        if let Some((hour, minute)) = parsed.hour_minute {
            let tz_suffix = match parsed.tz_offset_secs {
                Some(offset) if offset != 0 => {
                    let sign = if offset >= 0 { '+' } else { '-' };
                    let abs_mins = offset.unsigned_abs() / 60;
                    let oh = abs_mins / 60;
                    let om = abs_mins % 60;
                    if om == 0 {
                        format!(" UTC{sign}{oh}")
                    } else {
                        format!(" UTC{sign}{oh:02}:{om:02}")
                    }
                }
                _ => String::new(),
            };
            let hh_mm = format!("{hour:02}:{minute:02}");
            let time_part = format!("{hh_mm}{tz_suffix}");
            let full = match lang {
                Language::French => format!("{date_part} à {time_part}"),
                Language::English => format!("{date_part} at {time_part}"),
            };
            let table_short = format!("{short_date_only} · {hh_mm}");
            return Some(FormattedEmailDate {
                full,
                date_part,
                time_part,
                table_short,
            });
        }

        return Some(FormattedEmailDate {
            full: date_part.clone(),
            date_part,
            time_part: String::new(),
            table_short: short_date_only,
        });
    }

    // Fallback for non-standard date strings
    let cleaned = s.replace('T', " ");
    Some(FormattedEmailDate {
        full: cleaned.clone(),
        date_part: cleaned.clone(),
        time_part: String::new(),
        table_short: cleaned,
    })
}

fn parse_any_datetime(s: &str) -> Option<ParsedTimestamp> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(ParsedTimestamp {
            date: dt.date_naive(),
            hour_minute: Some((dt.hour(), dt.minute())),
            tz_offset_secs: Some(dt.offset().local_minus_utc()),
        });
    }
    if let Ok(dt) = DateTime::parse_from_rfc2822(s) {
        return Some(ParsedTimestamp {
            date: dt.date_naive(),
            hour_minute: Some((dt.hour(), dt.minute())),
            tz_offset_secs: Some(dt.offset().local_minus_utc()),
        });
    }
    for fmt in [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M",
        "%Y/%m/%d %H:%M:%S",
        "%Y/%m/%d %H:%M",
        "%d/%m/%Y %H:%M:%S",
        "%d/%m/%Y %H:%M",
    ] {
        if let Ok(ndt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(ParsedTimestamp {
                date: ndt.date(),
                hour_minute: Some((ndt.hour(), ndt.minute())),
                tz_offset_secs: None,
            });
        }
    }
    for fmt in ["%Y-%m-%d", "%Y/%m/%d", "%d/%m/%Y"] {
        if let Ok(nd) = NaiveDate::parse_from_str(s, fmt) {
            return Some(ParsedTimestamp {
                date: nd,
                hour_minute: None,
                tz_offset_secs: None,
            });
        }
    }
    None
}

/// Returns `true` if `raw` appears to contain HTML markup rather than plain text.
pub fn is_html_email(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() || !trimmed.contains('<') || !trimmed.contains('>') {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    const HTML_MARKERS: &[&str] = &[
        "<!doctype html",
        "<html",
        "<body",
        "<head",
        "<div",
        "<p>",
        "<p ",
        "<span",
        "<table",
        "<thead",
        "<tbody",
        "<tr",
        "<td",
        "<th",
        "<br>",
        "<br/>",
        "<br />",
        "<br ",
        "<hr>",
        "<hr/>",
        "<hr />",
        "<ul",
        "<ol",
        "<li",
        "<blockquote",
        "<style",
        "<font",
        "<center",
        "<strong",
        "<em>",
        "<b>",
        "<i>",
        "<u>",
        "<h1",
        "<h2",
        "<h3",
        "<h4",
        "<h5",
        "<h6",
        "<a href=",
        "<img ",
    ];

    HTML_MARKERS.iter().any(|marker| lower.contains(marker))
}

/// Formats an email body (either raw HTML or plain text) into safe, styled HTML
/// suitable for rendering in the reading pane.
pub fn format_email_body_html(raw: &str) -> String {
    let repaired = repair_text_encoding(raw);
    let trimmed = repaired.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if is_html_email(trimmed) {
        sanitize_html_email(trimmed)
    } else {
        format_plain_text_email_html(trimmed)
    }
}

/// Converts an HTML email body into clean plain text for search excerpts and LLM context.
/// If the input is already plain text, returns it unchanged.
pub fn html_to_plain_text(raw: &str) -> String {
    let repaired = repair_text_encoding(raw);
    let raw = repaired.as_str();
    if !is_html_email(raw) {
        return repaired;
    }

    let cleaned = strip_ignored_blocks(raw);
    let mut out = String::with_capacity(cleaned.len());
    let mut chars = cleaned.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '<' {
            let mut tag_content = String::new();
            for next_ch in chars.by_ref() {
                if next_ch == '>' {
                    break;
                }
                tag_content.push(next_ch);
            }
            let tag_name = extract_tag_name(&tag_content);
            let is_closing = tag_content.trim_start().starts_with('/');
            match tag_name.as_str() {
                "br" => out.push('\n'),
                "hr" if !is_closing => {
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str("---\n");
                }
                "li" if !is_closing => {
                    if !out.ends_with('\n') && !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str("- ");
                }
                "p" | "div" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote"
                | "li" | "table" | "ul" | "ol" => {
                    if !out.ends_with('\n') && !out.is_empty() {
                        out.push('\n');
                    }
                }
                "td" | "th" if is_closing => {
                    out.push('\t');
                }
                _ => {}
            }
        } else {
            out.push(ch);
        }
    }

    let decoded = decode_html_entities(&out);
    let mut lines_out = Vec::new();
    let mut empty_run = 0usize;
    for line in decoded.lines() {
        let trimmed_end = line.trim_end();
        if trimmed_end.trim().is_empty() {
            empty_run += 1;
            if empty_run <= 1 && !lines_out.is_empty() {
                lines_out.push(String::new());
            }
        } else {
            empty_run = 0;
            lines_out.push(trimmed_end.to_string());
        }
    }
    while lines_out.last().is_some_and(|l| l.is_empty()) {
        lines_out.pop();
    }
    lines_out.join("\n")
}

/// Sanitizes raw HTML email markup so it is safe and readable inside the application's dark theme.
pub fn sanitize_html_email(raw: &str) -> String {
    let repaired = repair_text_encoding(raw);
    let without_blocks = strip_ignored_blocks(&repaired);
    let body_slice = extract_body_inner(&without_blocks);

    let mut out = String::with_capacity(body_slice.len());
    let mut chars = body_slice.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '<' {
            let mut raw_tag = String::new();
            let mut in_quote: Option<char> = None;
            for next_ch in chars.by_ref() {
                if let Some(q) = in_quote {
                    if next_ch == q {
                        in_quote = None;
                    }
                    raw_tag.push(next_ch);
                } else if next_ch == '"' || next_ch == '\'' {
                    in_quote = Some(next_ch);
                    raw_tag.push(next_ch);
                } else if next_ch == '>' {
                    break;
                } else {
                    raw_tag.push(next_ch);
                }
            }
            out.push_str(&sanitize_tag(&raw_tag));
        } else {
            out.push(ch);
        }
    }

    out.trim().to_string()
}

fn strip_ignored_blocks(input: &str) -> String {
    let mut result = strip_html_comments(input);
    for tag in [
        "script", "style", "head", "title", "iframe", "object", "embed", "applet", "xml", "svg",
    ] {
        result = remove_element_with_content(&result, tag);
    }
    result
}

fn strip_html_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        let after_start = &rest[start + 4..];
        if let Some(end) = after_start.find("-->") {
            rest = &after_start[end + 3..];
        } else {
            return out;
        }
    }
    out.push_str(rest);
    out
}

fn remove_element_with_content(input: &str, tag_lower: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut idx = 0;
    let bytes = input.as_bytes();
    let open_pat = format!("<{tag_lower}");
    let close_pat = format!("</{tag_lower}");

    while idx < input.len() {
        let rem = &input[idx..];
        let rem_lower = rem.to_ascii_lowercase();
        let Some(rel_open) = rem_lower.find(&open_pat) else {
            out.push_str(rem);
            break;
        };

        out.push_str(&rem[..rel_open]);
        let tag_start = idx + rel_open;
        let after_name_idx = tag_start + open_pat.len();

        // Ensure it's actually this tag and not a prefix of another tag (e.g., <header vs <head)
        if after_name_idx < bytes.len() {
            let next_b = bytes[after_name_idx];
            if next_b.is_ascii_alphanumeric() || next_b == b'-' || next_b == b':' {
                out.push_str(&input[tag_start..after_name_idx]);
                idx = after_name_idx;
                continue;
            }
        }

        let after_open = &input[after_name_idx..];
        // Check if self-closing `<tag ... />`
        if let Some(gt_pos) = after_open.find('>') {
            let open_tag_body = &after_open[..gt_pos];
            if open_tag_body.trim_end().ends_with('/') {
                idx = after_name_idx + gt_pos + 1;
                continue;
            }
            let content_start = after_name_idx + gt_pos + 1;
            let content_rem_lower = input[content_start..].to_ascii_lowercase();
            if let Some(rel_close) = content_rem_lower.find(&close_pat) {
                let close_start = content_start + rel_close;
                if let Some(close_gt) = input[close_start..].find('>') {
                    idx = close_start + close_gt + 1;
                    continue;
                }
            }
            // Unclosed ignored block: drop to end
            break;
        } else {
            break;
        }
    }
    out
}

fn extract_body_inner(input: &str) -> &str {
    let lower = input.to_ascii_lowercase();
    let start = if let Some(body_pos) = lower.find("<body") {
        let after_body = body_pos + 5;
        if input
            .as_bytes()
            .get(after_body)
            .is_none_or(|b| !b.is_ascii_alphanumeric())
        {
            input[after_body..]
                .find('>')
                .map(|gt| after_body + gt + 1)
                .unwrap_or(0)
        } else {
            0
        }
    } else {
        0
    };

    let rem = &input[start..];
    let rem_lower = rem.to_ascii_lowercase();
    if let Some(end_pos) = rem_lower.rfind("</body") {
        &rem[..end_pos]
    } else {
        rem
    }
}

fn extract_tag_name(raw_tag: &str) -> String {
    let trimmed = raw_tag.trim().trim_start_matches('/').trim_start();
    trimmed
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == ':' || *c == '-')
        .collect::<String>()
        .to_ascii_lowercase()
}

fn sanitize_tag(raw_tag: &str) -> String {
    let trimmed = raw_tag.trim();
    if trimmed.is_empty() || trimmed.starts_with('!') || trimmed.starts_with('?') {
        return String::new();
    }

    let is_closing = trimmed.starts_with('/');
    let inner = if is_closing {
        trimmed[1..].trim_start()
    } else {
        trimmed
    };

    let tag_name: String = inner
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == ':' || *c == '-')
        .collect::<String>()
        .to_ascii_lowercase();

    if tag_name.is_empty() || tag_name.contains(':') {
        // Strip XML/Office namespace tags such as <o:p>, <v:shape>, <w:WordDocument>
        return String::new();
    }

    let mapped_tag = match tag_name.as_str() {
        "font" => "span",
        "center" => "div",
        other => other,
    };

    const ALLOWED_TAGS: &[&str] = &[
        "div",
        "p",
        "span",
        "br",
        "hr",
        "a",
        "b",
        "strong",
        "i",
        "em",
        "u",
        "s",
        "strike",
        "del",
        "ins",
        "sub",
        "sup",
        "small",
        "mark",
        "code",
        "pre",
        "blockquote",
        "ul",
        "ol",
        "li",
        "dl",
        "dt",
        "dd",
        "table",
        "caption",
        "thead",
        "tbody",
        "tfoot",
        "tr",
        "th",
        "td",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "img",
        "details",
        "summary",
        "section",
        "article",
        "header",
        "footer",
    ];

    if !ALLOWED_TAGS.contains(&mapped_tag) {
        return String::new();
    }

    if is_closing {
        if matches!(mapped_tag, "br" | "hr" | "img") {
            return String::new();
        }
        return format!("</{mapped_tag}>");
    }

    let attr_str = inner[tag_name.len()..].trim().trim_end_matches('/').trim();
    let attrs = parse_html_attributes(attr_str);

    if mapped_tag == "img" {
        return sanitize_img_tag(&attrs);
    }

    if mapped_tag == "br" {
        return "<br>".to_string();
    }

    if mapped_tag == "hr" {
        return "<hr class=\"email-divider\">".to_string();
    }

    let mut clean_attrs = Vec::new();

    if mapped_tag == "a" {
        if let Some(href) = attrs
            .iter()
            .find(|(k, _)| k == "href")
            .map(|(_, v)| v.trim())
            && is_safe_href(href)
        {
            clean_attrs.push(format!("href=\"{}\"", escape_html_attr(href)));
            clean_attrs.push("class=\"email-link\"".to_string());
            clean_attrs.push("target=\"_blank\"".to_string());
            clean_attrs.push("rel=\"noopener noreferrer\"".to_string());
        }
    } else if mapped_tag == "blockquote" {
        clean_attrs.push("class=\"email-quote\"".to_string());
    } else if matches!(mapped_tag, "td" | "th") {
        for (k, v) in &attrs {
            if matches!(k.as_str(), "colspan" | "rowspan")
                && v.chars().all(|c| c.is_ascii_digit())
                && !v.is_empty()
            {
                clean_attrs.push(format!("{k}=\"{v}\""));
            }
        }
    }

    let mut style_rules = Vec::new();
    if tag_name == "center" {
        style_rules.push("text-align: center".to_string());
    }
    if let Some((_, raw_style)) = attrs.iter().find(|(k, _)| k == "style") {
        let sanitized_style = sanitize_inline_style(raw_style);
        if !sanitized_style.is_empty() {
            style_rules.push(sanitized_style);
        }
    }
    if !style_rules.is_empty() {
        clean_attrs.push(format!(
            "style=\"{}\"",
            escape_html_attr(&style_rules.join("; "))
        ));
    }

    if clean_attrs.is_empty() {
        format!("<{mapped_tag}>")
    } else {
        format!("<{mapped_tag} {}>", clean_attrs.join(" "))
    }
}

fn sanitize_img_tag(attrs: &[(String, String)]) -> String {
    let src = attrs
        .iter()
        .find(|(k, _)| k == "src")
        .map(|(_, v)| v.trim())
        .unwrap_or("");
    let alt = attrs
        .iter()
        .find(|(k, _)| k == "alt")
        .map(|(_, v)| v.trim())
        .unwrap_or("");
    let width = attrs
        .iter()
        .find(|(k, _)| k == "width")
        .map(|(_, v)| v.trim())
        .unwrap_or("");
    let height = attrs
        .iter()
        .find(|(k, _)| k == "height")
        .map(|(_, v)| v.trim())
        .unwrap_or("");

    // Drop 1x1 tracking pixels
    if width == "1" || height == "1" || width == "0" || height == "0" {
        return String::new();
    }

    let src_lower = src.to_ascii_lowercase();
    if src_lower.starts_with("cid:") {
        let label = if !alt.is_empty() {
            alt
        } else {
            src.get(4..).unwrap_or("image")
        };
        let label_clean = label.split('@').next().unwrap_or(label);
        return format!(
            "<span class=\"email-cid-chip\">📎 {}</span>",
            escape_html(label_clean)
        );
    }

    // Only allow self-contained inline `data:image/` URIs; external `http(s)://` images are
    // blocked so opening an email in the WebView never makes automatic outbound network requests.
    if src_lower.starts_with("data:image/") {
        let mut img_attrs = vec![
            format!("src=\"{}\"", escape_html_attr(src)),
            "class=\"email-inline-img\"".to_string(),
            "loading=\"lazy\"".to_string(),
        ];
        if !alt.is_empty() {
            img_attrs.push(format!("alt=\"{}\"", escape_html_attr(alt)));
        }
        return format!("<img {}>", img_attrs.join(" "));
    }

    String::new()
}

fn is_safe_href(href: &str) -> bool {
    let clean: String = href
        .chars()
        .filter(|c| !c.is_ascii_control() && !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    clean.starts_with("https://")
        || clean.starts_with("http://")
        || clean.starts_with("mailto:")
        || clean.starts_with('#')
}

fn sanitize_inline_style(raw_style: &str) -> String {
    let mut allowed = Vec::new();
    for decl in raw_style.split(';') {
        let Some((prop, val)) = decl.split_once(':') else {
            continue;
        };
        let prop_clean = prop.trim().to_ascii_lowercase();
        let val_clean = val.trim();
        let val_lower = val_clean.to_ascii_lowercase();

        if val_lower.contains("url(")
            || val_lower.contains("expression(")
            || val_lower.contains("javascript:")
        {
            continue;
        }

        // Only keep layout/alignment/emphasis properties that do not clash with dark mode
        if matches!(
            prop_clean.as_str(),
            "text-align"
                | "font-weight"
                | "font-style"
                | "text-decoration"
                | "border-collapse"
                | "vertical-align"
        ) {
            allowed.push(format!("{prop_clean}: {val_clean}"));
        }
    }
    allowed.join("; ")
}

fn parse_html_attributes(attr_str: &str) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    let mut chars = attr_str.chars().peekable();

    while chars.peek().is_some() {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let mut name = String::new();
        while let Some(&c) = chars.peek() {
            if c == '=' || c.is_whitespace() || c == '/' || c == '>' {
                break;
            }
            name.push(c.to_ascii_lowercase());
            chars.next();
        }
        if name.is_empty() {
            chars.next();
            continue;
        }
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        if chars.peek() != Some(&'=') {
            attrs.push((name, String::new()));
            continue;
        }
        chars.next(); // consume '='
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let mut value = String::new();
        if let Some(&quote) = chars.peek()
            && (quote == '"' || quote == '\'')
        {
            chars.next();
            for c in chars.by_ref() {
                if c == quote {
                    break;
                }
                value.push(c);
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() || c == '>' {
                    break;
                }
                value.push(c);
                chars.next();
            }
        }
        attrs.push((name, decode_html_entities(&value)));
    }

    attrs
}

/// Converts a plain-text email body into rich semantic HTML with structured reply headers,
/// nested blockquotes, lists, clickable links, signatures, and legal disclaimer styling.
pub fn format_plain_text_email_html(raw: &str) -> String {
    let repaired = repair_text_encoding(raw);
    let normalized = repaired.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized.lines().map(|l| l.trim_end()).collect();
    format_plain_lines(&lines)
}

fn format_plain_lines(lines: &[&str]) -> String {
    let mut out = String::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // 1. Skip empty lines
        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        // 2. Check for "-----Original Message-----" / forwarded message banner + header block
        if is_reply_banner(trimmed) {
            let banner_title = trimmed
                .trim_matches(|c: char| c == '-' || c == '_' || c == '=' || c.is_whitespace());
            i += 1;
            // Skip blank lines right after banner
            while i < lines.len() && lines[i].trim().is_empty() {
                i += 1;
            }
            let (headers, next_i) = collect_reply_headers(lines, i);
            i = next_i;
            out.push_str(&render_reply_header_card(
                (!banner_title.is_empty()).then_some(banner_title),
                &headers,
            ));
            continue;
        }

        // 3. Check for standalone embedded header block (2+ consecutive From:/Sent:/To:/Subject: lines)
        let (headers, next_i) = collect_reply_headers(lines, i);
        if headers.len() >= 2 {
            i = next_i;
            out.push_str(&render_reply_header_card(None, &headers));
            continue;
        }

        // 4. Check for horizontal divider line (---, ===, ___)
        if is_horizontal_divider(trimmed) {
            out.push_str("<hr class=\"email-divider\">");
            i += 1;
            continue;
        }

        // 5. Check for quoted block (`> ...`)
        if trimmed.starts_with('>') {
            let mut quoted_inner: Vec<String> = Vec::new();
            while i < lines.len() {
                let cur = lines[i].trim();
                if let Some(stripped) = cur.strip_prefix('>') {
                    let after_gt = stripped.strip_prefix(' ').unwrap_or(stripped);
                    quoted_inner.push(after_gt.to_string());
                    i += 1;
                } else if cur.is_empty()
                    && i + 1 < lines.len()
                    && lines[i + 1].trim().starts_with('>')
                {
                    quoted_inner.push(String::new());
                    i += 1;
                } else {
                    break;
                }
            }
            let borrowed: Vec<&str> = quoted_inner.iter().map(String::as_str).collect();
            let inner_html = format_plain_lines(&borrowed);
            out.push_str("<blockquote class=\"email-quote\">");
            out.push_str(&inner_html);
            out.push_str("</blockquote>");
            continue;
        }

        // 6. Check for "On ... wrote:" / "Le ... a écrit :" attribution line
        if is_quote_attribution(trimmed) {
            out.push_str("<div class=\"email-quote-attribution\">");
            out.push_str(&format_inline_text(trimmed));
            out.push_str("</div>");
            i += 1;
            continue;
        }

        // 7. Check for unordered list (`- `, `* `, `• `)
        if let Some(first_item) = strip_unordered_marker(trimmed) {
            out.push_str("<ul class=\"email-list\">");
            out.push_str("<li>");
            out.push_str(&format_inline_text(first_item));
            out.push_str("</li>");
            i += 1;
            while i < lines.len() {
                if let Some(item) = strip_unordered_marker(lines[i].trim()) {
                    out.push_str("<li>");
                    out.push_str(&format_inline_text(item));
                    out.push_str("</li>");
                    i += 1;
                } else {
                    break;
                }
            }
            out.push_str("</ul>");
            continue;
        }

        // 8. Check for ordered list (`1. `, `2) `)
        if let Some(first_item) = strip_ordered_marker(trimmed) {
            out.push_str("<ol class=\"email-list\">");
            out.push_str("<li>");
            out.push_str(&format_inline_text(first_item));
            out.push_str("</li>");
            i += 1;
            while i < lines.len() {
                if let Some(item) = strip_ordered_marker(lines[i].trim()) {
                    out.push_str("<li>");
                    out.push_str(&format_inline_text(item));
                    out.push_str("</li>");
                    i += 1;
                } else {
                    break;
                }
            }
            out.push_str("</ol>");
            continue;
        }

        // 9. Check for signature delimiter (`--` or `-- `)
        if line == "-- " || trimmed == "--" {
            i += 1;
            let mut sig_lines = Vec::new();
            while i < lines.len() {
                let cur_trim = lines[i].trim();
                if is_reply_banner(cur_trim)
                    || is_legal_disclaimer(cur_trim)
                    || cur_trim.starts_with('>')
                {
                    break;
                }
                sig_lines.push(lines[i]);
                i += 1;
            }
            while sig_lines.last().is_some_and(|l| l.trim().is_empty()) {
                sig_lines.pop();
            }
            if !sig_lines.is_empty() {
                out.push_str("<div class=\"email-signature\">");
                for (idx, sline) in sig_lines.iter().enumerate() {
                    if idx > 0 {
                        out.push_str("<br>");
                    }
                    out.push_str(&format_inline_text(sline.trim()));
                }
                out.push_str("</div>");
            }
            continue;
        }

        // 10. Check for legal disclaimer footer
        if is_legal_disclaimer(trimmed) {
            let mut disc_lines = vec![trimmed];
            i += 1;
            while i < lines.len() && !lines[i].trim().is_empty() {
                disc_lines.push(lines[i].trim());
                i += 1;
            }
            out.push_str("<div class=\"email-disclaimer\">");
            out.push_str(&format_inline_text(&disc_lines.join(" ")));
            out.push_str("</div>");
            continue;
        }

        // 11. Regular paragraph
        let mut para_lines = vec![trimmed];
        i += 1;
        while i < lines.len() {
            let next_trim = lines[i].trim();
            if next_trim.is_empty()
                || is_reply_banner(next_trim)
                || is_horizontal_divider(next_trim)
                || next_trim.starts_with('>')
                || is_quote_attribution(next_trim)
                || strip_unordered_marker(next_trim).is_some()
                || strip_ordered_marker(next_trim).is_some()
                || lines[i] == "-- "
                || next_trim == "--"
                || is_legal_disclaimer(next_trim)
                || collect_reply_headers(lines, i).0.len() >= 2
            {
                break;
            }
            para_lines.push(next_trim);
            i += 1;
        }

        out.push_str("<p class=\"email-paragraph\">");
        for (idx, pline) in para_lines.iter().enumerate() {
            if idx > 0 {
                out.push_str("<br>");
            }
            out.push_str(&format_inline_text(pline));
        }
        out.push_str("</p>");
    }

    out
}

fn is_reply_banner(trimmed: &str) -> bool {
    if !trimmed.starts_with("---") && !trimmed.starts_with("___") {
        return false;
    }
    let lower = trimmed.to_lowercase();
    lower.contains("original message")
        || lower.contains("message d'origine")
        || lower.contains("forwarded message")
        || lower.contains("message transféré")
        || lower.contains("reply message")
}

fn is_horizontal_divider(trimmed: &str) -> bool {
    trimmed.len() >= 3
        && (trimmed.chars().all(|c| c == '-')
            || trimmed.chars().all(|c| c == '=')
            || trimmed.chars().all(|c| c == '_'))
}

fn is_quote_attribution(trimmed: &str) -> bool {
    if !trimmed.ends_with(':') {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    (lower.starts_with("on ") && lower.contains(" wrote:"))
        || (lower.starts_with("le ")
            && (lower.contains(" a écrit :") || lower.contains(" a écrit:")))
}

fn is_legal_disclaimer(trimmed: &str) -> bool {
    let lower = trimmed.to_lowercase();
    lower.starts_with("confidentiality notice")
        || lower.starts_with("this e-mail and any attachments")
        || lower.starts_with("this email and any attachments")
        || lower.starts_with("this email, including any attachments")
        || lower.starts_with("this message contains confidential information")
        || lower.starts_with("this communication is intended only for")
        || lower.starts_with("the information contained in this email")
        || lower.starts_with("privileged/confidential information")
        || lower.starts_with("please consider the environment before printing")
        || lower.starts_with("this message is for the designated recipient only")
        || lower.starts_with("ce message et toutes les pièces jointes")
        || lower.starts_with("avis de confidentialité")
}

fn strip_unordered_marker(trimmed: &str) -> Option<&str> {
    for prefix in ["- ", "* ", "• "] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let clean = rest.trim();
            if !clean.is_empty() {
                return Some(clean);
            }
        }
    }
    None
}

fn strip_ordered_marker(trimmed: &str) -> Option<&str> {
    let digit_count = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if digit_count == 0 || digit_count > 3 {
        return None;
    }
    let after_digits = &trimmed[digit_count..];
    if let Some(rest) = after_digits
        .strip_prefix(". ")
        .or_else(|| after_digits.strip_prefix(") "))
    {
        let clean = rest.trim();
        if !clean.is_empty() {
            return Some(clean);
        }
    }
    None
}

fn parse_reply_header_line(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim();
    let (key, val) = trimmed.split_once(':')?;
    let key_clean = key.trim();
    let val_clean = val.trim();
    if val_clean.is_empty() {
        return None;
    }
    let key_lower = key_clean.to_lowercase();
    if matches!(
        key_lower.as_str(),
        "from"
            | "de"
            | "sent"
            | "envoyé"
            | "date"
            | "to"
            | "à"
            | "a"
            | "cc"
            | "bcc"
            | "cci"
            | "subject"
            | "objet"
            | "sujet"
            | "importance"
    ) {
        Some((key_clean, val_clean))
    } else {
        None
    }
}

fn collect_reply_headers<'a>(lines: &[&'a str], start: usize) -> (Vec<(&'a str, &'a str)>, usize) {
    let mut headers = Vec::new();
    let mut idx = start;
    while idx < lines.len() {
        if let Some(kv) = parse_reply_header_line(lines[idx]) {
            headers.push(kv);
            idx += 1;
        } else {
            break;
        }
    }
    (headers, idx)
}

fn render_reply_header_card(title: Option<&str>, headers: &[(&str, &str)]) -> String {
    let mut html = String::from("<div class=\"email-reply-header\">");
    if let Some(t) = title {
        html.push_str("<div class=\"email-reply-header-title\">");
        html.push_str(&escape_html(t));
        html.push_str("</div>");
    }
    if !headers.is_empty() {
        html.push_str("<div class=\"email-reply-meta-grid\">");
        for (k, v) in headers {
            html.push_str("<div class=\"email-reply-meta-row\">");
            html.push_str("<span class=\"email-reply-key\">");
            html.push_str(&escape_html(k));
            html.push_str("</span>");
            html.push_str("<span class=\"email-reply-val\">");
            html.push_str(&format_inline_text(v));
            html.push_str("</span>");
            html.push_str("</div>");
        }
        html.push_str("</div>");
    }
    html.push_str("</div>");
    html
}

/// Formats inline text by escaping HTML special characters and auto-linking URLs and emails.
pub fn format_inline_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut idx = 0;

    while idx < text.len() {
        let rem = &text[idx..];

        // Check if a URL starts at `rem`
        if rem.starts_with("https://") || rem.starts_with("http://") {
            let raw_len = rem
                .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\''))
                .unwrap_or(rem.len());
            let raw_url = &rem[..raw_len];
            let clean_url = raw_url.trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']']);
            if clean_url.len() > 8 {
                let escaped_url = escape_html_attr(clean_url);
                out.push_str(&format!(
                    "<a class=\"email-link\" href=\"{escaped_url}\" target=\"_blank\" rel=\"noopener noreferrer\">{}</a>",
                    escape_html(clean_url)
                ));
                idx += clean_url.len();
                continue;
            }
        }

        // Check if an email token starts at `rem` (when at start or preceded by non-word char)
        let prev_char = text[..idx].chars().next_back();
        if prev_char.is_none_or(|c| {
            c.is_whitespace() || matches!(c, '<' | '(' | '[' | ',' | ';' | ':' | '"')
        }) && let Some(email_len) = match_email_address(rem)
        {
            let addr = &rem[..email_len];
            let escaped_addr = escape_html_attr(addr);
            out.push_str(&format!(
                "<a class=\"email-address-link\" href=\"mailto:{escaped_addr}\">{}</a>",
                escape_html(addr)
            ));
            idx += email_len;
            continue;
        }

        let ch = rem.chars().next().unwrap();
        push_escaped_char(&mut out, ch);
        idx += ch.len_utf8();
    }

    out
}

fn match_email_address(rem: &str) -> Option<usize> {
    let token_end = rem
        .find(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    '<' | '>' | '(' | ')' | '[' | ']' | ',' | ';' | '"' | '\''
                )
        })
        .unwrap_or(rem.len());
    let candidate = rem[..token_end].trim_end_matches(['.', ':', '!', '?']);
    let (local, domain) = candidate.split_once('@')?;
    if local.is_empty() || domain.len() < 3 || !domain.contains('.') {
        return None;
    }
    let valid_local = local
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'));
    let valid_domain = domain
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
        && !domain.starts_with('.')
        && !domain.ends_with('.');
    if valid_local && valid_domain {
        Some(candidate.len())
    } else {
        None
    }
}

pub fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        push_escaped_char(&mut out, ch);
    }
    out
}

fn escape_html_attr(input: &str) -> String {
    escape_html(input)
}

fn push_escaped_char(out: &mut String, ch: char) {
    match ch {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '"' => out.push_str("&quot;"),
        '\'' => out.push_str("&#39;"),
        _ => out.push(ch),
    }
}

pub fn decode_html_entities(input: &str) -> String {
    if !input.contains('&') {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(amp_idx) = rest.find('&') {
        out.push_str(&rest[..amp_idx]);
        let after_amp = &rest[amp_idx + 1..];
        if let Some(semi_idx) = after_amp.find(';')
            && semi_idx <= 10
        {
            let entity = &after_amp[..semi_idx];
            if let Some(decoded) = resolve_html_entity(entity) {
                out.push_str(&decoded);
                rest = &after_amp[semi_idx + 1..];
                continue;
            }
        }
        out.push('&');
        rest = after_amp;
    }
    out.push_str(rest);
    out
}

fn resolve_html_entity(entity: &str) -> Option<String> {
    if let Some(hex) = entity
        .strip_prefix("#x")
        .or_else(|| entity.strip_prefix("#X"))
    {
        let code = u32::from_str_radix(hex, 16).ok()?;
        return decode_numeric_entity(code).map(|ch| ch.to_string());
    }
    if let Some(dec) = entity.strip_prefix('#') {
        let code = dec.parse::<u32>().ok()?;
        return decode_numeric_entity(code).map(|ch| ch.to_string());
    }
    let ch = match entity {
        "nbsp" | "ensp" | "emsp" | "thinsp" => " ",
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "ndash" => "–",
        "mdash" => "—",
        "hellip" => "…",
        "rsquo" | "lsquo" => "’",
        "rdquo" | "ldquo" => "”",
        "sbquo" => "‚",
        "bdquo" => "„",
        "lsaquo" => "‹",
        "rsaquo" => "›",
        "laquo" => "«",
        "raquo" => "»",
        "bull" => "•",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "euro" => "€",
        "pound" => "£",
        "yen" => "¥",
        "cent" => "¢",
        "curren" => "¤",
        "deg" => "°",
        "ordm" => "º",
        "ordf" => "ª",
        "sect" => "§",
        "para" => "¶",
        "middot" => "·",
        "iexcl" => "¡",
        "iquest" => "¿",
        "times" => "×",
        "divide" => "÷",
        "plusmn" => "±",
        "sup1" => "¹",
        "sup2" => "²",
        "sup3" => "³",
        "frac14" => "¼",
        "frac12" => "½",
        "frac34" => "¾",
        "micro" => "µ",
        "brvbar" => "¦",
        "not" => "¬",
        "macr" => "¯",
        "acute" => "´",
        "cedil" => "¸",
        "uml" => "¨",
        "dagger" => "†",
        "Dagger" => "‡",
        "permil" => "‰",
        "prime" => "′",
        "Prime" => "″",
        "shy" | "zwnj" | "zwj" | "lrm" | "rlm" => "",
        // Latin-1 & French accented characters
        "Agrave" => "À",
        "Aacute" => "Á",
        "Acirc" => "Â",
        "Atilde" => "Ã",
        "Auml" => "Ä",
        "Aring" => "Å",
        "AElig" => "Æ",
        "Ccedil" => "Ç",
        "Egrave" => "È",
        "Eacute" => "É",
        "Ecirc" => "Ê",
        "Euml" => "Ë",
        "Igrave" => "Ì",
        "Iacute" => "Í",
        "Icirc" => "Î",
        "Iuml" => "Ï",
        "ETH" => "Ð",
        "Ntilde" => "Ñ",
        "Ograve" => "Ò",
        "Oacute" => "Ó",
        "Ocirc" => "Ô",
        "Otilde" => "Õ",
        "Ouml" => "Ö",
        "Oslash" => "Ø",
        "Ugrave" => "Ù",
        "Uacute" => "Ú",
        "Ucirc" => "Û",
        "Uuml" => "Ü",
        "Yacute" => "Ý",
        "THORN" => "Þ",
        "szlig" => "ß",
        "agrave" => "à",
        "aacute" => "á",
        "acirc" => "â",
        "atilde" => "ã",
        "auml" => "ä",
        "aring" => "å",
        "aelig" => "æ",
        "ccedil" => "ç",
        "egrave" => "è",
        "eacute" => "é",
        "ecirc" => "ê",
        "euml" => "ë",
        "igrave" => "ì",
        "iacute" => "í",
        "icirc" => "î",
        "iuml" => "ï",
        "eth" => "ð",
        "ntilde" => "ñ",
        "ograve" => "ò",
        "oacute" => "ó",
        "ocirc" => "ô",
        "otilde" => "õ",
        "ouml" => "ö",
        "oslash" => "ø",
        "ugrave" => "ù",
        "uacute" => "ú",
        "ucirc" => "û",
        "uuml" => "ü",
        "yacute" => "ý",
        "thorn" => "þ",
        "yuml" => "ÿ",
        // Latin Extended-A (French ligatures & common European characters)
        "OElig" => "Œ",
        "oelig" => "œ",
        "Scaron" => "Š",
        "scaron" => "š",
        "Yuml" => "Ÿ",
        _ => return None,
    };
    Some(ch.to_string())
}

fn decode_numeric_entity(code: u32) -> Option<char> {
    if code == 0 {
        return None;
    }
    if code == 160 {
        return Some(' ');
    }
    if (0x80..=0x9F).contains(&code)
        && let Some(ch) = win1252_c1_char(code as u8)
    {
        return Some(ch);
    }
    char::from_u32(code)
}

const WIN1252_C1: [Option<char>; 32] = [
    Some('€'), // 0x80
    None,      // 0x81
    Some('‚'), // 0x82
    Some('ƒ'), // 0x83
    Some('„'), // 0x84
    Some('…'), // 0x85
    Some('†'), // 0x86
    Some('‡'), // 0x87
    Some('ˆ'), // 0x88
    Some('‰'), // 0x89
    Some('Š'), // 0x8A
    Some('‹'), // 0x8B
    Some('Œ'), // 0x8C
    None,      // 0x8D
    Some('Ž'), // 0x8E
    None,      // 0x8F
    None,      // 0x90
    Some('‘'), // 0x91
    Some('’'), // 0x92
    Some('“'), // 0x93
    Some('”'), // 0x94
    Some('•'), // 0x95
    Some('–'), // 0x96
    Some('—'), // 0x97
    Some('˜'), // 0x98
    Some('™'), // 0x99
    Some('š'), // 0x9A
    Some('›'), // 0x9B
    Some('œ'), // 0x9C
    None,      // 0x9D
    Some('ž'), // 0x9E
    Some('Ÿ'), // 0x9F
];

fn win1252_c1_char(byte: u8) -> Option<char> {
    (0x80..=0x9F)
        .contains(&byte)
        .then(|| WIN1252_C1[(byte - 0x80) as usize])
        .flatten()
}

fn char_to_win1252_byte(ch: char) -> Option<u8> {
    let cp = ch as u32;
    if cp <= 0xFF {
        return Some(cp as u8);
    }
    for (idx, mapped) in WIN1252_C1.iter().enumerate() {
        if *mapped == Some(ch) {
            return Some(0x80 + idx as u8);
        }
    }
    None
}

fn map_c1_and_strip_nul(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch == '\0' {
            continue;
        }
        let cp = ch as u32;
        if (0x80..=0x9F).contains(&cp) {
            if let Some(mapped) = win1252_c1_char(cp as u8) {
                out.push(mapped);
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Repairs common PST/email encoding artifacts without altering valid UTF-8 text:
/// 1. Recovers UTF-8 text that was previously decoded byte-for-byte as Latin-1 or Windows-1252
///    (e.g., `"Ã©"` -> `"é"`, `"Ã "` -> `"à"`, `"â€™"` -> `"’"`).
/// 2. Maps C1 control characters (`U+0080..=U+009F`) to their Windows-1252 characters (`€`, `’`, `œ`, …).
pub fn repair_text_encoding(input: &str) -> String {
    if input.is_ascii() && !input.contains('\0') {
        return input.to_string();
    }

    // Fast path: whole string is representable as Latin-1/Windows-1252 single bytes and
    // forms valid UTF-8 containing multi-byte sequences.
    let mut raw_bytes = Vec::with_capacity(input.len());
    let mut all_single_byte = true;
    for ch in input.chars() {
        if ch == '\0' {
            continue;
        }
        if let Some(b) = char_to_win1252_byte(ch) {
            raw_bytes.push(b);
        } else {
            all_single_byte = false;
            break;
        }
    }
    if all_single_byte
        && raw_bytes.iter().any(|&b| b >= 0x80)
        && let Ok(recovered) = std::str::from_utf8(&raw_bytes)
        && recovered != input
    {
        return map_c1_and_strip_nul(recovered);
    }

    // Fallback for mixed strings (e.g., valid UTF-8 headers around a mojibake quoted section):
    // collapse contiguous 2–4 byte UTF-8 mojibake runs and normalize stray C1 control chars.
    let chars: Vec<char> = input.chars().filter(|&c| c != '\0').collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if let Some(lead) = char_to_win1252_byte(ch)
            && (0xC2..=0xF4).contains(&lead)
        {
            let expected_len = if lead < 0xE0 {
                2
            } else if lead < 0xF0 {
                3
            } else {
                4
            };
            if i + expected_len <= chars.len() {
                let mut seq = [0u8; 4];
                seq[0] = lead;
                let mut ok = true;
                for offset in 1..expected_len {
                    if let Some(cont) = char_to_win1252_byte(chars[i + offset])
                        && (0x80..=0xBF).contains(&cont)
                    {
                        seq[offset] = cont;
                    } else {
                        ok = false;
                        break;
                    }
                }
                if ok
                    && let Ok(s) = std::str::from_utf8(&seq[..expected_len])
                    && let Some(decoded_ch) = s.chars().next()
                    && !((0x80..=0x9F).contains(&(decoded_ch as u32)))
                {
                    out.push(decoded_ch);
                    i += expected_len;
                    continue;
                }
            }
        }

        let cp = ch as u32;
        if (0x80..=0x9F).contains(&cp) {
            if let Some(mapped) = win1252_c1_char(cp as u8) {
                out.push(mapped);
            }
        } else {
            out.push(ch);
        }
        i += 1;
    }
    out
}
