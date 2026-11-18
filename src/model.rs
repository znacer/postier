use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentText {
    pub filename: String,
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub size_bytes: u64,
    pub text: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Email {
    pub source_key: String,
    pub message_id: String,
    pub conversation_id: String,
    pub folder: String,
    pub subject: String,
    pub sender: String,
    pub recipients: String,
    pub sent_at: Option<String>,
    pub body: String,
    pub attachment_names: String,
    #[serde(default)]
    pub attachments: Vec<AttachmentText>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AssignedLabel {
    pub label: String,
    pub confidence: Option<f64>,
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LabelDefinition {
    pub name: String,
    pub description: String,
    pub color: String,
}

pub fn default_label_definitions() -> Vec<LabelDefinition> {
    vec![
        LabelDefinition {
            name: "Action Required".into(),
            description:
                "Requires a reply, task, approval, or concrete follow-up action from the recipient"
                    .into(),
            color: "#D9534F".into(),
        },
        LabelDefinition {
            name: "Decision".into(),
            description: "Records a decision, sign-off, milestone agreement, or policy resolution"
                .into(),
            color: "#166EA6".into(),
        },
        LabelDefinition {
            name: "Project Update".into(),
            description:
                "Status report, schedule update, engineering progress, or roadmap discussion".into(),
            color: "#2E8B57".into(),
        },
        LabelDefinition {
            name: "Urgent".into(),
            description:
                "Time-sensitive blocker, outage, critical deadline, or immediate escalation".into(),
            color: "#E67E22".into(),
        },
        LabelDefinition {
            name: "FYI".into(),
            description:
                "Informational notice, newsletter, general announcement, or archival reference"
                    .into(),
            color: "#7F8C8D".into(),
        },
    ]
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LabelerBackend {
    #[default]
    #[serde(alias = "laya")]
    Laya,
    #[serde(alias = "llm_judge", alias = "llm-judge", alias = "llmjudge")]
    LlmJudge,
}

impl LabelerBackend {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Laya => "Laya (Local GGUF)",
            Self::LlmJudge => "LLM Judge",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MailSort {
    #[default]
    DateDesc,
    DateAsc,
    SubjectAsc,
    SenderAsc,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MailScope {
    #[default]
    AllMessages,
    Source(String),
    Folder {
        source_key: String,
        folder_prefix: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LabelFilter {
    #[default]
    Any,
    Labeled(String),
    Unlabeled,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct EmailRecord {
    pub id: i64,
    pub source_key: String,
    pub message_id: String,
    pub conversation_id: String,
    pub folder: String,
    pub subject: String,
    pub sender: String,
    pub recipients: String,
    pub sent_at: Option<String>,
    pub body: String,
    pub attachment_names: String,
    pub labels: Vec<AssignedLabel>,
}

impl EmailRecord {
    pub fn excerpt(&self, max_chars: usize) -> String {
        let plain = crate::email_html::html_to_plain_text(&self.body);
        let mut out = String::new();
        let trimmed = plain.trim();
        for ch in trimmed.chars().take(max_chars) {
            out.push(if ch == '\n' || ch == '\r' { ' ' } else { ch });
        }
        if trimmed.chars().count() > max_chars {
            out.push_str("...");
        }
        out
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub struct SearchHit {
    pub id: i64,
    pub source_key: String,
    pub subject: String,
    pub sender: String,
    pub recipients: String,
    pub sent_at: Option<String>,
    pub folder: String,
    pub body: String,
    pub attachment_names: String,
    pub conversation_id: String,
    pub message_id: String,
    pub labels: Vec<AssignedLabel>,
    pub score: f64,
    /// Indexed plain text (markup, quoted replies and signatures removed).
    pub body_text: String,
    /// Number of matching messages in this hit's thread (search results are one per thread).
    pub thread_matches: usize,
    /// Extracted plain text for supported common attachments.
    pub attachments: Vec<AttachmentText>,
}

#[derive(Clone, Debug)]
pub struct IndexedFolder {
    pub path: String,
    pub enabled: bool,
    pub message_count: u64,
}

#[derive(Clone, Debug)]
pub struct IndexedSource {
    pub key: String,
    pub name: String,
    pub enabled: bool,
    pub message_count: u64,
    pub folders: Vec<IndexedFolder>,
}

#[derive(Clone, Debug)]
pub struct EmbeddingDocument {
    pub id: i64,
    pub text: String,
}

impl SearchHit {
    pub fn archive_name(&self) -> &str {
        std::path::Path::new(&self.source_key)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&self.source_key)
    }

    pub fn excerpt(&self, max_chars: usize) -> String {
        let plain = crate::email_html::html_to_plain_text(&self.body);
        let mut out = String::new();
        for ch in plain.chars().take(max_chars) {
            out.push(ch);
        }
        if plain.chars().count() > max_chars {
            out.push_str("...");
        }
        out
    }
}
