mod agent;
mod context;

use std::time::Duration;

use anyhow::{Context, Result, bail};
use rig::{
    completion::{Chat, Prompt},
    embeddings::EmbeddingModel,
    prelude::*,
    providers::openai,
};
use serde::{Deserialize, Serialize};

use crate::model::{AssignedLabel, EmailRecord, LabelDefinition, LabelerBackend};

pub use self::agent::{ProgressFn, run_agent};
#[cfg(test)]
pub(crate) use self::agent::{extract_citation_ids, linkify_citations};
#[allow(unused_imports)]
pub use self::context::{
    AGENT_SYSTEM_PROMPT, AGENT_SYSTEM_PROMPT_FR, ChatTurn, agent_system_prompt, build_context,
};
pub(crate) use self::context::{clean_email_body, strip_mail_links};
pub use crate::i18n::Language;

/// Remote embeddings / hybrid search are hidden from the UI and disabled: retrieval is
/// BM25-only. The code paths are kept so they can be re-enabled later.
pub const EMBEDDINGS_ENABLED: bool = false;

const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LlmConfig {
    pub endpoint: String,
    pub model: String,
    #[serde(default)]
    pub use_remote_embeddings: bool,
    #[serde(default)]
    pub embedding_model: String,
    #[serde(default)]
    pub bearer_token: String,
    #[serde(default = "default_context_chars")]
    pub max_context_chars: usize,
    /// Number of emails retrieved from the index per chat question (before thread expansion).
    #[serde(default = "default_context_emails")]
    pub max_context_emails: usize,
    #[serde(default)]
    pub labeler_backend: LabelerBackend,
    #[serde(default)]
    pub laya_gguf_path: String,
    #[serde(default)]
    pub language: Language,
    #[serde(default = "default_true")]
    pub dark_mode: bool,
}

fn default_true() -> bool {
    true
}

fn default_context_chars() -> usize {
    16_000
}

pub const MIN_CONTEXT_EMAILS: usize = 1;
pub const MAX_CONTEXT_EMAILS: usize = 100;

fn default_context_emails() -> usize {
    8
}

impl LlmConfig {
    /// Default number of results (one per thread) returned by each agent search,
    /// clamped to a sane range.
    pub fn context_email_limit(&self) -> usize {
        self.max_context_emails
            .clamp(MIN_CONTEXT_EMAILS, MAX_CONTEXT_EMAILS)
    }

    /// Whether remote embeddings should be used (always false while they are disabled).
    pub fn embeddings_active(&self) -> bool {
        EMBEDDINGS_ENABLED && self.use_remote_embeddings
    }
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            endpoint: "https://llm.internal.example/v1/chat/completions".into(),
            model: "private-model".into(),
            use_remote_embeddings: false,
            embedding_model: String::new(),
            bearer_token: String::new(),
            max_context_chars: default_context_chars(),
            max_context_emails: default_context_emails(),
            labeler_backend: LabelerBackend::Laya,
            laya_gguf_path: String::new(),
            language: Language::default(),
            dark_mode: true,
        }
    }
}

pub(crate) fn build_http_client(url: &url::Url) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .https_only(url.scheme() == "https")
        .connect_timeout(HTTP_CONNECT_TIMEOUT)
        .timeout(HTTP_REQUEST_TIMEOUT)
        .build()
        .context("build secure HTTP client")
}

fn to_rig_history(history: &[ChatTurn]) -> Vec<rig::completion::Message> {
    let mut rig_messages = Vec::new();
    for turn in history {
        if turn.role.eq_ignore_ascii_case("user") {
            rig_messages.push(rig::completion::Message::user(&turn.content));
        } else if turn.role.eq_ignore_ascii_case("assistant") {
            let cleaned = strip_mail_links(&turn.content);
            rig_messages.push(rig::completion::Message::assistant(&cleaned));
        }
    }
    rig_messages
}

#[allow(dead_code)]
pub fn ask(config: &LlmConfig, question: &str, context: &str) -> Result<String> {
    ask_with_history(config, question, context, &[])
}

pub fn ask_with_history(
    config: &LlmConfig,
    question: &str,
    context: &str,
    history: &[ChatTurn],
) -> Result<String> {
    let url = validate_endpoint(&config.endpoint)?;
    let endpoint_for_log = endpoint_for_log(&url);
    let base_url = openai_base_url(&url)?;
    let http_client = build_http_client(&url)?;
    let client = openai::Client::builder()
        .api_key(config.bearer_token.clone())
        .base_url(base_url.as_str())
        .http_client(http_client)
        .build()
        .context("build Rig OpenAI-compatible client")?
        .completions_api();

    let agent = client
        .agent(&config.model)
        .preamble(agent_system_prompt(config.language))
        .context(context)
        .build();

    let mut rig_messages = to_rig_history(history);

    debug_log(format_args!(
        "LLM request: POST {endpoint_for_log}; model={:?}; auth={}; history_turns={}; question_bytes={}; context_bytes={}",
        config.model,
        if config.bearer_token.is_empty() {
            "none"
        } else {
            "bearer (hidden)"
        },
        rig_messages.len(),
        question.len(),
        context.len()
    ));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("start Rig runtime")?;
    runtime
        .block_on(async {
            if rig_messages.is_empty() {
                agent.prompt(question).await
            } else {
                agent.chat(question, &mut rig_messages).await
            }
        })
        .map_err(|error| {
            debug_log(format_args!("Rig agent error: {error:#}"));
            anyhow::anyhow!(error).context(format!("Rig agent request to {endpoint_for_log}"))
        })
}

pub fn judge_labels(
    config: &LlmConfig,
    email: &EmailRecord,
    definitions: &[LabelDefinition],
) -> Result<Vec<AssignedLabel>> {
    if definitions.is_empty() {
        bail!("configure at least one label before running LLM Judge");
    }
    let url = validate_endpoint(&config.endpoint)?;
    let endpoint_for_log = endpoint_for_log(&url);
    let base_url = openai_base_url(&url)?;
    let http_client = build_http_client(&url)?;
    let client = openai::Client::builder()
        .api_key(config.bearer_token.clone())
        .base_url(base_url.as_str())
        .http_client(http_client)
        .build()
        .context("build Rig OpenAI-compatible client")?
        .completions_api();

    let mut rubric = String::from("Available labels and criteria:\n");
    for def in definitions {
        rubric.push_str(&format!("- {}: {}\n", def.name, def.description));
    }
    let prompt = format!(
        "{rubric}\nEmail to evaluate:\nDate: {}\nFrom: {}\nTo: {}\nFolder: {}\nSubject: {}\nBody:\n{}\n\n\
         Respond ONLY with JSON in the exact format: {{\"labels\": [{{\"label\": \"<exact label name>\", \"confidence\": 0.92}}]}} \
         selecting 1 or 2 matching labels from the list above.",
        email.sent_at.as_deref().unwrap_or("unknown"),
        email.sender,
        email.recipients,
        email.folder,
        email.subject,
        email.excerpt(3_500)
    );

    let agent = client
        .agent(&config.model)
        .preamble(
            "You are an expert email classification judge. Evaluate the email strictly against the supplied label criteria and output valid JSON only.",
        )
        .build();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("start Rig runtime")?;
    let response = runtime
        .block_on(async { agent.prompt(&prompt).await })
        .map_err(|error| {
            anyhow::anyhow!(error).context(format!("LLM judge request to {endpoint_for_log}"))
        })?;
    parse_judge_response(&response, definitions)
}

#[derive(Deserialize)]
struct JudgeResponsePayload {
    labels: Vec<JudgeItem>,
}

#[derive(Deserialize)]
struct JudgeItem {
    label: String,
    #[serde(default)]
    confidence: Option<f64>,
}

pub(crate) fn parse_judge_response(
    raw: &str,
    definitions: &[LabelDefinition],
) -> Result<Vec<AssignedLabel>> {
    let trimmed = raw.trim();
    let json_slice = trimmed
        .find('{')
        .and_then(|start| trimmed.rfind('}').map(|end| &trimmed[start..=end]))
        .unwrap_or(trimmed);

    let mut assigned = Vec::new();
    if let Ok(parsed) = serde_json::from_str::<JudgeResponsePayload>(json_slice) {
        for item in parsed.labels {
            if let Some(def) = definitions
                .iter()
                .find(|d| d.name.eq_ignore_ascii_case(item.label.trim()))
                && !assigned.iter().any(|a: &AssignedLabel| a.label == def.name)
            {
                assigned.push(AssignedLabel {
                    label: def.name.clone(),
                    confidence: Some(item.confidence.unwrap_or(0.85).clamp(0.0, 1.0)),
                    source: "llm-judge".into(),
                });
            }
        }
    }

    if assigned.is_empty() {
        let lower = raw.to_lowercase();
        for def in definitions {
            if lower.contains(&def.name.to_lowercase()) {
                assigned.push(AssignedLabel {
                    label: def.name.clone(),
                    confidence: Some(0.80),
                    source: "llm-judge".into(),
                });
                if assigned.len() >= 2 {
                    break;
                }
            }
        }
    }

    if assigned.is_empty() {
        bail!("LLM Judge did not return any configured label");
    }
    Ok(assigned)
}

pub fn embed(config: &LlmConfig, texts: Vec<String>) -> Result<Vec<Vec<f64>>> {
    if config.embedding_model.trim().is_empty() {
        bail!("enter an embedding model before enabling remote embeddings");
    }
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    let url = validate_endpoint(&config.endpoint)?;
    let base_url = openai_base_url(&url)?;
    let http_client = build_http_client(&url)?;
    let client = openai::CompletionsClient::builder()
        .api_key(config.bearer_token.clone())
        .base_url(base_url.as_str())
        .http_client(http_client)
        .build()
        .context("build Rig embedding client")?;
    let model = client.embedding_model(config.embedding_model.trim());
    debug_log(format_args!(
        "Embedding request: POST {}/embeddings; model={:?}; documents={}",
        base_url.as_str().trim_end_matches('/'),
        config.embedding_model,
        texts.len()
    ));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("start Rig runtime")?;
    runtime
        .block_on(async { model.embed_texts(texts).await })
        .map(|embeddings| embeddings.into_iter().map(|item| item.vec).collect())
        .map_err(|error| anyhow::anyhow!(error).context("remote embedding request failed"))
}

pub(crate) fn openai_base_url(endpoint: &url::Url) -> Result<url::Url> {
    let path = endpoint.path().trim_end_matches('/');
    let base_path = path
        .strip_suffix("/chat/completions")
        .context("LLM endpoint must end with /chat/completions")?;
    let mut base = endpoint.clone();
    base.set_path(base_path);
    base.set_query(None);
    base.set_fragment(None);
    Ok(base)
}

pub(crate) fn endpoint_for_log(url: &url::Url) -> String {
    let host = url.host_str().unwrap_or("<missing-host>");
    let port = url
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    format!("{}://{host}{port}{}", url.scheme(), url.path())
}

fn debug_log(args: std::fmt::Arguments<'_>) {
    #[cfg(debug_assertions)]
    eprintln!("[pstbeetle] {args}");
    #[cfg(not(debug_assertions))]
    let _ = args;
}

pub(crate) fn validate_endpoint(endpoint: &str) -> Result<url::Url> {
    let url = url::Url::parse(endpoint).context("invalid LLM endpoint")?;
    match (url.scheme(), url.host_str()) {
        ("https", Some(_)) => Ok(url),
        ("http", Some("127.0.0.1")) => Ok(url),
        ("http", _) => {
            bail!("HTTP is allowed only for an LLM on 127.0.0.1; use HTTPS for every other host")
        }
        _ => bail!("the LLM endpoint must use HTTPS, or HTTP on 127.0.0.1"),
    }
}
