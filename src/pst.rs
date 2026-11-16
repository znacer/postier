use std::{
    collections::HashSet,
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
    rc::Rc,
};

use crate::{email_html::repair_text_encoding, model::Email};
use anyhow::{Context, Result, bail};
use encoding_rs::Encoding;
use outlook_pst::{
    ltp::{prop_context::PropertyValue, table_context::TableContext},
    messaging::{
        folder::Folder,
        store::{EntryId, Store},
    },
    ndb::node_id::NodeId,
};

pub const IMPORT_BATCH_SIZE: usize = 500;
const MAX_FOLDER_DEPTH: usize = 64;

pub trait MailSource {
    fn read_messages(&self, path: &Path) -> Result<Vec<Email>> {
        let mut all = Vec::new();
        self.stream_messages(path, &mut |batch| {
            all.extend(batch);
            Ok(())
        })?;
        Ok(all)
    }

    fn stream_messages(
        &self,
        path: &Path,
        on_batch: &mut dyn FnMut(Vec<Email>) -> Result<()>,
    ) -> Result<usize>;
}

/// Development importer. One serialized `Email` per line permits end-to-end testing
/// without handling real mail outside the workstation.
pub struct JsonlSource;

impl MailSource for JsonlSource {
    fn stream_messages(
        &self,
        path: &Path,
        on_batch: &mut dyn FnMut(Vec<Email>) -> Result<()>,
    ) -> Result<usize> {
        let source = path.to_string_lossy().into_owned();
        let mut batch = Vec::with_capacity(IMPORT_BATCH_SIZE);
        let mut total = 0usize;
        for (line_no, line) in BufReader::new(File::open(path)?).lines().enumerate() {
            let mut email: Email = serde_json::from_str(&line?)
                .with_context(|| format!("invalid message on line {}", line_no + 1))?;
            email.source_key = source.clone();
            batch.push(email);
            total += 1;
            if batch.len() >= IMPORT_BATCH_SIZE {
                on_batch(std::mem::replace(
                    &mut batch,
                    Vec::with_capacity(IMPORT_BATCH_SIZE),
                ))?;
            }
        }
        if !batch.is_empty() {
            on_batch(batch)?;
        }
        Ok(total)
    }
}

pub struct PstSource;

impl MailSource for PstSource {
    fn stream_messages(
        &self,
        path: &Path,
        on_batch: &mut dyn FnMut(Vec<Email>) -> Result<()>,
    ) -> Result<usize> {
        // outlook-pst opens the archive read-only and supports both ANSI and Unicode PSTs.
        let store = outlook_pst::open_store(path).context("open PST read-only")?;
        let root_id = store
            .properties()
            .ipm_sub_tree_entry_id()
            .context("locate the PST mail subtree")?;
        let root = store
            .open_folder(&root_id)
            .context("open the PST mail subtree")?;
        let mut batch = Vec::with_capacity(IMPORT_BATCH_SIZE);
        let mut total = 0usize;
        let mut skipped = 0usize;
        let mut visited = HashSet::new();
        visited.insert(u32::from(root_id.node_id()));
        walk_folder(
            store,
            root,
            String::new(),
            &path.to_string_lossy(),
            0,
            &mut visited,
            &mut batch,
            &mut total,
            &mut skipped,
            on_batch,
        )?;
        if !batch.is_empty() {
            on_batch(batch)?;
        }
        #[cfg(debug_assertions)]
        if skipped > 0 {
            eprintln!("[pstbeetle] PST import skipped {skipped} unreadable message(s)");
        }
        Ok(total)
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_folder(
    store: Rc<dyn Store>,
    folder: Rc<dyn Folder>,
    parent: String,
    source: &str,
    depth: usize,
    visited: &mut HashSet<u32>,
    batch: &mut Vec<Email>,
    total: &mut usize,
    skipped: &mut usize,
    on_batch: &mut dyn FnMut(Vec<Email>) -> Result<()>,
) -> Result<()> {
    if depth > MAX_FOLDER_DEPTH {
        return Ok(());
    }
    let folder_props = folder.properties();
    let folder_cp =
        codepage_prop(folder_props.get(0x3FFD)).or_else(|| codepage_prop(folder_props.get(0x3FDE)));
    let name = text_prop(folder_props.get(0x3001), folder_cp)
        .or_else(|| {
            folder_props
                .display_name()
                .ok()
                .map(|s| repair_text_encoding(&s))
        })
        .unwrap_or_else(|| "Unnamed".into());
    let path = if parent.is_empty() {
        name
    } else {
        format!("{parent}/{name}")
    };
    if let Some(contents) = folder.contents_table() {
        for row in contents.rows_matrix() {
            match read_message(store.clone(), row, &path, source) {
                Ok(email) => {
                    batch.push(email);
                    *total += 1;
                    if batch.len() >= IMPORT_BATCH_SIZE {
                        on_batch(std::mem::replace(
                            batch,
                            Vec::with_capacity(IMPORT_BATCH_SIZE),
                        ))?;
                    }
                }
                Err(_) => {
                    *skipped += 1;
                }
            }
        }
    }
    if let Some(children) = folder.hierarchy_table() {
        for row in children.rows_matrix() {
            let raw_id = u32::from(row.id());
            if !visited.insert(raw_id) {
                continue;
            }
            let node = NodeId::from(raw_id);
            let entry_id = store.properties().make_entry_id(node)?;
            if let Ok(child) = store.open_folder(&entry_id) {
                walk_folder(
                    store.clone(),
                    child,
                    path.clone(),
                    source,
                    depth + 1,
                    visited,
                    batch,
                    total,
                    skipped,
                    on_batch,
                )?;
            }
        }
    }
    Ok(())
}

fn read_message(
    store: Rc<dyn Store>,
    row: &outlook_pst::ltp::table_context::TableRowData,
    folder: &str,
    source: &str,
) -> Result<Email> {
    let node = NodeId::from(u32::from(row.id()));
    let entry_id: EntryId = store.properties().make_entry_id(node)?;
    let message = store.open_message(&entry_id, None)?;
    let props = message.properties();
    let msg_codepage = codepage_prop(props.get(0x3FFD));
    let inet_codepage = codepage_prop(props.get(0x3FDE));
    let text_cp = msg_codepage.or(inet_codepage);
    let html_cp = inet_codepage.or(msg_codepage);

    let subject = text_prop(props.get(0x0037), text_cp)
        .map(|s| decode_rfc2047(&s))
        .unwrap_or_default();
    let sender_name = text_prop(props.get(0x0C1A), text_cp).map(|s| decode_rfc2047(&s));
    let sender_email = text_prop(props.get(0x5D01), text_cp)
        .or_else(|| text_prop(props.get(0x0C1F), text_cp).filter(|s| s.contains('@')));
    let sender = format_name_and_email(sender_name.as_deref(), sender_email.as_deref());
    let message_id = text_prop(props.get(0x1035), text_cp)
        .unwrap_or_else(|| format!("pst-node-{}", u32::from(row.id())));
    let conversation_id = extract_conversation_id(props, &subject, text_cp);
    let sent_at = time_prop(props.get(0x0039)).or_else(|| time_prop(props.get(0x0E06)));
    let body = html_body_prop(props.get(0x1013), html_cp)
        .or_else(|| text_prop(props.get(0x1000), text_cp))
        .unwrap_or_default();
    let display_to = text_prop(props.get(0x0E04), text_cp).map(|s| decode_rfc2047(&s));
    let display_cc = text_prop(props.get(0x0E03), text_cp).map(|s| decode_rfc2047(&s));
    let recipients = message
        .recipient_table()
        .map(|t| {
            extract_recipients(
                t.as_ref(),
                display_to.as_deref(),
                display_cc.as_deref(),
                text_cp,
            )
        })
        .unwrap_or_else(|| combine_to_and_cc(display_to.as_deref(), display_cc.as_deref()));
    let attachment_names = message
        .attachment_table()
        .map(|t| extract_attachment_names(t.as_ref(), text_cp))
        .unwrap_or_default();
    Ok(Email {
        source_key: source.into(),
        message_id,
        conversation_id,
        folder: folder.into(),
        subject,
        sender,
        recipients,
        sent_at,
        body,
        attachment_names,
    })
}

fn extract_conversation_id(
    props: &outlook_pst::messaging::message::MessageProperties,
    subject: &str,
    codepage: Option<u32>,
) -> String {
    // 1. PidTagConversationIndex (0x0071): first 22 bytes identify the root conversation thread
    if let Some(PropertyValue::Binary(bin)) = props.get(0x0071) {
        let buf = bin.buffer();
        if buf.len() >= 22 {
            let mut hex = String::with_capacity(44);
            for &b in &buf[..22] {
                use std::fmt::Write;
                let _ = write!(&mut hex, "{b:02x}");
            }
            return hex;
        }
    }
    // 2. PidTagConversationTopic (0x0070): normalized thread topic
    if let Some(topic) = text_prop(props.get(0x0070), codepage) {
        return decode_rfc2047(&topic);
    }
    // 3. Fallback to normalized subject (stripping Re:/Fwd:/TR: prefixes)
    normalize_subject_topic(subject)
}

fn normalize_subject_topic(subject: &str) -> String {
    let mut s = subject.trim();
    loop {
        let lower = s.to_ascii_lowercase();
        if let Some(rest) = lower
            .strip_prefix("re:")
            .or_else(|| lower.strip_prefix("fw:"))
            .or_else(|| lower.strip_prefix("fwd:"))
            .or_else(|| lower.strip_prefix("tr:"))
        {
            let advance = s.len() - rest.len();
            s = s[advance..].trim_start();
        } else {
            break;
        }
    }
    s.to_string()
}

fn extract_attachment_names(table: &dyn TableContext, codepage: Option<u32>) -> String {
    let context = table.context();
    let mut names = Vec::new();
    for row in table.rows_matrix() {
        let Ok(cols) = row.columns(context) else {
            continue;
        };
        let mut long_name: Option<String> = None;
        let mut short_name: Option<String> = None;
        let mut display_name: Option<String> = None;
        for (col, raw) in context.columns().iter().zip(cols) {
            let Some(raw_val) = raw else {
                continue;
            };
            let Ok(prop_val) = table.read_column(&raw_val, col.prop_type()) else {
                continue;
            };
            match col.prop_id() {
                0x3707 => long_name = text_prop(Some(&prop_val), codepage),
                0x3704 => short_name = text_prop(Some(&prop_val), codepage),
                0x3001 => display_name = text_prop(Some(&prop_val), codepage),
                _ => {}
            }
        }
        if let Some(name) = long_name.or(short_name).or(display_name)
            && !names.contains(&name)
        {
            names.push(decode_rfc2047(&name));
        }
    }
    names.join(", ")
}

fn format_name_and_email(name: Option<&str>, email: Option<&str>) -> String {
    match (name.map(str::trim), email.map(str::trim)) {
        (Some(n), Some(e))
            if !n.is_empty() && !e.is_empty() && !n.eq_ignore_ascii_case(e) && !n.contains(e) =>
        {
            format!("{n} <{e}>")
        }
        (None, Some(e)) | (Some(""), Some(e)) => e.to_string(),
        (Some(n), _) => n.to_string(),
        (None, None) => String::new(),
    }
}

fn combine_to_and_cc(to: Option<&str>, cc: Option<&str>) -> String {
    combine_recipients(to, cc, None)
}

fn combine_recipients(to: Option<&str>, cc: Option<&str>, bcc: Option<&str>) -> String {
    let mut parts = Vec::new();
    if let Some(t) = to.map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(t.to_string());
    }
    if let Some(c) = cc.map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(format!("Cc: {c}"));
    }
    if let Some(b) = bcc.map(str::trim).filter(|s| !s.is_empty()) {
        parts.push(format!("Bcc: {b}"));
    }
    parts.join("\n")
}

fn codepage_prop(value: Option<&PropertyValue>) -> Option<u32> {
    let raw = int_prop(value)?;
    (raw > 0).then_some(raw as u32)
}

fn text_prop(value: Option<&PropertyValue>, codepage: Option<u32>) -> Option<String> {
    decode_prop_value(value, codepage, false)
}

fn html_body_prop(value: Option<&PropertyValue>, codepage: Option<u32>) -> Option<String> {
    decode_prop_value(value, codepage, true)
}

fn decode_prop_value(
    value: Option<&PropertyValue>,
    codepage: Option<u32>,
    is_html: bool,
) -> Option<String> {
    match value? {
        PropertyValue::String8(v) => Some(decode_bytes(v.buffer(), codepage, is_html)),
        PropertyValue::Unicode(v) => Some(repair_text_encoding(&v.to_string())),
        PropertyValue::Binary(v) => Some(decode_bytes(v.buffer(), codepage, is_html)),
        _ => None,
    }
    .map(|s| s.trim_matches('\0').to_string())
    .filter(|s| !s.is_empty())
}

pub(crate) fn decode_bytes(bytes: &[u8], codepage: Option<u32>, is_html: bool) -> String {
    if bytes.is_empty() {
        return String::new();
    }

    // 1. Check explicit Unicode BOMs first.
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return decode_utf8_or_fallback(rest, codepage_to_encoding(codepage));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let (decoded, _) = encoding_rs::UTF_16LE.decode_without_bom_handling(rest);
        return decoded.trim_matches('\0').to_string();
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let (decoded, _) = encoding_rs::UTF_16BE.decode_without_bom_handling(rest);
        return decoded.trim_matches('\0').to_string();
    }

    // 2. Explicit UTF-16 codepage.
    if codepage == Some(1200) {
        let (decoded, _) = encoding_rs::UTF_16LE.decode_without_bom_handling(bytes);
        return decoded.trim_matches('\0').to_string();
    }
    if codepage == Some(1201) {
        let (decoded, _) = encoding_rs::UTF_16BE.decode_without_bom_handling(bytes);
        return decoded.trim_matches('\0').to_string();
    }

    // Strip trailing NUL padding on 8-bit buffers.
    let end = bytes.iter().rposition(|&b| b != 0).map_or(0, |idx| idx + 1);
    let trimmed = &bytes[..end];
    if trimmed.is_empty() {
        return String::new();
    }

    // 3. Heuristic UTF-16LE/BE when NUL bytes appear at odd/even positions without a BOM.
    if looks_like_utf16le(trimmed) {
        let (decoded, _) = encoding_rs::UTF_16LE.decode_without_bom_handling(trimmed);
        return decoded.trim_matches('\0').to_string();
    }

    let html_enc = if is_html {
        extract_html_charset(trimmed)
    } else {
        None
    };
    let cp_enc = codepage_to_encoding(codepage);

    // 4. Prefer UTF-8 if the bytes are valid UTF-8 and either UTF-8 is indicated or the
    //    declared codepage is ASCII/Latin-1/Windows-1252 (common when UTF-8 bytes are stored
    //    inside String8 or PidTagBodyHtml without updating meta/codepage).
    let allow_utf8_autodetect =
        matches!(codepage, None | Some(65001 | 1252 | 28591 | 20127 | 28590))
            || html_enc == Some(encoding_rs::UTF_8);

    if allow_utf8_autodetect && let Ok(s) = std::str::from_utf8(trimmed) {
        return repair_text_encoding(s);
    }

    // 5. Decode using HTML <meta charset>, MAPI codepage, or Windows-1252 fallback.
    let chosen = html_enc
        .filter(|&e| e != encoding_rs::UTF_8)
        .or_else(|| cp_enc.filter(|&e| e != encoding_rs::UTF_8))
        .unwrap_or(encoding_rs::WINDOWS_1252);

    let (decoded, _) = chosen.decode_without_bom_handling(trimmed);
    repair_text_encoding(&decoded)
}

fn decode_utf8_or_fallback(bytes: &[u8], fallback: Option<&'static Encoding>) -> String {
    let end = bytes.iter().rposition(|&b| b != 0).map_or(0, |idx| idx + 1);
    let trimmed = &bytes[..end];
    if let Ok(s) = std::str::from_utf8(trimmed) {
        return repair_text_encoding(s);
    }
    let enc = fallback
        .filter(|&e| e != encoding_rs::UTF_8)
        .unwrap_or(encoding_rs::WINDOWS_1252);
    let (decoded, _) = enc.decode_without_bom_handling(trimmed);
    repair_text_encoding(&decoded)
}

fn looks_like_utf16le(bytes: &[u8]) -> bool {
    if bytes.len() < 4 || !bytes.len().is_multiple_of(2) {
        return false;
    }
    let sample_len = bytes.len().min(64);
    let pairs = sample_len / 2;
    let odd_nulls = (1..sample_len)
        .step_by(2)
        .filter(|&i| bytes[i] == 0)
        .count();
    let even_non_nulls = (0..sample_len)
        .step_by(2)
        .filter(|&i| bytes[i] != 0)
        .count();
    odd_nulls * 2 >= pairs && even_non_nulls * 2 >= pairs
}

fn extract_html_charset(bytes: &[u8]) -> Option<&'static Encoding> {
    let prefix_len = bytes.len().min(4096);
    let head_latin1: String = bytes[..prefix_len].iter().map(|&b| char::from(b)).collect();
    let lower = head_latin1.to_ascii_lowercase();

    // Look for `charset=...` (inside `<meta ...>` or XML `encoding=...`).
    let mut search_from = 0;
    while let Some(rel) = lower[search_from..].find("charset") {
        let after = &lower[search_from + rel + 7..];
        let after_eq = after.trim_start();
        if let Some(rest) = after_eq.strip_prefix('=') {
            let val = rest
                .trim_start()
                .trim_start_matches(['"', '\''])
                .split(|c: char| {
                    c.is_ascii_whitespace() || matches!(c, '"' | '\'' | ';' | '>' | '/')
                })
                .next()
                .unwrap_or("");
            if !val.is_empty()
                && let Some(enc) = Encoding::for_label(val.as_bytes())
            {
                return Some(enc);
            }
        }
        search_from += rel + 7;
    }
    None
}

fn codepage_to_encoding(codepage: Option<u32>) -> Option<&'static Encoding> {
    match codepage? {
        65001 => Some(encoding_rs::UTF_8),
        1252 | 28591 | 20127 | 28590 => Some(encoding_rs::WINDOWS_1252),
        28605 => Some(encoding_rs::ISO_8859_15),
        1250 => Some(encoding_rs::WINDOWS_1250),
        1251 => Some(encoding_rs::WINDOWS_1251),
        1253 => Some(encoding_rs::WINDOWS_1253),
        1254 | 28599 => Some(encoding_rs::WINDOWS_1254),
        1255 | 28598 => Some(encoding_rs::WINDOWS_1255),
        1256 => Some(encoding_rs::WINDOWS_1256),
        1257 => Some(encoding_rs::WINDOWS_1257),
        1258 => Some(encoding_rs::WINDOWS_1258),
        874 => Some(encoding_rs::WINDOWS_874),
        28592 => Some(encoding_rs::ISO_8859_2),
        28593 => Some(encoding_rs::ISO_8859_3),
        28594 => Some(encoding_rs::ISO_8859_4),
        28595 => Some(encoding_rs::ISO_8859_5),
        28596 => Some(encoding_rs::ISO_8859_6),
        28597 => Some(encoding_rs::ISO_8859_7),
        28600 => Some(encoding_rs::ISO_8859_10),
        28603 => Some(encoding_rs::ISO_8859_13),
        28604 => Some(encoding_rs::ISO_8859_14),
        28606 => Some(encoding_rs::ISO_8859_16),
        20866 => Some(encoding_rs::KOI8_R),
        21866 => Some(encoding_rs::KOI8_U),
        932 => Some(encoding_rs::SHIFT_JIS),
        50220..=50222 => Some(encoding_rs::ISO_2022_JP),
        51932 => Some(encoding_rs::EUC_JP),
        936 => Some(encoding_rs::GBK),
        54936 => Some(encoding_rs::GB18030),
        949 => Some(encoding_rs::EUC_KR),
        950 => Some(encoding_rs::BIG5),
        10000 => Some(encoding_rs::MACINTOSH),
        _ => None,
    }
}

/// Decodes RFC 2047 MIME encoded-words such as `=?ISO-8859-1?Q?R=E9union_=E0_Paris?=`.
pub(crate) fn decode_rfc2047(input: &str) -> String {
    if !input.contains("=?") || !input.contains("?=") {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    let mut last_was_encoded = false;

    while let Some(start) = rest.find("=?") {
        let between = &rest[..start];
        if !(last_was_encoded && between.chars().all(|c| c.is_whitespace())) {
            out.push_str(between);
        }
        let after_prefix = &rest[start + 2..];
        if let Some((decoded, consumed)) = parse_rfc2047_word(after_prefix) {
            out.push_str(&decoded);
            rest = &after_prefix[consumed..];
            last_was_encoded = true;
        } else {
            out.push_str("=?");
            rest = after_prefix;
            last_was_encoded = false;
        }
    }
    out.push_str(rest);
    repair_text_encoding(&out)
}

fn parse_rfc2047_word(after_prefix: &str) -> Option<(String, usize)> {
    let q1 = after_prefix.find('?')?;
    let charset = &after_prefix[..q1];
    if charset.is_empty() || charset.contains(char::is_whitespace) {
        return None;
    }
    let after_q1 = &after_prefix[q1 + 1..];
    let q2 = after_q1.find('?')?;
    let encoding_tag = &after_q1[..q2];
    if encoding_tag.len() != 1 {
        return None;
    }
    let after_q2 = &after_q1[q2 + 1..];
    let end = after_q2.find("?=")?;
    let encoded_text = &after_q2[..end];
    if encoded_text.contains(char::is_whitespace) {
        return None;
    }

    let bytes = match encoding_tag.as_bytes()[0].to_ascii_uppercase() {
        b'Q' => decode_q_encoding(encoded_text)?,
        b'B' => decode_base64(encoded_text)?,
        _ => return None,
    };

    let enc = Encoding::for_label(charset.as_bytes()).unwrap_or(encoding_rs::UTF_8);
    let (decoded, _) = if enc == encoding_rs::UTF_8 {
        match std::str::from_utf8(&bytes) {
            Ok(s) => return Some((s.to_owned(), q1 + 1 + q2 + 1 + end + 2)),
            Err(_) => encoding_rs::WINDOWS_1252.decode_without_bom_handling(&bytes),
        }
    } else {
        enc.decode_without_bom_handling(&bytes)
    };
    Some((decoded.into_owned(), q1 + 1 + q2 + 1 + end + 2))
}

fn decode_q_encoding(input: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(input.len());
    let raw = input.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        match raw[i] {
            b'_' => {
                bytes.push(b' ');
                i += 1;
            }
            b'=' if i + 2 < raw.len() => {
                let hi = hex_val(raw[i + 1])?;
                let lo = hex_val(raw[i + 2])?;
                bytes.push((hi << 4) | lo);
                i += 3;
            }
            b'=' => return None,
            b => {
                bytes.push(b);
                i += 1;
            }
        }
    }
    Some(bytes)
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn decode_base64(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u8;
    for b in input.bytes() {
        if b == b'=' || b.is_ascii_whitespace() {
            continue;
        }
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        buf = (buf << 6) | u32::from(val);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

fn int_prop(value: Option<&PropertyValue>) -> Option<i32> {
    match value? {
        PropertyValue::Integer16(v) => Some(i32::from(*v)),
        PropertyValue::Integer32(v) => Some(*v),
        _ => None,
    }
}

fn time_prop(value: Option<&PropertyValue>) -> Option<String> {
    let PropertyValue::Time(filetime) = value? else {
        return None;
    };
    let unix_seconds = filetime
        .checked_div(10_000_000)?
        .checked_sub(11_644_473_600)?;
    chrono::DateTime::from_timestamp(unix_seconds, 0).map(|d| d.to_rfc3339())
}

fn extract_recipients(
    table: &dyn TableContext,
    fallback_to: Option<&str>,
    fallback_cc: Option<&str>,
    codepage: Option<u32>,
) -> String {
    let context = table.context();
    let mut to_list = Vec::new();
    let mut cc_list = Vec::new();
    let mut bcc_list = Vec::new();

    for row in table.rows_matrix() {
        let Ok(cols) = row.columns(context) else {
            continue;
        };
        let mut display_name: Option<String> = None;
        let mut smtp_addr: Option<String> = None;
        let mut email_addr: Option<String> = None;
        let mut recipient_type: i32 = 1; // 1 = MAPI_TO, 2 = MAPI_CC, 3 = MAPI_BCC

        for (col, raw) in context.columns().iter().zip(cols) {
            let Some(raw_val) = raw else {
                continue;
            };
            let Ok(prop_val) = table.read_column(&raw_val, col.prop_type()) else {
                continue;
            };
            match col.prop_id() {
                0x3001 => {
                    display_name = text_prop(Some(&prop_val), codepage).map(|s| decode_rfc2047(&s))
                }
                0x39FE => smtp_addr = text_prop(Some(&prop_val), codepage),
                0x3003 => {
                    email_addr = text_prop(Some(&prop_val), codepage).filter(|s| s.contains('@'))
                }
                0x0C15 => {
                    if let Some(rt) = int_prop(Some(&prop_val)) {
                        recipient_type = rt;
                    }
                }
                _ => {}
            }
        }

        let best_email = smtp_addr.or(email_addr);
        let formatted = format_name_and_email(display_name.as_deref(), best_email.as_deref());
        if formatted.is_empty() {
            continue;
        }
        match recipient_type {
            2 => cc_list.push(formatted),
            3 => bcc_list.push(formatted),
            _ => to_list.push(formatted),
        }
    }

    let to_str = if !to_list.is_empty() {
        Some(to_list.join(", "))
    } else {
        fallback_to.map(str::to_owned)
    };
    let cc_str = if !cc_list.is_empty() {
        Some(cc_list.join(", "))
    } else {
        fallback_cc.map(str::to_owned)
    };
    let bcc_str = (!bcc_list.is_empty()).then(|| bcc_list.join(", "));
    combine_recipients(to_str.as_deref(), cc_str.as_deref(), bcc_str.as_deref())
}

#[allow(dead_code)]
pub fn import(path: &Path) -> Result<Vec<Email>> {
    match path
        .extension()
        .and_then(|x| x.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jsonl") => JsonlSource.read_messages(path),
        Some("pst") => PstSource.read_messages(path),
        _ => bail!("choose a .pst archive or .jsonl development fixture"),
    }
}

pub fn import_streaming(
    path: &Path,
    mut on_batch: impl FnMut(Vec<Email>) -> Result<()>,
) -> Result<usize> {
    match path
        .extension()
        .and_then(|x| x.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jsonl") => JsonlSource.stream_messages(path, &mut on_batch),
        Some("pst") => PstSource.stream_messages(path, &mut on_batch),
        _ => bail!("choose a .pst archive or .jsonl development fixture"),
    }
}
