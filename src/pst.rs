use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
    rc::Rc,
};

use crate::model::Email;
use anyhow::{Context, Result, bail};
use outlook_pst::{
    ltp::{prop_context::PropertyValue, table_context::TableContext},
    messaging::{
        folder::Folder,
        store::{EntryId, Store},
    },
    ndb::node_id::NodeId,
};

pub const IMPORT_BATCH_SIZE: usize = 500;

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
        walk_folder(
            store,
            root,
            String::new(),
            &path.to_string_lossy(),
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
    batch: &mut Vec<Email>,
    total: &mut usize,
    skipped: &mut usize,
    on_batch: &mut dyn FnMut(Vec<Email>) -> Result<()>,
) -> Result<()> {
    let name = folder
        .properties()
        .display_name()
        .unwrap_or_else(|_| "Unnamed".into());
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
            let node = NodeId::from(u32::from(row.id()));
            let entry_id = store.properties().make_entry_id(node)?;
            if let Ok(child) = store.open_folder(&entry_id) {
                walk_folder(
                    store.clone(),
                    child,
                    path.clone(),
                    source,
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
    let subject = text_prop(props.get(0x0037)).unwrap_or_default();
    let sender_name = text_prop(props.get(0x0C1A));
    let sender_email = text_prop(props.get(0x5D01))
        .or_else(|| text_prop(props.get(0x0C1F)).filter(|s| s.contains('@')));
    let sender = format_name_and_email(sender_name.as_deref(), sender_email.as_deref());
    let message_id =
        text_prop(props.get(0x1035)).unwrap_or_else(|| format!("pst-node-{}", u32::from(row.id())));
    let conversation_id = extract_conversation_id(props, &subject);
    let sent_at = time_prop(props.get(0x0039)).or_else(|| time_prop(props.get(0x0E06)));
    let body = text_prop(props.get(0x1013))
        .or_else(|| text_prop(props.get(0x1000)))
        .unwrap_or_default();
    let display_to = text_prop(props.get(0x0E04));
    let display_cc = text_prop(props.get(0x0E03));
    let recipients = message
        .recipient_table()
        .map(|t| extract_recipients(t.as_ref(), display_to.as_deref(), display_cc.as_deref()))
        .unwrap_or_else(|| combine_to_and_cc(display_to.as_deref(), display_cc.as_deref()));
    let attachment_names = message
        .attachment_table()
        .map(|t| extract_attachment_names(t.as_ref()))
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
    if let Some(topic) = text_prop(props.get(0x0070)) {
        return topic;
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

fn extract_attachment_names(table: &dyn TableContext) -> String {
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
                0x3707 => long_name = text_prop(Some(&prop_val)),
                0x3704 => short_name = text_prop(Some(&prop_val)),
                0x3001 => display_name = text_prop(Some(&prop_val)),
                _ => {}
            }
        }
        if let Some(name) = long_name.or(short_name).or(display_name)
            && !names.contains(&name)
        {
            names.push(name);
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
    let to_clean = to.map(str::trim).filter(|s| !s.is_empty());
    let cc_clean = cc.map(str::trim).filter(|s| !s.is_empty());
    match (to_clean, cc_clean) {
        (Some(t), Some(c)) => format!("{t}\nCc: {c}"),
        (Some(t), None) => t.to_string(),
        (None, Some(c)) => format!("Cc: {c}"),
        (None, None) => String::new(),
    }
}

fn text_prop(value: Option<&PropertyValue>) -> Option<String> {
    match value? {
        PropertyValue::String8(v) => Some(v.to_string()),
        PropertyValue::Unicode(v) => Some(v.to_string()),
        PropertyValue::Binary(v) => Some(String::from_utf8_lossy(v.buffer()).into_owned()),
        _ => None,
    }
    .map(|s| s.trim_matches('\0').to_string())
    .filter(|s| !s.is_empty())
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
) -> String {
    let context = table.context();
    let mut to_list = Vec::new();
    let mut cc_list = Vec::new();

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
                0x3001 => display_name = text_prop(Some(&prop_val)),
                0x39FE => smtp_addr = text_prop(Some(&prop_val)),
                0x3003 => email_addr = text_prop(Some(&prop_val)).filter(|s| s.contains('@')),
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
    combine_to_and_cc(to_str.as_deref(), cc_str.as_deref())
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
