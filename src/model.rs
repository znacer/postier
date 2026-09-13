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
