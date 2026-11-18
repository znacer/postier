//! Pure-Rust text extraction for common mail attachment formats.
//!
//! Supported formats:
//! - Word documents: `.docx`, `.dotx`, `.docm`, `.doc`, `.dot`
//! - PowerPoint presentations: `.pptx`, `.ppsx`, `.pptm`, `.ppt`, `.pps`
//! - LibreOffice / OpenDocument: `.odt`, `.ott`, `.odp`, `.otp`, `.ods`, `.ots`
//! - OOXML spreadsheets: `.xlsx`, `.xlsm`
//! - Text & markup files: `.txt`, `.text`, `.md`, `.markdown`, `.rst`, `.log`,
//!   `.csv`, `.tsv`, `.json`, `.xml`, `.yaml`, `.yml`, `.ini`, `.cfg`, `.conf`,
//!   `.html`, `.htm`, `.xhtml`, `.rtf`

use std::{
    collections::HashSet,
    io::{Cursor, Read},
};

use cfb::CompoundFile;
use flate2::read::DeflateDecoder;
use quick_xml::{Reader, XmlVersion, events::Event};

use crate::email_html::{html_to_plain_text, repair_text_encoding};

pub const MAX_ATTACHMENT_RAW_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_DECOMPRESSED_ENTRY_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_EXTRACTED_CHARS: usize = 100_000;
pub const MAX_EMAIL_ATTACHMENT_CHARS: usize = 200_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachmentFormat {
    PlainText,
    Xml,
    Html,
    Rtf,
    Docx,
    Pptx,
    Xlsx,
    Odt,
    Odp,
    Ods,
    LegacyDoc,
    LegacyPpt,
}

pub fn classify_attachment(filename: &str) -> Option<(AttachmentFormat, &'static str)> {
    let base = filename
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(filename)
        .trim();
    let (_, ext) = base.rsplit_once('.')?;
    match ext.to_ascii_lowercase().as_str() {
        // Word
        "docx" => Some((
            AttachmentFormat::Docx,
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        )),
        "dotx" => Some((
            AttachmentFormat::Docx,
            "application/vnd.openxmlformats-officedocument.wordprocessingml.template",
        )),
        "docm" => Some((
            AttachmentFormat::Docx,
            "application/vnd.ms-word.document.macroEnabled.12",
        )),
        "doc" | "dot" => Some((AttachmentFormat::LegacyDoc, "application/msword")),
        // PowerPoint
        "pptx" => Some((
            AttachmentFormat::Pptx,
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        )),
        "ppsx" => Some((
            AttachmentFormat::Pptx,
            "application/vnd.openxmlformats-officedocument.presentationml.slideshow",
        )),
        "pptm" => Some((
            AttachmentFormat::Pptx,
            "application/vnd.ms-powerpoint.presentation.macroEnabled.12",
        )),
        "ppt" | "pps" => Some((AttachmentFormat::LegacyPpt, "application/vnd.ms-powerpoint")),
        // LibreOffice / OpenDocument
        "odt" => Some((
            AttachmentFormat::Odt,
            "application/vnd.oasis.opendocument.text",
        )),
        "ott" => Some((
            AttachmentFormat::Odt,
            "application/vnd.oasis.opendocument.text-template",
        )),
        "odp" => Some((
            AttachmentFormat::Odp,
            "application/vnd.oasis.opendocument.presentation",
        )),
        "otp" => Some((
            AttachmentFormat::Odp,
            "application/vnd.oasis.opendocument.presentation-template",
        )),
        "ods" => Some((
            AttachmentFormat::Ods,
            "application/vnd.oasis.opendocument.spreadsheet",
        )),
        "ots" => Some((
            AttachmentFormat::Ods,
            "application/vnd.oasis.opendocument.spreadsheet-template",
        )),
        // OOXML spreadsheets
        "xlsx" => Some((
            AttachmentFormat::Xlsx,
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        )),
        "xlsm" => Some((
            AttachmentFormat::Xlsx,
            "application/vnd.ms-excel.sheet.macroEnabled.12",
        )),
        // Text / RTF / HTML
        "txt" | "text" | "log" | "ini" | "cfg" | "conf" => {
            Some((AttachmentFormat::PlainText, "text/plain"))
        }
        "md" | "markdown" => Some((AttachmentFormat::PlainText, "text/markdown")),
        "rst" => Some((AttachmentFormat::PlainText, "text/x-rst")),
        "csv" => Some((AttachmentFormat::PlainText, "text/csv")),
        "tsv" => Some((AttachmentFormat::PlainText, "text/tab-separated-values")),
        "json" => Some((AttachmentFormat::PlainText, "application/json")),
        "yaml" | "yml" => Some((AttachmentFormat::PlainText, "application/yaml")),
        "xml" => Some((AttachmentFormat::Xml, "application/xml")),
        "html" | "htm" | "xhtml" => Some((AttachmentFormat::Html, "text/html")),
        "rtf" => Some((AttachmentFormat::Rtf, "application/rtf")),
        _ => None,
    }
}

pub fn is_supported_attachment(filename: &str) -> bool {
    classify_attachment(filename).is_some()
}

pub fn attachment_mime_type(filename: &str) -> &'static str {
    classify_attachment(filename)
        .map(|(_, mime)| mime)
        .unwrap_or("application/octet-stream")
}

/// Extracts clean, normalized text from a supported attachment buffer.
/// Returns `None` for unsupported extensions, empty files, encrypted documents,
/// binary garbage, or malformed archives.
pub fn extract_attachment_text(filename: &str, bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() > MAX_ATTACHMENT_RAW_BYTES {
        return None;
    }
    let (format, _) = classify_attachment(filename)?;
    let raw = match format {
        AttachmentFormat::PlainText => decode_plain_text_strict(bytes),
        AttachmentFormat::Xml => extract_xml_text(bytes),
        AttachmentFormat::Html => extract_html_text(bytes),
        AttachmentFormat::Rtf => extract_rtf_text(bytes),
        AttachmentFormat::Docx => extract_docx_text(bytes),
        AttachmentFormat::Pptx => extract_pptx_text(bytes),
        AttachmentFormat::Xlsx => extract_xlsx_text(bytes),
        AttachmentFormat::Odt | AttachmentFormat::Odp | AttachmentFormat::Ods => {
            extract_odf_text(bytes, format)
        }
        AttachmentFormat::LegacyDoc => extract_legacy_doc_text(bytes),
        AttachmentFormat::LegacyPpt => extract_legacy_ppt_text(bytes),
    }?;
    normalize_extracted_text(&raw)
}

fn normalize_extracted_text(input: &str) -> Option<String> {
    let cleaned = input
        .replace('\0', "")
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut out = String::with_capacity(cleaned.len().min(MAX_EXTRACTED_CHARS));
    let mut blank_run = 0usize;
    let mut char_count = 0usize;

    for line in cleaned.lines() {
        let trimmed_end = line.trim_end();
        if trimmed_end.trim().is_empty() {
            if !out.is_empty() {
                blank_run += 1;
                if blank_run <= 1 && char_count < MAX_EXTRACTED_CHARS {
                    out.push('\n');
                    char_count += 1;
                }
            }
            continue;
        }
        blank_run = 0;
        if !out.is_empty() && char_count < MAX_EXTRACTED_CHARS {
            out.push('\n');
            char_count += 1;
        }
        for ch in trimmed_end.chars() {
            if char_count >= MAX_EXTRACTED_CHARS {
                break;
            }
            if ch.is_control() && ch != '\t' {
                continue;
            }
            out.push(ch);
            char_count += 1;
        }
        if char_count >= MAX_EXTRACTED_CHARS {
            break;
        }
    }

    let trimmed = out.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn decode_plain_text_strict(bytes: &[u8]) -> Option<String> {
    let decoded = decode_text_bytes(bytes)?;
    let mut total = 0usize;
    let mut bad_ctrl = 0usize;
    for ch in decoded.chars().take(16_384) {
        total += 1;
        if (ch.is_control() && !matches!(ch, '\n' | '\r' | '\t')) || ch == '\u{FFFD}' {
            bad_ctrl += 1;
        }
    }
    if total == 0 || bad_ctrl * 10 > total {
        return None;
    }
    Some(decoded)
}

fn decode_text_bytes(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return std::str::from_utf8(rest)
            .ok()
            .map(repair_text_encoding)
            .or_else(|| {
                let (decoded, _) = encoding_rs::WINDOWS_1252.decode_without_bom_handling(rest);
                Some(repair_text_encoding(&decoded))
            });
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let (decoded, _) = encoding_rs::UTF_16LE.decode_without_bom_handling(rest);
        return Some(decoded.into_owned());
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let (decoded, _) = encoding_rs::UTF_16BE.decode_without_bom_handling(rest);
        return Some(decoded.into_owned());
    }
    if looks_like_utf16le(bytes) {
        let (decoded, _) = encoding_rs::UTF_16LE.decode_without_bom_handling(bytes);
        return Some(decoded.into_owned());
    }
    if looks_like_utf16be(bytes) {
        let (decoded, _) = encoding_rs::UTF_16BE.decode_without_bom_handling(bytes);
        return Some(decoded.into_owned());
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Some(repair_text_encoding(s));
    }
    // Reject obvious binary buffers before falling back to Windows-1252.
    let nul_count = bytes.iter().take(4096).filter(|&&b| b == 0).count();
    if nul_count > 4 {
        return None;
    }
    let (decoded, _) = encoding_rs::WINDOWS_1252.decode_without_bom_handling(bytes);
    Some(repair_text_encoding(&decoded))
}

fn looks_like_utf16le(bytes: &[u8]) -> bool {
    if bytes.len() < 4 || !bytes.len().is_multiple_of(2) {
        return false;
    }
    let sample = bytes.len().min(64);
    let pairs = sample / 2;
    let odd_nulls = (1..sample).step_by(2).filter(|&i| bytes[i] == 0).count();
    let even_non_nulls = (0..sample).step_by(2).filter(|&i| bytes[i] != 0).count();
    odd_nulls * 2 >= pairs && even_non_nulls * 2 >= pairs
}

fn looks_like_utf16be(bytes: &[u8]) -> bool {
    if bytes.len() < 4 || !bytes.len().is_multiple_of(2) {
        return false;
    }
    let sample = bytes.len().min(64);
    let pairs = sample / 2;
    let even_nulls = (0..sample).step_by(2).filter(|&i| bytes[i] == 0).count();
    let odd_non_nulls = (1..sample).step_by(2).filter(|&i| bytes[i] != 0).count();
    even_nulls * 2 >= pairs && odd_non_nulls * 2 >= pairs
}

fn extract_html_text(bytes: &[u8]) -> Option<String> {
    let decoded = decode_text_bytes(bytes)?;
    let plain = html_to_plain_text(&decoded);
    if plain.trim().is_empty() {
        None
    } else {
        Some(plain)
    }
}

fn extract_xml_text(bytes: &[u8]) -> Option<String> {
    let decoded = decode_text_bytes(bytes)?;
    let mut reader = Reader::from_str(&decoded);
    reader.config_mut().trim_text(false);
    let mut out = String::new();
    let mut skip_depth = 0usize;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                if skip_depth > 0 || matches!(local_name(e.name().as_ref()), b"style" | b"script") {
                    skip_depth += 1;
                }
            }
            Ok(Event::End(e)) => {
                if skip_depth > 0 {
                    skip_depth -= 1;
                } else if matches!(
                    local_name(e.name().as_ref()),
                    b"p" | b"row" | b"item" | b"record" | b"line" | b"entry" | b"section"
                ) {
                    push_newline(&mut out);
                } else if !out.ends_with([' ', '\n', '\t']) && !out.is_empty() {
                    out.push(' ');
                }
            }
            Ok(Event::Text(e)) if skip_depth == 0 => {
                if let Ok(t) = e.decode() {
                    let trimmed = t.trim();
                    if !trimmed.is_empty() {
                        if !out.is_empty() && !out.ends_with([' ', '\n', '\t']) {
                            out.push(' ');
                        }
                        out.push_str(trimmed);
                    }
                }
            }
            Ok(Event::CData(e)) if skip_depth == 0 => {
                if let Ok(t) = e.decode() {
                    let trimmed = t.trim();
                    if !trimmed.is_empty() {
                        if !out.is_empty() && !out.ends_with([' ', '\n', '\t']) {
                            out.push(' ');
                        }
                        out.push_str(trimmed);
                    }
                }
            }
            Ok(Event::GeneralRef(e)) if skip_depth == 0 => {
                push_general_ref(&mut out, &e);
            }
            Ok(Event::Eof) => break,
            Err(_) => return decode_plain_text_strict(bytes),
            _ => {}
        }
    }
    if out.trim().is_empty() {
        decode_plain_text_strict(bytes)
    } else {
        Some(out)
    }
}

// ---------------------------------------------------------------------------
// RTF extractor

fn extract_rtf_text(bytes: &[u8]) -> Option<String> {
    let trimmed_start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    if !bytes[trimmed_start..].starts_with(b"{\\rtf") {
        return decode_plain_text_strict(bytes);
    }

    #[derive(Clone, Copy)]
    struct RtfGroup {
        uc: usize,
        skip: bool,
    }

    let mut stack = vec![RtfGroup { uc: 1, skip: false }];
    let mut out = String::new();
    let mut cp1252_buf: Vec<u8> = Vec::new();
    let mut skip_chars = 0usize;
    let mut ignorable_next = false;
    let mut i = trimmed_start;

    let flush_cp1252 = |buf: &mut Vec<u8>, out: &mut String| {
        if !buf.is_empty() {
            let (decoded, _) = encoding_rs::WINDOWS_1252.decode_without_bom_handling(buf);
            out.push_str(&decoded);
            buf.clear();
        }
    };

    while i < bytes.len() {
        let cur = *stack.last().unwrap_or(&RtfGroup { uc: 1, skip: false });
        match bytes[i] {
            b'{' => {
                flush_cp1252(&mut cp1252_buf, &mut out);
                stack.push(cur);
                ignorable_next = false;
                i += 1;
            }
            b'}' => {
                flush_cp1252(&mut cp1252_buf, &mut out);
                if stack.len() > 1 {
                    stack.pop();
                }
                ignorable_next = false;
                skip_chars = 0;
                i += 1;
            }
            b'\\' => {
                i += 1;
                if i >= bytes.len() {
                    break;
                }
                match bytes[i] {
                    b'*' => {
                        ignorable_next = true;
                        i += 1;
                    }
                    b'\'' => {
                        if i + 2 < bytes.len()
                            && let (Some(h), Some(l)) =
                                (hex_nibble(bytes[i + 1]), hex_nibble(bytes[i + 2]))
                        {
                            let byte = (h << 4) | l;
                            if !cur.skip {
                                if skip_chars > 0 {
                                    skip_chars -= 1;
                                } else {
                                    cp1252_buf.push(byte);
                                }
                            }
                            i += 3;
                        } else {
                            i += 1;
                        }
                    }
                    b'\\' | b'{' | b'}' => {
                        flush_cp1252(&mut cp1252_buf, &mut out);
                        if !cur.skip {
                            if skip_chars > 0 {
                                skip_chars -= 1;
                            } else {
                                out.push(char::from(bytes[i]));
                            }
                        }
                        i += 1;
                    }
                    b'~' => {
                        flush_cp1252(&mut cp1252_buf, &mut out);
                        if !cur.skip && skip_chars == 0 {
                            out.push(' ');
                        }
                        i += 1;
                    }
                    b'_' => {
                        flush_cp1252(&mut cp1252_buf, &mut out);
                        if !cur.skip && skip_chars == 0 {
                            out.push('-');
                        }
                        i += 1;
                    }
                    b'\r' | b'\n' => {
                        flush_cp1252(&mut cp1252_buf, &mut out);
                        if !cur.skip {
                            out.push('\n');
                        }
                        i += 1;
                    }
                    b if b.is_ascii_alphabetic() => {
                        flush_cp1252(&mut cp1252_buf, &mut out);
                        let word_start = i;
                        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                            i += 1;
                        }
                        let word = std::str::from_utf8(&bytes[word_start..i]).unwrap_or("");
                        let param_start = i;
                        if i < bytes.len() && (bytes[i] == b'-' || bytes[i].is_ascii_digit()) {
                            i += 1;
                            while i < bytes.len() && bytes[i].is_ascii_digit() {
                                i += 1;
                            }
                        }
                        let param =
                            std::str::from_utf8(&bytes[param_start..i])
                                .ok()
                                .and_then(|s| {
                                    if s.is_empty() {
                                        None
                                    } else {
                                        s.parse::<i32>().ok()
                                    }
                                });
                        if i < bytes.len() && bytes[i] == b' ' {
                            i += 1;
                        }

                        if ignorable_next {
                            if let Some(top) = stack.last_mut() {
                                top.skip = true;
                            }
                            ignorable_next = false;
                            continue;
                        }
                        if is_rtf_skip_destination(word) {
                            if let Some(top) = stack.last_mut() {
                                top.skip = true;
                            }
                            continue;
                        }
                        if cur.skip {
                            continue;
                        }
                        match word {
                            "uc" => {
                                if let Some(n) = param
                                    && let Some(top) = stack.last_mut()
                                {
                                    top.uc = n.max(0) as usize;
                                }
                            }
                            "u" => {
                                if let Some(val) = param {
                                    let code = if val < 0 {
                                        (val + 65536) as u32
                                    } else {
                                        val as u32
                                    };
                                    if let Some(ch) = char::from_u32(code) {
                                        out.push(ch);
                                    }
                                    skip_chars = cur.uc;
                                }
                            }
                            "par" | "line" => out.push('\n'),
                            "tab" => out.push('\t'),
                            "emdash" => out.push('—'),
                            "endash" => out.push('–'),
                            "lquote" | "rquote" => out.push('\''),
                            "ldblquote" | "rdblquote" => out.push('"'),
                            "bullet" => out.push('•'),
                            _ => {}
                        }
                    }
                    _ => {
                        ignorable_next = false;
                        i += 1;
                    }
                }
            }
            b'\r' | b'\n' => {
                i += 1;
            }
            b => {
                ignorable_next = false;
                if !cur.skip {
                    if skip_chars > 0 {
                        skip_chars -= 1;
                    } else if b.is_ascii() {
                        flush_cp1252(&mut cp1252_buf, &mut out);
                        out.push(char::from(b));
                    } else {
                        cp1252_buf.push(b);
                    }
                }
                i += 1;
            }
        }
    }
    flush_cp1252(&mut cp1252_buf, &mut out);
    (!out.trim().is_empty()).then_some(out)
}

fn is_rtf_skip_destination(word: &str) -> bool {
    matches!(
        word,
        "fonttbl"
            | "colortbl"
            | "stylesheet"
            | "info"
            | "pict"
            | "object"
            | "header"
            | "footer"
            | "headerl"
            | "headerr"
            | "headerf"
            | "footerl"
            | "footerr"
            | "footerf"
            | "fldinst"
            | "xe"
            | "tc"
            | "rxe"
            | "bkmkstart"
            | "bkmkend"
            | "listtable"
            | "listoverridetable"
            | "rsidtbl"
            | "mmathPr"
            | "themedata"
            | "colorschememapping"
            | "latentstyles"
            | "datastore"
            | "xmlnstbl"
    )
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Minimal pure-Rust ZIP reader (Stored + Deflate via flate2)

struct ZipEntryMeta {
    name: String,
    flags: u16,
    compression_method: u16,
    compressed_size: usize,
    uncompressed_size: usize,
    local_header_offset: usize,
}

struct ZipArchive<'a> {
    data: &'a [u8],
    entries: Vec<ZipEntryMeta>,
}

impl<'a> ZipArchive<'a> {
    fn parse(data: &'a [u8]) -> Option<Self> {
        if data.len() < 22 {
            return None;
        }
        let search_start = data.len().saturating_sub(65_535 + 22);
        let mut eocd_pos = None;
        let mut i = data.len().saturating_sub(22);
        loop {
            if &data[i..i + 4] == b"PK\x05\x06" {
                let comment_len = read_u16_le(data, i + 20)? as usize;
                if i + 22 + comment_len == data.len() {
                    eocd_pos = Some(i);
                    break;
                }
            }
            if i <= search_start {
                break;
            }
            i -= 1;
        }
        let eocd = eocd_pos?;
        let total_entries = read_u16_le(data, eocd + 10)? as usize;
        let cd_size = read_u32_le(data, eocd + 12)? as usize;
        let cd_offset = read_u32_le(data, eocd + 16)? as usize;
        if cd_offset.checked_add(cd_size)? > eocd {
            return None;
        }

        let mut entries = Vec::with_capacity(total_entries.min(4096));
        let mut pos = cd_offset;
        let cd_end = cd_offset + cd_size;
        while pos + 46 <= cd_end && entries.len() < 8192 {
            if &data[pos..pos + 4] != b"PK\x01\x02" {
                break;
            }
            let flags = read_u16_le(data, pos + 8)?;
            let method = read_u16_le(data, pos + 10)?;
            let compressed_size = read_u32_le(data, pos + 20)? as usize;
            let uncompressed_size = read_u32_le(data, pos + 24)? as usize;
            let name_len = read_u16_le(data, pos + 28)? as usize;
            let extra_len = read_u16_le(data, pos + 30)? as usize;
            let comment_len = read_u16_le(data, pos + 32)? as usize;
            let local_header_offset = read_u32_le(data, pos + 42)? as usize;

            let name_start = pos + 46;
            let next_pos = name_start + name_len + extra_len + comment_len;
            if next_pos > cd_end {
                break;
            }
            let name = String::from_utf8_lossy(&data[name_start..name_start + name_len])
                .replace('\\', "/");
            entries.push(ZipEntryMeta {
                name,
                flags,
                compression_method: method,
                compressed_size,
                uncompressed_size,
                local_header_offset,
            });
            pos = next_pos;
        }
        Some(Self { data, entries })
    }

    fn read_entry(&self, name: &str) -> Option<Vec<u8>> {
        let meta = self
            .entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(name))?;
        self.read_meta(meta)
    }

    fn read_meta(&self, meta: &ZipEntryMeta) -> Option<Vec<u8>> {
        if (meta.flags & 0x0001) != 0 {
            return None;
        }
        if meta.uncompressed_size > MAX_DECOMPRESSED_ENTRY_BYTES {
            return None;
        }
        let off = meta.local_header_offset;
        if off + 30 > self.data.len() || &self.data[off..off + 4] != b"PK\x03\x04" {
            return None;
        }
        let local_flags = read_u16_le(self.data, off + 6)?;
        if (local_flags & 0x0001) != 0 {
            return None;
        }
        let fname_len = read_u16_le(self.data, off + 26)? as usize;
        let extra_len = read_u16_le(self.data, off + 28)? as usize;
        let data_start = off.checked_add(30 + fname_len + extra_len)?;
        let data_end = data_start.checked_add(meta.compressed_size)?;
        if data_end > self.data.len() {
            return None;
        }
        let payload = &self.data[data_start..data_end];
        match meta.compression_method {
            0 => {
                if payload.len() > MAX_DECOMPRESSED_ENTRY_BYTES {
                    return None;
                }
                Some(payload.to_vec())
            }
            8 => {
                let mut decoder =
                    DeflateDecoder::new(payload).take((MAX_DECOMPRESSED_ENTRY_BYTES + 1) as u64);
                let mut out = Vec::with_capacity(meta.uncompressed_size.min(64 * 1024));
                decoder.read_to_end(&mut out).ok()?;
                if out.len() > MAX_DECOMPRESSED_ENTRY_BYTES {
                    return None;
                }
                Some(out)
            }
            _ => None,
        }
    }

    fn entry_names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.name.as_str())
    }
}

fn read_u16_le(data: &[u8], off: usize) -> Option<u16> {
    let slice = data.get(off..off + 2)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32_le(data: &[u8], off: usize) -> Option<u32> {
    let slice = data.get(off..off + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

// ---------------------------------------------------------------------------
// XML helpers

fn local_name(raw: &[u8]) -> &[u8] {
    match raw.iter().rposition(|&b| b == b':') {
        Some(pos) => &raw[pos + 1..],
        None => raw,
    }
}

fn push_general_ref(out: &mut String, e: &quick_xml::events::BytesRef<'_>) {
    if let Ok(Some(ch)) = e.resolve_char_ref() {
        out.push(ch);
        return;
    }
    if let Ok(name) = e.decode() {
        match name.as_ref() {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            "nbsp" => out.push(' '),
            _ => {}
        }
    }
}

fn push_newline(out: &mut String) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
}

fn numeric_suffix(path: &str, prefix: &str, suffix: &str) -> Option<usize> {
    let lower = path.to_ascii_lowercase();
    let mid = lower.strip_prefix(prefix)?.strip_suffix(suffix)?;
    if mid.is_empty() || !mid.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    mid.parse().ok()
}

// ---------------------------------------------------------------------------
// OOXML: DOCX / PPTX / XLSX

fn extract_docx_text(bytes: &[u8]) -> Option<String> {
    let zip = ZipArchive::parse(bytes)?;
    let mut parts: Vec<String> = Vec::new();
    if let Some(doc_xml) = zip.read_entry("word/document.xml")
        && let Some(text) = parse_wordprocessing_xml(&doc_xml)
    {
        parts.push(text);
    }

    let mut headers_footers: Vec<String> = zip
        .entry_names()
        .filter(|name| {
            let lower = name.to_ascii_lowercase();
            (lower.starts_with("word/header") || lower.starts_with("word/footer"))
                && lower.ends_with(".xml")
        })
        .map(str::to_string)
        .collect();
    headers_footers.sort();

    for extra_name in headers_footers.into_iter().chain(
        [
            "word/footnotes.xml".to_string(),
            "word/endnotes.xml".to_string(),
            "word/comments.xml".to_string(),
        ]
        .into_iter(),
    ) {
        if let Some(xml) = zip.read_entry(&extra_name)
            && let Some(text) = parse_wordprocessing_xml(&xml)
            && !text.trim().is_empty()
        {
            parts.push(text);
        }
    }

    let joined = parts.join("\n\n");
    (!joined.trim().is_empty()).then_some(joined)
}

fn parse_wordprocessing_xml(xml_bytes: &[u8]) -> Option<String> {
    let xml = decode_text_bytes(xml_bytes)?;
    let mut reader = Reader::from_str(&xml);
    reader.config_mut().trim_text(false);
    let mut out = String::new();
    let mut skip_depth = 0usize;
    let mut first_cell_in_row = true;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let qname = e.name();
                let name = local_name(qname.as_ref());
                if skip_depth > 0 || matches!(name, b"del" | b"delText" | b"instrText" | b"binData")
                {
                    skip_depth += 1;
                } else if name == b"tr" {
                    first_cell_in_row = true;
                } else if name == b"tc" {
                    if !first_cell_in_row {
                        while out.ends_with('\n') {
                            out.pop();
                        }
                        if !out.is_empty() && !out.ends_with('\t') {
                            out.push('\t');
                        }
                    }
                    first_cell_in_row = false;
                }
            }
            Ok(Event::Empty(e)) if skip_depth == 0 => match local_name(e.name().as_ref()) {
                b"tab" => out.push('\t'),
                b"br" | b"cr" => out.push('\n'),
                _ => {}
            },
            Ok(Event::End(e)) => {
                if skip_depth > 0 {
                    skip_depth -= 1;
                } else {
                    match local_name(e.name().as_ref()) {
                        b"p" | b"tr" => push_newline(&mut out),
                        _ => {}
                    }
                }
            }
            Ok(Event::Text(e)) if skip_depth == 0 => {
                if let Ok(t) = e.decode() {
                    out.push_str(&t);
                }
            }
            Ok(Event::CData(e)) if skip_depth == 0 => {
                if let Ok(t) = e.decode() {
                    out.push_str(&t);
                }
            }
            Ok(Event::GeneralRef(e)) if skip_depth == 0 => {
                push_general_ref(&mut out, &e);
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    (!out.trim().is_empty()).then_some(out)
}

fn extract_pptx_text(bytes: &[u8]) -> Option<String> {
    let zip = ZipArchive::parse(bytes)?;
    let mut slides: Vec<(usize, String)> = zip
        .entry_names()
        .filter_map(|name| {
            numeric_suffix(name, "ppt/slides/slide", ".xml").map(|n| (n, name.to_string()))
        })
        .collect();
    slides.sort_by_key(|(n, _)| *n);

    let mut notes: Vec<(usize, String)> = zip
        .entry_names()
        .filter_map(|name| {
            numeric_suffix(name, "ppt/notesSlides/notesSlide", ".xml")
                .or_else(|| numeric_suffix(name, "ppt/notesslides/notesslide", ".xml"))
                .map(|n| (n, name.to_string()))
        })
        .collect();
    notes.sort_by_key(|(n, _)| *n);

    let mut sections: Vec<String> = Vec::new();
    for (num, path) in slides {
        if let Some(xml) = zip.read_entry(&path)
            && let Some(text) = parse_drawing_xml(&xml)
        {
            sections.push(format!("[Slide {num}]\n{text}"));
        }
    }
    for (num, path) in notes {
        if let Some(xml) = zip.read_entry(&path)
            && let Some(text) = parse_drawing_xml(&xml)
        {
            sections.push(format!("[Notes {num}]\n{text}"));
        }
    }

    let joined = sections.join("\n\n");
    (!joined.trim().is_empty()).then_some(joined)
}

fn parse_drawing_xml(xml_bytes: &[u8]) -> Option<String> {
    let xml = decode_text_bytes(xml_bytes)?;
    let mut reader = Reader::from_str(&xml);
    reader.config_mut().trim_text(false);
    let mut out = String::new();
    let mut in_text = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                if local_name(e.name().as_ref()) == b"t" {
                    in_text = true;
                }
            }
            Ok(Event::Empty(e)) => match local_name(e.name().as_ref()) {
                b"br" => out.push('\n'),
                b"tab" => out.push('\t'),
                _ => {}
            },
            Ok(Event::End(e)) => match local_name(e.name().as_ref()) {
                b"t" => in_text = false,
                b"p" | b"tr" => push_newline(&mut out),
                _ => {}
            },
            Ok(Event::Text(e)) if in_text => {
                if let Ok(t) = e.decode() {
                    out.push_str(&t);
                }
            }
            Ok(Event::CData(e)) if in_text => {
                if let Ok(t) = e.decode() {
                    out.push_str(&t);
                }
            }
            Ok(Event::GeneralRef(e)) if in_text => {
                push_general_ref(&mut out, &e);
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    let trimmed = out.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn extract_xlsx_text(bytes: &[u8]) -> Option<String> {
    let zip = ZipArchive::parse(bytes)?;
    let shared_strings = zip
        .read_entry("xl/sharedStrings.xml")
        .and_then(|b| parse_xlsx_shared_strings(&b))
        .unwrap_or_default();
    let sheet_names = zip
        .read_entry("xl/workbook.xml")
        .map(|b| parse_xlsx_sheet_names(&b))
        .unwrap_or_default();

    let mut sheets: Vec<(usize, String)> = zip
        .entry_names()
        .filter_map(|name| {
            numeric_suffix(name, "xl/worksheets/sheet", ".xml").map(|n| (n, name.to_string()))
        })
        .collect();
    sheets.sort_by_key(|(n, _)| *n);

    let mut sections = Vec::new();
    for (idx, path) in sheets {
        if let Some(xml) = zip.read_entry(&path)
            && let Some(body) = parse_xlsx_sheet(&xml, &shared_strings)
        {
            let title = sheet_names
                .get(idx.saturating_sub(1))
                .cloned()
                .unwrap_or_else(|| format!("Sheet {idx}"));
            sections.push(format!("[Sheet: {title}]\n{body}"));
        }
    }
    let joined = sections.join("\n\n");
    (!joined.trim().is_empty()).then_some(joined)
}

fn parse_xlsx_shared_strings(xml_bytes: &[u8]) -> Option<Vec<String>> {
    let xml = decode_text_bytes(xml_bytes)?;
    let mut reader = Reader::from_str(&xml);
    reader.config_mut().trim_text(false);
    let mut strings = Vec::new();
    let mut current = String::new();
    let mut in_si = false;
    let mut in_t = false;
    let mut skip_phonetic = 0usize;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match local_name(e.name().as_ref()) {
                b"si" => {
                    in_si = true;
                    current.clear();
                }
                b"rPh" if in_si => skip_phonetic += 1,
                b"t" if in_si && skip_phonetic == 0 => in_t = true,
                _ => {}
            },
            Ok(Event::End(e)) => match local_name(e.name().as_ref()) {
                b"si" => {
                    strings.push(std::mem::take(&mut current));
                    in_si = false;
                }
                b"rPh" => skip_phonetic = skip_phonetic.saturating_sub(1),
                b"t" => in_t = false,
                _ => {}
            },
            Ok(Event::Text(e)) if in_si && in_t && skip_phonetic == 0 => {
                if let Ok(t) = e.decode() {
                    current.push_str(&t);
                }
            }
            Ok(Event::GeneralRef(e)) if in_si && in_t && skip_phonetic == 0 => {
                push_general_ref(&mut current, &e);
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    Some(strings)
}

fn parse_xlsx_sheet_names(xml_bytes: &[u8]) -> Vec<String> {
    let Some(xml) = decode_text_bytes(xml_bytes) else {
        return Vec::new();
    };
    let mut reader = Reader::from_str(&xml);
    let mut names = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e) | Event::Empty(e)) if local_name(e.name().as_ref()) == b"sheet" => {
                for attr in e.attributes().flatten() {
                    if local_name(attr.key.as_ref()) == b"name"
                        && let Ok(val) = attr.decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder())
                    {
                        names.push(val.into_owned());
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    names
}

fn parse_xlsx_sheet(xml_bytes: &[u8], shared_strings: &[String]) -> Option<String> {
    let xml = decode_text_bytes(xml_bytes)?;
    let mut reader = Reader::from_str(&xml);
    reader.config_mut().trim_text(false);
    let mut out = String::new();
    let mut row_cells: Vec<String> = Vec::new();
    let mut cell_type = String::new();
    let mut cell_text = String::new();
    let mut in_cell = false;
    let mut in_val = false;
    let mut in_inline_t = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match local_name(e.name().as_ref()) {
                b"row" => row_cells.clear(),
                b"c" => {
                    in_cell = true;
                    cell_type.clear();
                    cell_text.clear();
                    for attr in e.attributes().flatten() {
                        if local_name(attr.key.as_ref()) == b"t"
                            && let Ok(v) = attr.decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder())
                        {
                            cell_type = v.into_owned();
                        }
                    }
                }
                b"v" if in_cell => in_val = true,
                b"t" if in_cell => in_inline_t = true,
                _ => {}
            },
            Ok(Event::End(e)) => match local_name(e.name().as_ref()) {
                b"v" => in_val = false,
                b"t" => in_inline_t = false,
                b"c" => {
                    let resolved = if cell_type == "s" {
                        cell_text
                            .trim()
                            .parse::<usize>()
                            .ok()
                            .and_then(|idx| shared_strings.get(idx).cloned())
                            .unwrap_or_default()
                    } else {
                        std::mem::take(&mut cell_text)
                    };
                    if !resolved.trim().is_empty() {
                        row_cells.push(resolved.trim().to_string());
                    }
                    in_cell = false;
                }
                b"row" => {
                    if !row_cells.is_empty() {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(&row_cells.join("\t"));
                    }
                }
                _ => {}
            },
            Ok(Event::Text(e)) if in_cell && (in_val || in_inline_t) => {
                if let Ok(t) = e.decode() {
                    cell_text.push_str(&t);
                }
            }
            Ok(Event::GeneralRef(e)) if in_cell && (in_val || in_inline_t) => {
                push_general_ref(&mut cell_text, &e);
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    (!out.trim().is_empty()).then_some(out)
}

// ---------------------------------------------------------------------------
// LibreOffice / OpenDocument: ODT / ODP / ODS

fn extract_odf_text(bytes: &[u8], format: AttachmentFormat) -> Option<String> {
    let zip = ZipArchive::parse(bytes)?;
    let xml_bytes = zip.read_entry("content.xml")?;
    let xml = decode_text_bytes(&xml_bytes)?;
    let mut reader = Reader::from_str(&xml);
    reader.config_mut().trim_text(false);

    let mut out = String::new();
    let mut skip_depth = 0usize;
    let mut slide_num = 0usize;
    let mut first_cell_in_row = true;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let qname = e.name();
                let name = local_name(qname.as_ref());
                if skip_depth > 0 || matches!(name, b"binary-data" | b"tracked-changes") {
                    skip_depth += 1;
                    continue;
                }
                match name {
                    b"page" if format == AttachmentFormat::Odp => {
                        slide_num += 1;
                        let page_name = attr_value(&e, b"name", &reader);
                        push_newline(&mut out);
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        match page_name {
                            Some(n) if !n.trim().is_empty() => {
                                out.push_str(&format!("[Slide {slide_num}: {}]\n", n.trim()));
                            }
                            _ => out.push_str(&format!("[Slide {slide_num}]\n")),
                        }
                    }
                    b"table" if format == AttachmentFormat::Ods => {
                        if let Some(table_name) = attr_value(&e, b"name", &reader)
                            && !table_name.trim().is_empty()
                        {
                            push_newline(&mut out);
                            if !out.is_empty() {
                                out.push('\n');
                            }
                            out.push_str(&format!("[Sheet: {}]\n", table_name.trim()));
                        }
                    }
                    b"table-row" => first_cell_in_row = true,
                    b"table-cell" => {
                        if !first_cell_in_row {
                            while out.ends_with('\n') {
                                out.pop();
                            }
                            if !out.is_empty() && !out.ends_with('\t') {
                                out.push('\t');
                            }
                        }
                        first_cell_in_row = false;
                    }
                    b"s" => {
                        let count = attr_value(&e, b"c", &reader)
                            .and_then(|v| v.parse::<usize>().ok())
                            .unwrap_or(1)
                            .min(16);
                        for _ in 0..count {
                            out.push(' ');
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) if skip_depth == 0 => match local_name(e.name().as_ref()) {
                b"line-break" => out.push('\n'),
                b"tab" => out.push('\t'),
                b"s" => {
                    let count = attr_value(&e, b"c", &reader)
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(1)
                        .min(16);
                    for _ in 0..count {
                        out.push(' ');
                    }
                }
                _ => {}
            },
            Ok(Event::End(e)) => {
                if skip_depth > 0 {
                    skip_depth -= 1;
                } else {
                    match local_name(e.name().as_ref()) {
                        b"p" | b"h" | b"table-row" => push_newline(&mut out),
                        _ => {}
                    }
                }
            }
            Ok(Event::Text(e)) if skip_depth == 0 => {
                if let Ok(t) = e.decode() {
                    out.push_str(&t);
                }
            }
            Ok(Event::CData(e)) if skip_depth == 0 => {
                if let Ok(t) = e.decode() {
                    out.push_str(&t);
                }
            }
            Ok(Event::GeneralRef(e)) if skip_depth == 0 => {
                push_general_ref(&mut out, &e);
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    (!out.trim().is_empty()).then_some(out)
}

fn attr_value(
    e: &quick_xml::events::BytesStart<'_>,
    key_local: &[u8],
    reader: &Reader<&[u8]>,
) -> Option<String> {
    for attr in e.attributes().flatten() {
        if local_name(attr.key.as_ref()) == key_local
            && let Ok(val) = attr.decoded_and_normalized_value(XmlVersion::Implicit1_0, reader.decoder())
        {
            return Some(val.into_owned());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Legacy OLE2 Word (.doc / .dot) via cfb + [MS-DOC] Piece Table

fn read_cfb_stream<R: std::io::Read + std::io::Seek>(
    comp: &mut CompoundFile<R>,
    names: &[&str],
) -> Option<Vec<u8>> {
    let path = names.iter().copied().find(|n| comp.exists(n))?;
    let mut stream = comp.open_stream(path).ok()?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).ok()?;
    Some(buf)
}

fn extract_legacy_doc_text(bytes: &[u8]) -> Option<String> {
    // Outlook/Word sometimes saves RTF, HTML, or DOCX content with a `.doc` filename.
    let trimmed_start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let head = &bytes[trimmed_start..];
    if head.starts_with(b"{\\rtf") {
        return extract_rtf_text(bytes);
    }
    if head.starts_with(b"PK\x03\x04") {
        return extract_docx_text(bytes);
    }
    if head.get(..16).is_some_and(|p| {
        p.to_ascii_lowercase().starts_with(b"<!doctype html")
            || p.to_ascii_lowercase().starts_with(b"<html")
    }) {
        return extract_html_text(bytes);
    }

    let mut comp = CompoundFile::open(Cursor::new(bytes)).ok()?;
    let word_doc = read_cfb_stream(&mut comp, &["/WordDocument", "WordDocument"])?;
    if word_doc.len() < 0x1B0 {
        return None;
    }

    let w_ident = read_u16_le(&word_doc, 0)?;
    if w_ident != 0xA5EC {
        return None;
    }
    let flags = read_u16_le(&word_doc, 0x0A)?;
    // Bit 0x0100: fEncrypted
    if (flags & 0x0100) != 0 {
        return None;
    }

    let table_names: [&str; 2] = if (flags & 0x0200) != 0 {
        ["/1Table", "1Table"]
    } else {
        ["/0Table", "0Table"]
    };

    let ccp_text = read_u32_le(&word_doc, 0x4C).unwrap_or(0);
    let fc_clx = read_u32_le(&word_doc, 0x01A2).unwrap_or(0) as usize;
    let lcb_clx = read_u32_le(&word_doc, 0x01A6).unwrap_or(0) as usize;

    if lcb_clx > 0
        && let Some(table_buf) = read_cfb_stream(&mut comp, &table_names)
        && let Some(clx) = table_buf.get(fc_clx..fc_clx.saturating_add(lcb_clx))
        && let Some(raw) = parse_doc_piece_table(&word_doc, clx, ccp_text)
    {
        return clean_legacy_doc_text(&raw);
    }

    // Fallback for non-complex legacy Word documents (fcMin..fcMac).
    let fc_min = read_u32_le(&word_doc, 0x18).unwrap_or(0) as usize;
    let fc_mac = read_u32_le(&word_doc, 0x1C).unwrap_or(0) as usize;
    if fc_mac > fc_min && fc_mac <= word_doc.len() {
        let (decoded, _) =
            encoding_rs::WINDOWS_1252.decode_without_bom_handling(&word_doc[fc_min..fc_mac]);
        return clean_legacy_doc_text(&decoded);
    }
    None
}

fn parse_doc_piece_table(word_doc: &[u8], clx: &[u8], ccp_text: u32) -> Option<String> {
    let mut pos = 0usize;
    while pos < clx.len() {
        match clx[pos] {
            0x01 => {
                // Prc: 1-byte type + 2-byte cbGrpprl + cbGrpprl bytes
                let cb = read_u16_le(clx, pos + 1)? as usize;
                pos = pos.checked_add(3 + cb)?;
            }
            0x02 => {
                let lcb = read_u32_le(clx, pos + 1)? as usize;
                let plc = clx.get(pos + 5..pos + 5 + lcb)?;
                if plc.len() < 16 || !(plc.len() - 4).is_multiple_of(12) {
                    return None;
                }
                let n = (plc.len() - 4) / 12;
                let cps: Vec<u32> = (0..=n).filter_map(|i| read_u32_le(plc, i * 4)).collect();
                if cps.len() != n + 1 {
                    return None;
                }
                let pcd_base = (n + 1) * 4;
                let mut out = String::new();
                for i in 0..n {
                    let cp_start = cps[i];
                    let cp_end = cps[i + 1];
                    if cp_end <= cp_start {
                        continue;
                    }
                    // Keep main body (0..ccp_text) if ccp_text > 0; avoid garbled internal streams
                    if ccp_text > 0 && cp_start >= ccp_text {
                        break;
                    }
                    let cp_capped_end = if ccp_text > 0 {
                        cp_end.min(ccp_text)
                    } else {
                        cp_end
                    };
                    let char_len = (cp_capped_end - cp_start) as usize;
                    let fc_raw = read_u32_le(plc, pcd_base + i * 8 + 2)?;
                    let is_compressed = (fc_raw & 0x4000_0000) != 0;
                    let fc_addr = fc_raw & 0x3FFF_FFFF;
                    if is_compressed {
                        let offset = (fc_addr / 2) as usize;
                        if let Some(slice) = word_doc.get(offset..offset.saturating_add(char_len)) {
                            let (decoded, _) =
                                encoding_rs::WINDOWS_1252.decode_without_bom_handling(slice);
                            out.push_str(&decoded);
                        }
                    } else {
                        let offset = fc_addr as usize;
                        let byte_len = char_len.saturating_mul(2);
                        if let Some(slice) = word_doc.get(offset..offset.saturating_add(byte_len)) {
                            let (decoded, _) =
                                encoding_rs::UTF_16LE.decode_without_bom_handling(slice);
                            out.push_str(&decoded);
                        }
                    }
                }
                return Some(out);
            }
            _ => return None,
        }
    }
    None
}

fn clean_legacy_doc_text(raw: &str) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    let mut field_depth = 0usize;
    let mut in_field_instr = false;

    for ch in raw.chars() {
        match ch {
            '\u{0013}' => {
                field_depth += 1;
                in_field_instr = true;
            }
            '\u{0014}' => {
                if field_depth > 0 {
                    in_field_instr = false;
                }
            }
            '\u{0015}' => {
                if field_depth > 0 {
                    field_depth -= 1;
                    if field_depth == 0 {
                        in_field_instr = false;
                    }
                }
            }
            _ if in_field_instr => {}
            '\r' | '\u{000B}' | '\u{000C}' => out.push('\n'),
            '\u{0007}' => {
                if !out.ends_with(['\n', '\t']) && !out.is_empty() {
                    out.push('\t');
                }
            }
            '\u{0001}'..='\u{0008}' | '\u{000E}'..='\u{001F}' => {}
            _ => out.push(ch),
        }
    }
    (!out.trim().is_empty()).then_some(out)
}

// ---------------------------------------------------------------------------
// Legacy OLE2 PowerPoint (.ppt / .pps) via cfb + [MS-PPT] record stream

fn extract_legacy_ppt_text(bytes: &[u8]) -> Option<String> {
    if bytes.starts_with(b"PK\x03\x04") {
        return extract_pptx_text(bytes);
    }
    let mut comp = CompoundFile::open(Cursor::new(bytes)).ok()?;
    if comp.exists("/EncryptedSummary") || comp.exists("EncryptedSummary") {
        return None;
    }
    let doc = read_cfb_stream(&mut comp, &["/PowerPoint Document", "PowerPoint Document"])?;

    let mut list_chunks: Vec<String> = Vec::new();
    let mut shape_chunks: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    walk_ppt_records(
        &doc,
        false,
        false,
        0,
        &mut list_chunks,
        &mut shape_chunks,
        &mut seen,
    );

    let mut combined = String::new();
    for chunk in list_chunks.into_iter().chain(shape_chunks.into_iter()) {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(&chunk);
    }
    (!combined.trim().is_empty()).then_some(combined)
}

fn walk_ppt_records(
    data: &[u8],
    in_slide_list: bool,
    in_slide_or_notes: bool,
    depth: usize,
    list_chunks: &mut Vec<String>,
    shape_chunks: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    if depth >= 64 {
        return;
    }
    let mut pos = 0usize;
    while pos + 8 <= data.len() {
        let ver_inst = u16::from_le_bytes([data[pos], data[pos + 1]]);
        let rec_ver = ver_inst & 0x000F;
        let rec_inst = ver_inst >> 4;
        let rec_type = u16::from_le_bytes([data[pos + 2], data[pos + 3]]);
        let rec_len =
            u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]])
                as usize;
        let payload_start = pos + 8;
        let Some(payload_end) = payload_start.checked_add(rec_len) else {
            break;
        };
        if payload_end > data.len() {
            break;
        }
        let payload = &data[payload_start..payload_end];

        if rec_ver == 0x000F {
            // Skip MainMaster (0x03F8) and master SlideListWithText (0x0FF0 with instance == 1).
            let skip = rec_type == 0x03F8 || (rec_type == 0x0FF0 && rec_inst == 1);
            if !skip {
                let child_in_list = in_slide_list || rec_type == 0x0FF0;
                let child_in_slide = in_slide_or_notes || rec_type == 0x03EE || rec_type == 0x03F0;
                walk_ppt_records(
                    payload,
                    child_in_list,
                    child_in_slide,
                    depth + 1,
                    list_chunks,
                    shape_chunks,
                    seen,
                );
            }
        } else if in_slide_list || in_slide_or_notes {
            let decoded = match rec_type {
                0x0FA0 => {
                    // TextCharsAtom (UTF-16LE)
                    let (s, _) = encoding_rs::UTF_16LE.decode_without_bom_handling(payload);
                    Some(s.replace(['\r', '\u{000B}'], "\n"))
                }
                0x0FA8 => {
                    // TextBytesAtom (Windows-1252)
                    let (s, _) = encoding_rs::WINDOWS_1252.decode_without_bom_handling(payload);
                    Some(s.replace(['\r', '\u{000B}'], "\n"))
                }
                _ => None,
            };
            if let Some(text) = decoded {
                let trimmed = text.trim().to_string();
                if !trimmed.is_empty() && seen.insert(trimmed.clone()) {
                    if in_slide_list {
                        list_chunks.push(trimmed);
                    } else {
                        shape_chunks.push(trimmed);
                    }
                }
            }
        }

        pos = payload_end;
    }
}

/// Concatenates extracted attachment texts for FTS indexing and stemming.
pub fn combine_attachment_text(attachments: &[crate::model::AttachmentText]) -> String {
    let mut out = String::new();
    for att in attachments {
        let text = att.text.trim();
        if text.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        let name = att.filename.trim();
        if !name.is_empty() {
            out.push_str("[Attachment: ");
            out.push_str(name);
            out.push_str("]\n");
        }
        out.push_str(text);
    }
    out
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use cfb::CompoundFile;
    use flate2::{Compression, write::DeflateEncoder};

    use super::*;

    fn build_test_zip(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for &(name, data, compress) in entries {
            let local_off = out.len() as u32;
            let (method, payload): (u16, Vec<u8>) = if compress {
                let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
                enc.write_all(data).unwrap();
                (8, enc.finish().unwrap())
            } else {
                (0, data.to_vec())
            };
            let name_bytes = name.as_bytes();
            // Local file header
            out.extend_from_slice(b"PK\x03\x04");
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name_bytes);
            out.extend_from_slice(&payload);

            // Central directory header
            central.extend_from_slice(b"PK\x01\x02");
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u32.to_le_bytes());
            central.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u32.to_le_bytes());
            central.extend_from_slice(&local_off.to_le_bytes());
            central.extend_from_slice(name_bytes);
        }
        let cd_off = out.len() as u32;
        let cd_size = central.len() as u32;
        out.extend_from_slice(&central);
        // EOCD
        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_off.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    #[test]
    fn rejects_unsupported_and_binary_attachments() {
        assert!(!is_supported_attachment("photo.png"));
        assert!(!is_supported_attachment("winmail.dat"));
        assert!(!is_supported_attachment("archive.zip"));
        assert!(!is_supported_attachment("installer.exe"));
        assert_eq!(extract_attachment_text("photo.png", b"PNGDATA"), None);
        assert_eq!(
            extract_attachment_text("notes.txt", &[0, 1, 2, 3, 0, 0, 0, 0, 4, 5]),
            None
        );
    }

    #[test]
    fn extracts_small_and_encoded_text_files() {
        let small = extract_attachment_text("note.txt", b"Approve Q3 budget: 42k EUR.").unwrap();
        assert_eq!(small, "Approve Q3 budget: 42k EUR.");

        let md = extract_attachment_text("README.MD", b"# Atlas\n\nDeploy Friday.\n").unwrap();
        assert_eq!(md, "# Atlas\n\nDeploy Friday.");

        // UTF-16LE with BOM
        let mut utf16 = vec![0xFF, 0xFE];
        for unit in "Réunion équipe".encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(
            extract_attachment_text("agenda.txt", &utf16).unwrap(),
            "Réunion équipe"
        );
    }

    #[test]
    fn extracts_rtf_and_html_and_xml() {
        let rtf = br#"{\rtf1\ansi{\fonttbl{\f0 Arial;}}\pard Project \b Atlas\b0  approved \'e9.\par Next: \u8364?500.\par}"#;
        let extracted = extract_rtf_text(rtf).unwrap();
        assert!(extracted.contains("Project Atlas approved é."));
        assert!(extracted.contains("Next: €500."));
        assert!(!extracted.contains("Arial"));

        let html = b"<html><body><h1>Summary</h1><p>Contract signed.</p></body></html>";
        let html_out = extract_attachment_text("summary.htm", html).unwrap();
        assert!(html_out.contains("Summary"));
        assert!(html_out.contains("Contract signed."));

        let xml = b"<report><title>Audit</title><status>Passed</status></report>";
        let xml_out = extract_attachment_text("audit.xml", xml).unwrap();
        assert!(xml_out.contains("Audit"));
        assert!(xml_out.contains("Passed"));
    }

    #[test]
    fn extracts_docx_pptx_and_xlsx() {
        let doc_xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>Vendor agreement</w:t></w:r><w:r><w:instrText>HYPERLINK</w:instrText></w:r></w:p>
    <w:p><w:del><w:r><w:delText>Old draft</w:delText></w:r></w:del><w:r><w:t>Final target: June 15</w:t></w:r></w:p>
  </w:body>
</w:document>"#;
        let docx = build_test_zip(&[("word/document.xml", doc_xml, true)]);
        let docx_text = extract_attachment_text("contract.docx", &docx).unwrap();
        assert!(docx_text.contains("Vendor agreement"));
        assert!(docx_text.contains("Final target: June 15"));
        assert!(!docx_text.contains("HYPERLINK"));
        assert!(!docx_text.contains("Old draft"));

        let slide1 = br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>Q2 Revenue</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#;
        let slide2 = br#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>Launch Milestone</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#;
        let pptx = build_test_zip(&[
            ("ppt/slides/slide2.xml", slide2, false),
            ("ppt/slides/slide1.xml", slide1, true),
        ]);
        let pptx_text = extract_attachment_text("deck.pptx", &pptx).unwrap();
        assert!(pptx_text.contains("[Slide 1]\nQ2 Revenue"));
        assert!(pptx_text.contains("[Slide 2]\nLaunch Milestone"));
        assert!(pptx_text.find("[Slide 1]") < pptx_text.find("[Slide 2]"));

        let ss = br#"<sst><si><t>Item</t></si><si><t>Budget</t></si></sst>"#;
        let wb = br#"<workbook><sheets><sheet name="Forecast" sheetId="1"/></sheets></workbook>"#;
        let sh1 = br#"<worksheet><sheetData><row><c t="s"><v>0</v></c><c t="s"><v>1</v></c></row><row><c t="inlineStr"><is><t>Server</t></is></c><c><v>12000</v></c></row></sheetData></worksheet>"#;
        let xlsx = build_test_zip(&[
            ("xl/sharedStrings.xml", ss, true),
            ("xl/workbook.xml", wb, true),
            ("xl/worksheets/sheet1.xml", sh1, true),
        ]);
        let xlsx_text = extract_attachment_text("budget.xlsx", &xlsx).unwrap();
        assert!(xlsx_text.contains("[Sheet: Forecast]"));
        assert!(xlsx_text.contains("Item\tBudget"));
        assert!(xlsx_text.contains("Server\t12000"));
    }

    #[test]
    fn extracts_libreoffice_odt_odp_and_ods() {
        let odt_xml = br#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:text><text:h>Minutes</text:h><text:p>Action<text:s text:c="2"/>required by Friday.</text:p></office:text></office:body></office:document-content>"#;
        let odt = build_test_zip(&[("content.xml", odt_xml, true)]);
        let odt_text = extract_attachment_text("minutes.odt", &odt).unwrap();
        assert!(odt_text.contains("Minutes"));
        assert!(odt_text.contains("Action  required by Friday."));

        let odp_xml = br#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:presentation><draw:page draw:name="Intro"><draw:frame><draw:text-box><text:p>Architecture Overview</text:p></draw:text-box></draw:frame></draw:page></office:presentation></office:body></office:document-content>"#;
        let odp = build_test_zip(&[("content.xml", odp_xml, false)]);
        let odp_text = extract_attachment_text("slides.odp", &odp).unwrap();
        assert!(odp_text.contains("[Slide 1: Intro]"));
        assert!(odp_text.contains("Architecture Overview"));

        let ods_xml = br#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><office:body><office:spreadsheet><table:table table:name="Costs"><table:table-row><table:table-cell><text:p>Hosting</text:p></table:table-cell><table:table-cell><text:p>450</text:p></table:table-cell></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#;
        let ods = build_test_zip(&[("content.xml", ods_xml, true)]);
        let ods_text = extract_attachment_text("costs.ods", &ods).unwrap();
        assert!(ods_text.contains("[Sheet: Costs]"));
        assert!(ods_text.contains("Hosting\t450"));
    }

    #[test]
    fn extracts_legacy_ole2_doc_and_ppt() {
        // Synthetic MS-DOC with 1 compressed piece at WordDocument offset 0x200
        let sample_text = b"Signed NDA attached.\rEffective immediately.\r";
        let mut word_doc = vec![0u8; 0x200 + sample_text.len()];
        word_doc[0..2].copy_from_slice(&0xA5ECu16.to_le_bytes());
        word_doc[0x0A..0x0C].copy_from_slice(&0x0200u16.to_le_bytes()); // fWhichTblStm = 1Table
        word_doc[0x4C..0x50].copy_from_slice(&(sample_text.len() as u32).to_le_bytes());
        word_doc[0x01A2..0x01A6].copy_from_slice(&0u32.to_le_bytes()); // fcClx = 0

        // Build Clx: clxt=2, lcb=16 (2 CPs = 8 bytes + 1 Pcd = 8 bytes)
        let mut clx = Vec::new();
        clx.push(0x02);
        clx.extend_from_slice(&16u32.to_le_bytes());
        clx.extend_from_slice(&0u32.to_le_bytes());
        clx.extend_from_slice(&(sample_text.len() as u32).to_le_bytes());
        // Pcd: 2 bytes attr, 4 bytes fc (compressed bit 0x40000000 | (0x200 * 2)), 2 bytes prm
        let fc = 0x4000_0000u32 | (0x200u32 * 2);
        clx.extend_from_slice(&0u16.to_le_bytes());
        clx.extend_from_slice(&fc.to_le_bytes());
        clx.extend_from_slice(&0u16.to_le_bytes());
        word_doc[0x01A6..0x01AA].copy_from_slice(&(clx.len() as u32).to_le_bytes());
        word_doc[0x200..0x200 + sample_text.len()].copy_from_slice(sample_text);

        let mut doc_cfb = CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        doc_cfb
            .create_stream("/WordDocument")
            .unwrap()
            .write_all(&word_doc)
            .unwrap();
        doc_cfb
            .create_stream("/1Table")
            .unwrap()
            .write_all(&clx)
            .unwrap();
        let doc_bytes = doc_cfb.into_inner().into_inner();
        let doc_text = extract_attachment_text("nda.doc", &doc_bytes).unwrap();
        assert_eq!(doc_text, "Signed NDA attached.\nEffective immediately.");

        // Synthetic MS-PPT with SlideListWithText (0x0FF0) containing TextBytesAtom (0x0FA8)
        // and a MainMaster (0x03F8) placeholder that must be ignored.
        let mut ppt_stream = Vec::new();
        let master_txt = b"Click to edit Master title";
        let mut master_atom = Vec::new();
        master_atom.extend_from_slice(&0u16.to_le_bytes());
        master_atom.extend_from_slice(&0x0FA8u16.to_le_bytes());
        master_atom.extend_from_slice(&(master_txt.len() as u32).to_le_bytes());
        master_atom.extend_from_slice(master_txt);
        ppt_stream.extend_from_slice(&0x000Fu16.to_le_bytes()); // container
        ppt_stream.extend_from_slice(&0x03F8u16.to_le_bytes()); // MainMaster
        ppt_stream.extend_from_slice(&(master_atom.len() as u32).to_le_bytes());
        ppt_stream.extend_from_slice(&master_atom);

        let slide_txt = b"Roadmap Q4 approved";
        let mut slide_atom = Vec::new();
        slide_atom.extend_from_slice(&0u16.to_le_bytes());
        slide_atom.extend_from_slice(&0x0FA8u16.to_le_bytes());
        slide_atom.extend_from_slice(&(slide_txt.len() as u32).to_le_bytes());
        slide_atom.extend_from_slice(slide_txt);
        ppt_stream.extend_from_slice(&0x000Fu16.to_le_bytes()); // container
        ppt_stream.extend_from_slice(&0x0FF0u16.to_le_bytes()); // SlideListWithText
        ppt_stream.extend_from_slice(&(slide_atom.len() as u32).to_le_bytes());
        ppt_stream.extend_from_slice(&slide_atom);

        let mut ppt_cfb = CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        ppt_cfb
            .create_stream("/PowerPoint Document")
            .unwrap()
            .write_all(&ppt_stream)
            .unwrap();
        let ppt_bytes = ppt_cfb.into_inner().into_inner();
        let ppt_text = extract_attachment_text("roadmap.ppt", &ppt_bytes).unwrap();
        assert_eq!(ppt_text, "Roadmap Q4 approved");
    }

    #[test]
    fn zip_handles_data_descriptors_and_rejects_encrypted_entries() {
        // Encrypted Stored entry (general purpose bit 0 = 0x0001) must be rejected.
        let odt_xml = b"<office:document-content><office:body><office:text><text:p>Secret</text:p></office:text></office:body></office:document-content>";
        let mut encrypted = build_test_zip(&[("content.xml", odt_xml, false)]);
        // Set bit 0 in local header (offset 6) and central directory (find PK\x01\x02).
        encrypted[6] |= 0x01;
        if let Some(cd_pos) = encrypted.windows(4).position(|w| w == b"PK\x01\x02") {
            encrypted[cd_pos + 8] |= 0x01;
        }
        assert_eq!(extract_attachment_text("locked.odt", &encrypted), None);

        // Data-descriptor ZIP (bit 3 = 0x0008, local header sizes zeroed, CD sizes populated) must still extract.
        let mut dd = build_test_zip(&[("content.xml", odt_xml, true)]);
        dd[6] |= 0x08;
        dd[18..26].fill(0);
        if let Some(cd_pos) = dd.windows(4).position(|w| w == b"PK\x01\x02") {
            dd[cd_pos + 8] |= 0x08;
        }
        assert_eq!(extract_attachment_text("dd.odt", &dd).as_deref(), Some("Secret"));
    }

    #[test]
    fn ppt_container_recursion_is_depth_bounded() {
        // Build 80 nested SlideListWithText containers around a TextBytesAtom: depth cap must prevent stack overflow.
        let txt = b"Deep slide text";
        let mut blob = Vec::new();
        blob.extend_from_slice(&0u16.to_le_bytes());
        blob.extend_from_slice(&0x0FA8u16.to_le_bytes());
        blob.extend_from_slice(&(txt.len() as u32).to_le_bytes());
        blob.extend_from_slice(txt);
        for _ in 0..80 {
            let mut wrapped = Vec::with_capacity(8 + blob.len());
            wrapped.extend_from_slice(&0x000Fu16.to_le_bytes());
            wrapped.extend_from_slice(&0x0FF0u16.to_le_bytes());
            wrapped.extend_from_slice(&(blob.len() as u32).to_le_bytes());
            wrapped.extend_from_slice(&blob);
            blob = wrapped;
        }
        let mut ppt_cfb = CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        ppt_cfb
            .create_stream("/PowerPoint Document")
            .unwrap()
            .write_all(&blob)
            .unwrap();
        let ppt_bytes = ppt_cfb.into_inner().into_inner();
        assert_eq!(extract_attachment_text("deep.ppt", &ppt_bytes), None);
    }
}
