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

pub trait MailSource {
    fn read_messages(&self, path: &Path) -> Result<Vec<Email>>;
}

/// Development importer. One serialized `Email` per line permits end-to-end testing
/// without handling real mail outside the workstation.
pub struct JsonlSource;

impl MailSource for JsonlSource {
    fn read_messages(&self, path: &Path) -> Result<Vec<Email>> {
        let source = path.to_string_lossy().into_owned();
        BufReader::new(File::open(path)?)
            .lines()
            .enumerate()
            .map(|(line_no, line)| {
                let mut email: Email = serde_json::from_str(&line?)
                    .with_context(|| format!("invalid message on line {}", line_no + 1))?;
                email.source_key = source.clone();
                Ok(email)
            })
            .collect()
    }
}

pub struct PstSource;

impl MailSource for PstSource {
    fn read_messages(&self, path: &Path) -> Result<Vec<Email>> {
        // outlook-pst opens the archive read-only and supports both ANSI and Unicode PSTs.
        let store = outlook_pst::open_store(path).context("open PST read-only")?;
        let root_id = store
            .properties()
            .ipm_sub_tree_entry_id()
            .context("locate the PST mail subtree")?;
        let root = store
            .open_folder(&root_id)
            .context("open the PST mail subtree")?;
        let mut emails = Vec::new();
        walk_folder(
            store,
            root,
            String::new(),
            &path.to_string_lossy(),
            &mut emails,
        )?;
        Ok(emails)
    }
}

fn walk_folder(
    store: Rc<dyn Store>,
    folder: Rc<dyn Folder>,
    parent: String,
    source: &str,
    out: &mut Vec<Email>,
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
            if let Ok(email) = read_message(store.clone(), row, &path, source) {
                out.push(email);
            }
        }
    }
    if let Some(children) = folder.hierarchy_table() {
        for row in children.rows_matrix() {
            let node = NodeId::from(u32::from(row.id()));
            let entry_id = store.properties().make_entry_id(node)?;
            if let Ok(child) = store.open_folder(&entry_id) {
                walk_folder(store.clone(), child, path.clone(), source, out)?;
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
    let sender = text_prop(props.get(0x5D01))
        .or_else(|| text_prop(props.get(0x0C1A)))
        .unwrap_or_default();
    let message_id =
        text_prop(props.get(0x1035)).unwrap_or_else(|| format!("pst-node-{}", u32::from(row.id())));
    let sent_at = time_prop(props.get(0x0039)).or_else(|| time_prop(props.get(0x0E06)));
    let body = text_prop(props.get(0x1000))
        .or_else(|| text_prop(props.get(0x1013)))
        .unwrap_or_default();
    let recipients = message
        .recipient_table()
        .map(|t| recipient_names(t.as_ref()))
        .unwrap_or_default();
    Ok(Email {
        source_key: source.into(),
        message_id,
        conversation_id: String::new(),
        folder: folder.into(),
        subject,
        sender,
        recipients,
        sent_at,
        body,
        attachment_names: String::new(),
    })
}

fn text_prop(value: Option<&PropertyValue>) -> Option<String> {
    match value? {
        PropertyValue::String8(v) => Some(v.to_string()),
        PropertyValue::Unicode(v) => Some(v.to_string()),
        PropertyValue::Binary(v) => String::from_utf8(v.buffer().to_vec()).ok(),
        _ => None,
    }
    .map(|s| s.trim_matches('\0').to_string())
    .filter(|s| !s.is_empty())
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

fn recipient_names(table: &dyn TableContext) -> String {
    let context = table.context();
    table
        .rows_matrix()
        .filter_map(|row| {
            context
                .columns()
                .iter()
                .zip(row.columns(context).ok()?)
                .find_map(|(col, raw)| {
                    if col.prop_id() != 0x3001 {
                        return None;
                    }
                    text_prop(Some(
                        &table.read_column(raw.as_ref()?, col.prop_type()).ok()?,
                    ))
                })
        })
        .collect::<Vec<_>>()
        .join(", ")
}

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
