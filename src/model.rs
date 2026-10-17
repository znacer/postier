use serde::{Deserialize, Serialize};

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
    Laya,
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
        let mut out = String::new();
        let trimmed = self.body.trim();
        for ch in trimmed.chars().take(max_chars) {
            out.push(if ch == '\n' || ch == '\r' { ' ' } else { ch });
        }
        if trimmed.chars().count() > max_chars {
            out.push('…');
        }
        out
    }
}

#[derive(Clone, Debug)]
pub struct SearchHit {
    pub id: i64,
    pub source_key: String,
    pub subject: String,
    pub sender: String,
    pub recipients: String,
    pub sent_at: Option<String>,
    pub folder: String,
    pub body: String,
    pub score: f64,
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
    pub fn excerpt(&self, max_chars: usize) -> String {
        let mut out = String::new();
        for ch in self.body.chars().take(max_chars) {
            out.push(ch);
        }
        if self.body.chars().count() > max_chars {
            out.push('…');
        }
        out
    }
}
