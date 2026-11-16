mod agent;
mod budget;
mod context;

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::Duration,
};

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
pub use self::budget::{
    EffectiveBudgets, MIN_CONTEXT_TOKENS, PromptDensity, estimate_tokens,
    trim_chat_turns_to_budget, trim_rig_history_for_completion,
};
#[cfg(test)]
pub(crate) use self::budget::{
    build_judge_prompt_and_max_tokens, build_rag_request_budgets, estimate_message_tokens,
};
#[allow(unused_imports)]
pub use self::context::{
    AGENT_SYSTEM_PROMPT, AGENT_SYSTEM_PROMPT_FR, ChatTurn, agent_system_prompt,
    agent_system_prompt_for_density, build_context, rag_system_prompt,
};
pub(crate) use self::context::{clean_email_body, strip_mail_links};
pub use crate::i18n::Language;

/// Compile-time shipped configuration loaded from the repository root `config.toml`.
pub const EMBEDDED_CONFIG_TOML: &str = include_str!("../config.toml");

/// Remote embeddings / hybrid search are hidden from the UI and disabled by default:
/// retrieval is BM25-only unless enabled via `embeddings_enabled` in `config.toml`.
pub const EMBEDDINGS_ENABLED: bool = false;

pub const MIN_CONTEXT_EMAILS: usize = 1;
pub const MAX_CONTEXT_EMAILS: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    pub endpoint: String,
    pub model: String,
    pub use_remote_embeddings: bool,
    pub embedding_model: String,
    pub bearer_token: String,
    /// Maximum total context window (input + output tokens per request) accepted by the LLM.
    /// `0` disables token-based scaling and uses raw configured character/result limits.
    #[serde(alias = "max_tokens", alias = "context_window_tokens")]
    pub max_context_tokens: usize,
    pub connect_timeout_secs: u64,
    pub request_timeout_secs: u64,
    pub embeddings_enabled: bool,
    pub max_context_chars: usize,
    /// Number of emails retrieved from the index per chat question (before thread expansion).
    pub max_context_emails: usize,
    /// Maximum number of model calls (tool rounds + final answer) per user question.
    #[serde(alias = "max_turns")]
    pub agent_max_turns: usize,
    /// Hard upper limit on results returned by a single `search_emails` call or local search.
    pub max_search_results: usize,
    /// Maximum character length of the matching passage snippet per search result.
    pub snippet_chars: usize,
    /// Maximum character length of an email body returned by `read_email`.
    pub read_email_max_chars: usize,
    /// Maximum number of messages returned by `get_thread`.
    pub thread_max_messages: usize,
    /// Total character budget shared across all messages returned by `get_thread`.
    pub thread_total_chars: usize,
    /// Number of previous user/assistant exchanges included as conversation history.
    pub history_turns: usize,
    pub labeler_backend: LabelerBackend,
    pub laya_gguf_path: String,
    /// Maximum email excerpt length (in chars) sent to the LLM Judge per email.
    pub judge_email_excerpt_chars: usize,
    /// Maximum number of visible emails labeled in a single batch action.
    pub batch_label_limit: usize,
    pub language: Language,
    pub dark_mode: bool,
    /// Maximum number of messages loaded in the explorer table at once.
    pub explorer_message_limit: usize,
}

#[derive(Default, Deserialize)]
struct TomlLlmSection {
    endpoint: Option<String>,
    model: Option<String>,
    bearer_token: Option<String>,
    #[serde(alias = "max_tokens", alias = "context_window_tokens")]
    max_context_tokens: Option<usize>,
    connect_timeout_secs: Option<u64>,
    request_timeout_secs: Option<u64>,
    embeddings_enabled: Option<bool>,
    use_remote_embeddings: Option<bool>,
    embedding_model: Option<String>,
}

#[derive(Default, Deserialize)]
struct TomlAgentSection {
    #[serde(alias = "agent_max_turns", alias = "max_model_calls")]
    max_turns: Option<usize>,
    #[serde(alias = "default_search_results")]
    max_context_emails: Option<usize>,
    max_search_results: Option<usize>,
    snippet_chars: Option<usize>,
    read_email_max_chars: Option<usize>,
    thread_max_messages: Option<usize>,
    thread_total_chars: Option<usize>,
    max_context_chars: Option<usize>,
    history_turns: Option<usize>,
}

#[derive(Default, Deserialize)]
struct TomlLayaSection {
    labeler_backend: Option<LabelerBackend>,
    laya_gguf_path: Option<String>,
    judge_email_excerpt_chars: Option<usize>,
    batch_label_limit: Option<usize>,
}

#[derive(Default, Deserialize)]
struct TomlUiSection {
    language: Option<Language>,
    dark_mode: Option<bool>,
    explorer_message_limit: Option<usize>,
}

#[derive(Default, Deserialize)]
struct TomlConfig {
    #[serde(default)]
    llm: TomlLlmSection,
    #[serde(default)]
    agent: TomlAgentSection,
    #[serde(default)]
    laya: TomlLayaSection,
    #[serde(default)]
    ui: TomlUiSection,
    // Also support top-level flat keys in `config.toml`
    endpoint: Option<String>,
    model: Option<String>,
    bearer_token: Option<String>,
    #[serde(alias = "max_tokens", alias = "context_window_tokens")]
    max_context_tokens: Option<usize>,
    connect_timeout_secs: Option<u64>,
    request_timeout_secs: Option<u64>,
    embeddings_enabled: Option<bool>,
    use_remote_embeddings: Option<bool>,
    embedding_model: Option<String>,
    #[serde(alias = "max_turns", alias = "max_model_calls")]
    agent_max_turns: Option<usize>,
    #[serde(alias = "default_search_results")]
    max_context_emails: Option<usize>,
    max_search_results: Option<usize>,
    snippet_chars: Option<usize>,
    read_email_max_chars: Option<usize>,
    thread_max_messages: Option<usize>,
    thread_total_chars: Option<usize>,
    max_context_chars: Option<usize>,
    history_turns: Option<usize>,
    labeler_backend: Option<LabelerBackend>,
    laya_gguf_path: Option<String>,
    judge_email_excerpt_chars: Option<usize>,
    batch_label_limit: Option<usize>,
    language: Option<Language>,
    dark_mode: Option<bool>,
    explorer_message_limit: Option<usize>,
}

impl LlmConfig {
    const fn builtin_defaults() -> Self {
        Self {
            endpoint: String::new(),
            model: String::new(),
            use_remote_embeddings: false,
            embedding_model: String::new(),
            bearer_token: String::new(),
            max_context_tokens: 16_384,
            connect_timeout_secs: 15,
            request_timeout_secs: 180,
            embeddings_enabled: EMBEDDINGS_ENABLED,
            max_context_chars: 16_000,
            max_context_emails: 8,
            agent_max_turns: agent::MAX_MODEL_CALLS,
            max_search_results: 50,
            snippet_chars: 360,
            read_email_max_chars: 12_000,
            thread_max_messages: 40,
            thread_total_chars: 14_000,
            history_turns: 3,
            labeler_backend: LabelerBackend::Laya,
            laya_gguf_path: String::new(),
            judge_email_excerpt_chars: 3_500,
            batch_label_limit: 50,
            language: Language::French,
            dark_mode: true,
            explorer_message_limit: 300,
        }
    }

    /// Parses a TOML configuration string (supporting `[llm]`, `[agent]`, `[laya]`, `[ui]`
    /// sections as well as top-level flat keys) over the built-in fallback defaults.
    pub fn from_toml_str(raw: &str) -> Result<Self> {
        let parsed: TomlConfig = toml::from_str(raw).context("parse TOML configuration")?;
        let mut cfg = Self {
            endpoint: "https://llm.internal.example/v1/chat/completions".into(),
            model: "private-model".into(),
            ..Self::builtin_defaults()
        };

        macro_rules! apply {
            ($field:ident, $section_val:expr, $flat_val:expr) => {
                if let Some(v) = $flat_val.or($section_val) {
                    cfg.$field = v;
                }
            };
        }

        apply!(endpoint, parsed.llm.endpoint, parsed.endpoint);
        apply!(model, parsed.llm.model, parsed.model);
        apply!(bearer_token, parsed.llm.bearer_token, parsed.bearer_token);
        apply!(
            max_context_tokens,
            parsed.llm.max_context_tokens,
            parsed.max_context_tokens
        );
        apply!(
            connect_timeout_secs,
            parsed.llm.connect_timeout_secs,
            parsed.connect_timeout_secs
        );
        apply!(
            request_timeout_secs,
            parsed.llm.request_timeout_secs,
            parsed.request_timeout_secs
        );
        apply!(
            embeddings_enabled,
            parsed.llm.embeddings_enabled,
            parsed.embeddings_enabled
        );
        apply!(
            use_remote_embeddings,
            parsed.llm.use_remote_embeddings,
            parsed.use_remote_embeddings
        );
        apply!(
            embedding_model,
            parsed.llm.embedding_model,
            parsed.embedding_model
        );

        apply!(
            agent_max_turns,
            parsed.agent.max_turns,
            parsed.agent_max_turns
        );
        apply!(
            max_context_emails,
            parsed.agent.max_context_emails,
            parsed.max_context_emails
        );
        apply!(
            max_search_results,
            parsed.agent.max_search_results,
            parsed.max_search_results
        );
        apply!(
            snippet_chars,
            parsed.agent.snippet_chars,
            parsed.snippet_chars
        );
        apply!(
            read_email_max_chars,
            parsed.agent.read_email_max_chars,
            parsed.read_email_max_chars
        );
        apply!(
            thread_max_messages,
            parsed.agent.thread_max_messages,
            parsed.thread_max_messages
        );
        apply!(
            thread_total_chars,
            parsed.agent.thread_total_chars,
            parsed.thread_total_chars
        );
        apply!(
            max_context_chars,
            parsed.agent.max_context_chars,
            parsed.max_context_chars
        );
        apply!(
            history_turns,
            parsed.agent.history_turns,
            parsed.history_turns
        );

        apply!(
            labeler_backend,
            parsed.laya.labeler_backend,
            parsed.labeler_backend
        );
        apply!(
            laya_gguf_path,
            parsed.laya.laya_gguf_path,
            parsed.laya_gguf_path
        );
        apply!(
            judge_email_excerpt_chars,
            parsed.laya.judge_email_excerpt_chars,
            parsed.judge_email_excerpt_chars
        );
        apply!(
            batch_label_limit,
            parsed.laya.batch_label_limit,
            parsed.batch_label_limit
        );

        apply!(language, parsed.ui.language, parsed.language);
        apply!(dark_mode, parsed.ui.dark_mode, parsed.dark_mode);
        apply!(
            explorer_message_limit,
            parsed.ui.explorer_message_limit,
            parsed.explorer_message_limit
        );

        Ok(cfg)
    }

    /// Configured total context window in tokens, or `None` when set to `0` (unlimited).
    /// Non-zero values below `MIN_CONTEXT_TOKENS` are clamped up to `MIN_CONTEXT_TOKENS`.
    pub fn max_context_tokens_limit(&self) -> Option<usize> {
        (self.max_context_tokens > 0).then(|| self.max_context_tokens.max(MIN_CONTEXT_TOKENS))
    }

    /// Computes effective prompt, tool, history, RAG, and judge budgets scaled to
    /// `max_context_tokens` (when non-zero).
    pub fn effective_budgets(&self) -> EffectiveBudgets {
        EffectiveBudgets::from_config(self)
    }

    /// Default number of results (one per thread) returned by each agent search,
    /// clamped to a sane range.
    pub fn context_email_limit(&self) -> usize {
        self.max_context_emails
            .clamp(MIN_CONTEXT_EMAILS, self.max_search_results_limit())
    }

    /// Maximum number of results returned by a single search call, clamped to a sane range.
    pub fn max_search_results_limit(&self) -> usize {
        self.max_search_results
            .clamp(MIN_CONTEXT_EMAILS, MAX_CONTEXT_EMAILS.max(500))
    }

    /// Maximum number of model turns per agent question.
    pub fn agent_max_turns_limit(&self) -> usize {
        self.agent_max_turns.clamp(1, 100)
    }

    /// Maximum passage snippet length in characters per search hit.
    pub fn snippet_chars_limit(&self) -> usize {
        self.snippet_chars.max(80)
    }

    /// Maximum email body length in characters returned by `read_email`.
    pub fn read_email_max_chars_limit(&self) -> usize {
        self.read_email_max_chars.max(400)
    }

    /// Maximum number of thread messages returned by `get_thread`.
    pub fn thread_max_messages_limit(&self) -> usize {
        self.thread_max_messages.clamp(1, 200)
    }

    /// Total character budget across a thread in `get_thread`.
    pub fn thread_total_chars_limit(&self) -> usize {
        self.thread_total_chars.max(400)
    }

    /// Number of prior conversation pairs sent as chat history.
    pub fn history_turns_limit(&self) -> usize {
        self.history_turns.min(50)
    }

    /// HTTP connection timeout.
    pub fn connect_timeout(&self) -> Duration {
        Duration::from_secs(self.connect_timeout_secs.max(1))
    }

    /// HTTP request timeout.
    pub fn request_timeout(&self) -> Duration {
        Duration::from_secs(self.request_timeout_secs.max(1))
    }

    /// Whether remote embeddings UI / code paths are enabled.
    pub fn embeddings_feature_enabled(&self) -> bool {
        EMBEDDINGS_ENABLED || self.embeddings_enabled
    }

    /// Whether remote embeddings should be used for indexing/queries.
    pub fn embeddings_active(&self) -> bool {
        self.embeddings_feature_enabled() && self.use_remote_embeddings
    }
}

impl Default for LlmConfig {
    fn default() -> Self {
        static SHIPPED_CONFIG: OnceLock<LlmConfig> = OnceLock::new();
        SHIPPED_CONFIG
            .get_or_init(|| {
                Self::from_toml_str(EMBEDDED_CONFIG_TOML)
                    .expect("embedded config.toml must be valid TOML")
            })
            .clone()
    }
}

pub(crate) fn runtime() -> Result<&'static tokio::runtime::Runtime> {
    static TOKIO_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    if let Some(rt) = TOKIO_RUNTIME.get() {
        return Ok(rt);
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("start Rig runtime")?;
    Ok(TOKIO_RUNTIME.get_or_init(|| rt))
}

type HttpClientCache = Mutex<HashMap<(bool, Duration, Duration), reqwest::Client>>;

pub(crate) fn build_http_client(url: &url::Url, config: &LlmConfig) -> Result<reqwest::Client> {
    static HTTP_CLIENTS: OnceLock<HttpClientCache> = OnceLock::new();
    let key = (
        url.scheme() == "https",
        config.connect_timeout(),
        config.request_timeout(),
    );
    let cache = HTTP_CLIENTS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock()
        && let Some(client) = guard.get(&key)
    {
        return Ok(client.clone());
    }
    let client = reqwest::Client::builder()
        .https_only(key.0)
        .connect_timeout(key.1)
        .timeout(key.2)
        .build()
        .context("build secure HTTP client")?;
    if let Ok(mut guard) = cache.lock() {
        guard.insert(key, client.clone());
    }
    Ok(client)
}

pub(crate) fn to_rig_history(history: &[ChatTurn]) -> Vec<rig::completion::Message> {
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
    let http_client = build_http_client(&url, config)?;
    let client = openai::Client::builder()
        .api_key(config.bearer_token.clone())
        .base_url(base_url.as_str())
        .http_client(http_client)
        .build()
        .context("build Rig OpenAI-compatible client")?
        .completions_api();

    let (preamble, fitted_context, mut rig_messages, max_output) =
        budget::build_rag_request_budgets(config, question, context, history);

    let mut builder = client
        .agent(&config.model)
        .preamble(preamble)
        .context(&fitted_context);
    if let Some(cap) = max_output {
        builder = builder.max_tokens(cap);
    }
    let agent = builder.build();

    debug_log(format_args!(
        "LLM request: POST {endpoint_for_log}; model={:?}; auth={}; history_turns={}; question_bytes={}; context_bytes={}; max_tokens={:?}",
        config.model,
        if config.bearer_token.is_empty() {
            "none"
        } else {
            "bearer (hidden)"
        },
        rig_messages.len(),
        question.len(),
        fitted_context.len(),
        max_output
    ));

    let runtime = runtime()?;
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
    let http_client = build_http_client(&url, config)?;
    let client = openai::Client::builder()
        .api_key(config.bearer_token.clone())
        .base_url(base_url.as_str())
        .http_client(http_client)
        .build()
        .context("build Rig OpenAI-compatible client")?
        .completions_api();

    let (preamble, prompt, max_output) =
        budget::build_judge_prompt_and_max_tokens(config, email, definitions);

    let mut builder = client.agent(&config.model).preamble(preamble);
    if let Some(cap) = max_output {
        builder = builder.max_tokens(cap);
    }
    let agent = builder.build();

    let runtime = runtime()?;
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
    let http_client = build_http_client(&url, config)?;
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
    let runtime = runtime()?;
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
