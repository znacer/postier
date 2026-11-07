//! Tool-calling research agent over the local BM25 index.
//!
//! The model gets four tools (`search_emails`, `read_email`, `get_thread`, `email_stats`)
//! and loops — search, read, refine — until it can answer with `[#id]` citations.

use std::{
    collections::{HashMap, HashSet},
    future::IntoFuture,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result};
use rig::{
    completion::{Prompt, PromptError},
    prelude::*,
    providers::openai,
    tool::PortableTool,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{
    ChatTurn, LlmConfig, agent_system_prompt, build_context, clean_email_body, debug_log,
    endpoint_for_log, openai_base_url, to_rig_history, validate_endpoint,
};
use crate::{
    db::{
        Index,
        query::{self, ParsedQuery, SearchFilters, StatsGroup},
    },
    i18n::Language,
    model::SearchHit,
    text,
};

/// Total model calls per question (each tool round costs one call, plus the answer).
pub const MAX_MODEL_CALLS: usize = 12;
const MAX_SEARCH_RESULTS: usize = 50;
const READ_EMAIL_MAX_CHARS: usize = 12_000;
const THREAD_MAX_MESSAGES: usize = 40;
const THREAD_TOTAL_CHARS: usize = 14_000;
const SNIPPET_CHARS: usize = 360;

pub type ProgressFn = Arc<dyn Fn(String) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct AgentAnswer {
    /// Markdown answer with citations turned into `mail://` links.
    pub answer: String,
    /// Emails cited first, then other emails the agent read.
    pub evidence: Vec<SearchHit>,
    /// Human-readable log of the tool calls.
    pub trace: String,
}

#[derive(Default)]
struct Evidence {
    order: Vec<i64>,
    hits: HashMap<i64, SearchHit>,
    read: HashSet<i64>,
}

impl Evidence {
    fn add(&mut self, hit: SearchHit, read: bool) {
        let id = hit.id;
        if !self.hits.contains_key(&id) {
            self.order.push(id);
            self.hits.insert(id, hit);
        }
        if read {
            self.read.insert(id);
        }
    }

    fn known_ids(&self) -> HashSet<i64> {
        self.hits.keys().copied().collect()
    }

    /// Cited emails first (citation order), then emails read in full.
    fn ordered(&self, cited: &[i64]) -> Vec<SearchHit> {
        let mut out = Vec::new();
        let mut used = HashSet::new();
        for id in cited {
            if let Some(hit) = self.hits.get(id)
                && used.insert(*id)
            {
                out.push(hit.clone());
            }
        }
        for id in &self.order {
            if self.read.contains(id)
                && used.insert(*id)
                && let Some(hit) = self.hits.get(id)
            {
                out.push(hit.clone());
            }
        }
        if out.is_empty() {
            out = self
                .order
                .iter()
                .take(8)
                .filter_map(|id| self.hits.get(id).cloned())
                .collect();
        }
        out
    }

    /// Emails to fall back on when the agent runs out of turns: read ones first.
    fn for_context(&self) -> Vec<SearchHit> {
        let mut out: Vec<SearchHit> = self
            .order
            .iter()
            .filter(|id| self.read.contains(id))
            .filter_map(|id| self.hits.get(id).cloned())
            .collect();
        out.extend(
            self.order
                .iter()
                .filter(|id| !self.read.contains(id))
                .filter_map(|id| self.hits.get(id).cloned()),
        );
        out
    }
}

struct Shared {
    db: Mutex<Index>,
    language: Language,
    default_limit: usize,
    evidence: Mutex<Evidence>,
    trace: Mutex<Vec<String>>,
    progress: ProgressFn,
}

impl Shared {
    fn db(&self) -> std::sync::MutexGuard<'_, Index> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn record(&self, entry: String) {
        debug_log(format_args!("agent tool: {entry}"));
        let mut trace = self.trace.lock().unwrap_or_else(|e| e.into_inner());
        let n = trace.len() + 1;
        trace.push(format!("{n}. {entry}"));
    }

    fn remember(&self, hits: impl IntoIterator<Item = SearchHit>, read: bool) {
        let mut evidence = self.evidence.lock().unwrap_or_else(|e| e.into_inner());
        for hit in hits {
            evidence.add(hit, read);
        }
    }
}

#[derive(Debug)]
pub struct ToolFailure(String);

impl std::fmt::Display for ToolFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ToolFailure {}

impl From<anyhow::Error> for ToolFailure {
    fn from(error: anyhow::Error) -> Self {
        Self(format!("{error:#}"))
    }
}

fn filter_properties() -> Map<String, Value> {
    let mut props = Map::new();
    props.insert(
        "from".into(),
        json!({"type": "string", "description": "Sender name or address (substring match), e.g. \"alice\" or \"@acme.com\"."}),
    );
    props.insert(
        "to".into(),
        json!({"type": "string", "description": "Recipient (To/Cc) name or address (substring match)."}),
    );
    props.insert(
        "date_from".into(),
        json!({"type": "string", "description": "Inclusive start date: YYYY, YYYY-MM or YYYY-MM-DD."}),
    );
    props.insert(
        "date_to".into(),
        json!({"type": "string", "description": "Inclusive end date: YYYY, YYYY-MM or YYYY-MM-DD."}),
    );
    props.insert(
        "folder".into(),
        json!({"type": "string", "description": "Folder path substring, e.g. \"Sent\" or \"Projects/Atlas\"."}),
    );
    props.insert(
        "label".into(),
        json!({"type": "string", "description": "Exact label name, e.g. \"Urgent\" or \"Decision\"."}),
    );
    props.insert(
        "has_attachment".into(),
        json!({"type": "boolean", "description": "Only emails with (true) or without (false) attachments."}),
    );
    props
}

fn describe_query(parsed: &ParsedQuery) -> String {
    let mut parts = Vec::new();
    if parsed.has_text() {
        let mut words: Vec<String> = parsed.phrases.iter().map(|p| format!("\"{p}\"")).collect();
        words.extend(parsed.terms.iter().cloned());
        parts.push(format!("\"{}\"", words.join(" ")));
    }
    let f = &parsed.filters;
    let mut push = |key: &str, value: &Option<String>| {
        if let Some(v) = value.as_deref().filter(|v| !v.trim().is_empty()) {
            parts.push(format!("{key}:{v}"));
        }
    };
    push("from", &f.from);
    push("to", &f.to);
    push("date_from", &f.date_from);
    push("date_to", &f.date_to);
    push("folder", &f.folder);
    push("label", &f.label);
    if let Some(has) = f.has_attachment {
        parts.push(format!("has_attachment:{has}"));
    }
    if parts.is_empty() {
        "(no criteria)".into()
    } else {
        parts.join(" ")
    }
}

fn hit_header(hit: &SearchHit) -> Map<String, Value> {
    let mut obj = Map::new();
    obj.insert("id".into(), json!(hit.id));
    obj.insert(
        "date".into(),
        json!(hit.sent_at.as_deref().unwrap_or("unknown")),
    );
    obj.insert("from".into(), json!(hit.sender));
    obj.insert(
        "to".into(),
        json!(text::truncate_chars(&hit.recipients, 200)),
    );
    obj.insert("subject".into(), json!(hit.subject));
    obj.insert("folder".into(), json!(hit.folder));
    if !hit.conversation_id.trim().is_empty() {
        obj.insert("thread_id".into(), json!(hit.conversation_id.trim()));
    }
    if !hit.attachment_names.trim().is_empty() {
        obj.insert("attachments".into(), json!(hit.attachment_names.trim()));
    }
    if !hit.labels.is_empty() {
        let labels: Vec<&str> = hit.labels.iter().map(|l| l.label.as_str()).collect();
        obj.insert("labels".into(), json!(labels));
    }
    obj
}

fn indexed_text(hit: &SearchHit) -> String {
    if hit.body_text.trim().is_empty() {
        text::index_body(&hit.subject, &hit.body)
    } else {
        hit.body_text.clone()
    }
}

fn parse_id(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s
            .trim()
            .trim_start_matches(|c: char| !c.is_ascii_digit())
            .parse()
            .ok(),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// search_emails

#[derive(Deserialize)]
pub struct SearchArgs {
    #[serde(default)]
    query: Option<String>,
    #[serde(flatten)]
    filters: SearchFilters,
    #[serde(default)]
    limit: Option<Value>,
}

pub struct SearchEmailsTool(Arc<Shared>);

impl SearchEmailsTool {
    fn run(&self, args: SearchArgs) -> Result<Value, ToolFailure> {
        let shared = &self.0;
        let mut parsed = query::parse_query(args.query.as_deref().unwrap_or(""));
        parsed.filters = args.filters.or(parsed.filters);
        if !parsed.has_text() && parsed.filters.is_empty() {
            return Ok(json!({"error": "Provide keywords in `query` and/or at least one filter."}));
        }
        let limit = args
            .limit
            .as_ref()
            .and_then(parse_id)
            .map(|n| n.max(1) as usize)
            .unwrap_or(shared.default_limit)
            .min(MAX_SEARCH_RESULTS);
        let described = describe_query(&parsed);
        (shared.progress)(shared.language.agent_progress_search(&described));

        let hits = shared.db().search_emails(&parsed, limit)?;
        let tokens = parsed.tokens();
        let results: Vec<Value> = hits
            .iter()
            .map(|hit| {
                let mut obj = hit_header(hit);
                if hit.thread_matches > 1 {
                    obj.insert("thread_matches".into(), json!(hit.thread_matches));
                }
                obj.insert(
                    "snippet".into(),
                    json!(text::best_passages(
                        &indexed_text(hit),
                        &tokens,
                        SNIPPET_CHARS
                    )),
                );
                Value::Object(obj)
            })
            .collect();

        shared.record(format!(
            "search_emails {described} → {} result(s)",
            hits.len()
        ));
        let count = hits.len();
        shared.remember(hits, false);

        let mut out = json!({
            "search": described,
            "result_count": count,
            "results": results,
        });
        let invalid = query::invalid_dates(&parsed.filters);
        if !invalid.is_empty() {
            out["warning"] = json!(format!(
                "Unrecognized date value(s) ignored: {}. Use YYYY, YYYY-MM or YYYY-MM-DD.",
                invalid.join(", ")
            ));
        }
        if count == 0 {
            out["hint"] = json!(
                "No match. Try fewer or different keywords, synonyms, the other language (French/English), or relax the filters."
            );
        }
        Ok(out)
    }
}

impl PortableTool for SearchEmailsTool {
    const NAME: &'static str = "search_emails";
    type Args = SearchArgs;
    type Output = Value;
    type Error = ToolFailure;

    fn description(&self) -> String {
        "Keyword (BM25) search over the email archive, with optional filters. Matching is \
         lexical with stemming (English and French): use 2-5 distinctive keywords rather than \
         sentences, and use the filters for people, dates, folders, labels and attachments. \
         Use \"double quotes\" in the query for exact phrases. Returns one result per thread \
         (best match), each with an id, metadata, thread_id and a snippet of the matching \
         passages. Snippets are partial: use read_email or get_thread before relying on an \
         email."
            .into()
    }

    fn parameters(&self) -> Value {
        let mut props = filter_properties();
        props.insert(
            "query".into(),
            json!({"type": "string", "description": "Keywords to search for. May be empty when filters are given (returns the most recent matching emails)."}),
        );
        props.insert(
            "limit".into(),
            json!({"type": "integer", "description": format!("Maximum number of threads to return (default {}, max {MAX_SEARCH_RESULTS}).", self.0.default_limit)}),
        );
        json!({"type": "object", "properties": props})
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.run(args)
    }
}

// ---------------------------------------------------------------------------
// read_email

#[derive(Deserialize)]
pub struct ReadArgs {
    id: Value,
}

pub struct ReadEmailTool(Arc<Shared>);

impl ReadEmailTool {
    fn run(&self, args: ReadArgs) -> Result<Value, ToolFailure> {
        let shared = &self.0;
        let Some(id) = parse_id(&args.id) else {
            return Ok(json!({"error": "`id` must be a numeric email id."}));
        };
        (shared.progress)(shared.language.agent_progress_read(id));
        let Some(hit) = shared.db().get_hit(id)? else {
            shared.record(format!("read_email #{id} → not found"));
            return Ok(json!({"error": format!("No email with id {id}.")}));
        };
        let mut obj = hit_header(&hit);
        obj.insert(
            "body".into(),
            json!(text::truncate_chars(
                &clean_email_body(&hit.body),
                READ_EMAIL_MAX_CHARS
            )),
        );
        shared.record(format!("read_email #{id} → \"{}\"", hit.subject));
        shared.remember([hit], true);
        Ok(Value::Object(obj))
    }
}

impl PortableTool for ReadEmailTool {
    const NAME: &'static str = "read_email";
    type Args = ReadArgs;
    type Output = Value;
    type Error = ToolFailure;

    fn description(&self) -> String {
        "Full content and metadata of one email, by id (as returned by search_emails or cited \
         as [#id] in earlier answers)."
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"id": {"type": "integer", "description": "Email id."}},
            "required": ["id"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.run(args)
    }
}

// ---------------------------------------------------------------------------
// get_thread

#[derive(Deserialize)]
pub struct ThreadArgs {
    #[serde(default)]
    thread_id: Option<String>,
    #[serde(default)]
    email_id: Option<Value>,
}

pub struct GetThreadTool(Arc<Shared>);

impl GetThreadTool {
    fn run(&self, args: ThreadArgs) -> Result<Value, ToolFailure> {
        let shared = &self.0;
        let db = shared.db();
        let mut anchor: Option<SearchHit> = None;
        let mut thread_id = args.thread_id.unwrap_or_default().trim().to_string();
        if thread_id.is_empty()
            && let Some(id) = args.email_id.as_ref().and_then(parse_id)
            && let Some(hit) = db.get_hit(id)?
        {
            thread_id = hit.conversation_id.trim().to_string();
            anchor = Some(hit);
        }
        let (total, messages) = if thread_id.is_empty() {
            match anchor {
                Some(hit) => (1, vec![hit]),
                None => {
                    return Ok(json!({"error": "Provide a `thread_id` or an `email_id`."}));
                }
            }
        } else {
            db.get_thread(&thread_id, THREAD_MAX_MESSAGES)?
        };
        drop(db);
        (shared.progress)(shared.language.agent_progress_thread(total));
        if messages.is_empty() {
            shared.record(format!("get_thread {thread_id} → not found"));
            return Ok(json!({"error": format!("No thread with id {thread_id}.")}));
        }
        let per_message = (THREAD_TOTAL_CHARS / messages.len()).max(400);
        let items: Vec<Value> = messages
            .iter()
            .map(|hit| {
                let mut obj = hit_header(hit);
                obj.remove("thread_id");
                obj.insert(
                    "body".into(),
                    json!(text::truncate_chars(&indexed_text(hit), per_message)),
                );
                Value::Object(obj)
            })
            .collect();
        let mut out = json!({
            "thread_id": thread_id,
            "total_messages": total,
            "messages": items,
        });
        if total > messages.len() {
            out["note"] = json!(format!(
                "Long thread: showing the first message and the {} most recent. Quoted replies are removed from bodies; use read_email for a full message.",
                messages.len() - 1
            ));
        }
        shared.record(format!(
            "get_thread {} → {} message(s)",
            if thread_id.is_empty() {
                "(single email)"
            } else {
                &thread_id
            },
            total
        ));
        shared.remember(messages, true);
        Ok(out)
    }
}

impl PortableTool for GetThreadTool {
    const NAME: &'static str = "get_thread";
    type Args = ThreadArgs;
    type Output = Value;
    type Error = ToolFailure;

    fn description(&self) -> String {
        "All messages of a conversation in chronological order (quoted replies removed). Pass \
         the thread_id from search results, or an email_id to open that email's thread. Use it \
         for discussions, decisions and status questions: the latest messages usually hold the \
         outcome."
            .into()
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "thread_id": {"type": "string", "description": "Thread id from search results."},
                "email_id": {"type": "integer", "description": "Any email id of the thread (alternative to thread_id)."}
            }
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.run(args)
    }
}

// ---------------------------------------------------------------------------
// email_stats

#[derive(Deserialize)]
pub struct StatsArgs {
    #[serde(default)]
    group_by: Option<StatsGroup>,
    #[serde(default)]
    query: Option<String>,
    #[serde(flatten)]
    filters: SearchFilters,
    #[serde(default)]
    top: Option<Value>,
}

pub struct EmailStatsTool(Arc<Shared>);

impl EmailStatsTool {
    fn run(&self, args: StatsArgs) -> Result<Value, ToolFailure> {
        let shared = &self.0;
        let mut parsed = query::parse_query(args.query.as_deref().unwrap_or(""));
        parsed.filters = args.filters.or(parsed.filters);
        let top = args
            .top
            .as_ref()
            .and_then(parse_id)
            .map(|n| n.clamp(1, 100) as usize)
            .unwrap_or(20);
        let described = describe_query(&parsed);
        let group_name = args
            .group_by
            .and_then(|g| {
                serde_json::to_value(g)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
            })
            .unwrap_or_else(|| "none".into());
        (shared.progress)(shared.language.agent_progress_stats(&group_name));
        let (total, buckets) = shared.db().email_stats(&parsed, args.group_by, top)?;
        shared.record(format!(
            "email_stats {described} group_by:{group_name} → {total} email(s)"
        ));
        let mut out = json!({
            "criteria": described,
            "total_matching_emails": total,
        });
        if args.group_by.is_some() {
            out["group_by"] = json!(group_name);
            out["groups"] = json!(buckets);
        }
        Ok(out)
    }
}

impl PortableTool for EmailStatsTool {
    const NAME: &'static str = "email_stats";
    type Args = StatsArgs;
    type Output = Value;
    type Error = ToolFailure;

    fn description(&self) -> String {
        "Counts emails matching keywords and/or filters, optionally grouped by sender, month, \
         year, folder or label (top groups by count; months/years in chronological order). \
         Use it for how-many / who-most / when / trend questions instead of counting search \
         results. Keyword matching here requires all keywords."
            .into()
    }

    fn parameters(&self) -> Value {
        let mut props = filter_properties();
        props.insert(
            "group_by".into(),
            json!({"type": "string", "enum": ["sender", "month", "year", "folder", "label"], "description": "Optional grouping."}),
        );
        props.insert(
            "query".into(),
            json!({"type": "string", "description": "Optional keywords (all must match)."}),
        );
        props.insert(
            "top".into(),
            json!({"type": "integer", "description": "Maximum number of groups (default 20, max 100)."}),
        );
        json!({"type": "object", "properties": props})
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.run(args)
    }
}

// ---------------------------------------------------------------------------
// citations

/// Turns `[#123]` / `[#123, #456]` / `[Email #123]` citations of known emails into
/// `[#123](mail://123)` links. Returns the rewritten text and cited ids in order.
pub fn linkify_citations(answer: &str, known: &HashSet<i64>) -> (String, Vec<i64>) {
    let mut out = String::with_capacity(answer.len() + 64);
    let mut cited: Vec<i64> = Vec::new();
    let mut rest = answer;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after
            .find(']')
            .filter(|&c| c <= 80 && !after[..c].contains('\n'));
        let Some(close) = close else {
            out.push('[');
            rest = after;
            continue;
        };
        let inner = &after[..close];
        let following = &after[close + 1..];
        let ids = if following.starts_with('(') {
            None
        } else {
            parse_citation_list(inner)
        };
        match ids {
            Some(ids) => {
                let rendered: Vec<String> = ids
                    .iter()
                    .map(|id| {
                        if known.contains(id) {
                            if !cited.contains(id) {
                                cited.push(*id);
                            }
                            format!("[#{id}](mail://{id})")
                        } else {
                            format!("#{id}")
                        }
                    })
                    .collect();
                out.push_str(&rendered.join(", "));
            }
            None => {
                out.push('[');
                out.push_str(inner);
                out.push(']');
            }
        }
        rest = following;
    }
    out.push_str(rest);
    (out, cited)
}

fn parse_citation_list(inner: &str) -> Option<Vec<i64>> {
    let mut ids = Vec::new();
    for part in inner.split([',', ';']) {
        let part = part.trim();
        let part = ["Email", "email", "Courriel", "courriel", "Mail", "mail"]
            .iter()
            .find_map(|p| part.strip_prefix(p))
            .unwrap_or(part)
            .trim_start();
        let digits = part.strip_prefix('#')?;
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        ids.push(digits.parse().ok()?);
    }
    (!ids.is_empty()).then_some(ids)
}

// ---------------------------------------------------------------------------
// run loop

async fn wait_for_cancel(flag: &AtomicBool) {
    while !flag.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

pub fn run_agent(
    config: &LlmConfig,
    db_path: &Path,
    question: &str,
    history: &[ChatTurn],
    progress: ProgressFn,
    cancel: Arc<AtomicBool>,
) -> Result<AgentAnswer> {
    let language = config.language;
    let shared = Arc::new(Shared {
        db: Mutex::new(Index::open(db_path)?),
        language,
        default_limit: config.context_email_limit(),
        evidence: Mutex::new(Evidence::default()),
        trace: Mutex::new(Vec::new()),
        progress: progress.clone(),
    });

    let url = validate_endpoint(&config.endpoint)?;
    let endpoint_for_log = endpoint_for_log(&url);
    let base_url = openai_base_url(&url)?;
    let http_client = reqwest::Client::builder()
        .https_only(url.scheme() == "https")
        .build()
        .context("build secure HTTP client")?;
    let client = openai::Client::builder()
        .api_key(config.bearer_token.clone())
        .base_url(base_url.as_str())
        .http_client(http_client)
        .build()
        .context("build Rig OpenAI-compatible client")?
        .completions_api();

    let preamble = format!(
        "{}\n\n{} {}.",
        agent_system_prompt(language),
        language.agent_today_label(),
        chrono::Local::now().format("%Y-%m-%d (%A)")
    );
    let agent = client
        .agent(&config.model)
        .preamble(&preamble)
        .tool(SearchEmailsTool(shared.clone()))
        .tool(ReadEmailTool(shared.clone()))
        .tool(GetThreadTool(shared.clone()))
        .tool(EmailStatsTool(shared.clone()))
        .build();

    debug_log(format_args!(
        "Agent request: POST {endpoint_for_log}; model={:?}; history_turns={}; question_bytes={}",
        config.model,
        history.len(),
        question.len()
    ));
    progress(language.agent_progress_thinking().to_string());

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("start Rig runtime")?;
    let outcome = runtime.block_on(async {
        let request = agent
            .prompt(question)
            .history(to_rig_history(history))
            .max_turns(MAX_MODEL_CALLS);
        tokio::select! {
            result = request.into_future() => Some(result),
            _ = wait_for_cancel(&cancel) => None,
        }
    });
    drop(runtime);

    let raw_answer = match outcome {
        None => anyhow::bail!("{}", language.agent_cancelled()),
        Some(Ok(answer)) => answer,
        Some(Err(PromptError::MaxTurnsError { .. })) => {
            progress(language.agent_progress_finalizing().to_string());
            let hits = shared
                .evidence
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .for_context();
            shared.record("research budget exhausted → answering with collected evidence".into());
            let context = build_context(&hits, config.max_context_chars);
            let prompt = format!("{question}\n\n{}", language.agent_budget_exhausted_note());
            super::ask_with_history(config, &prompt, &context, history)?
        }
        Some(Err(error)) => {
            debug_log(format_args!("Rig agent error: {error:#}"));
            return Err(
                anyhow::anyhow!(error).context(format!("Rig agent request to {endpoint_for_log}"))
            );
        }
    };

    let evidence = shared.evidence.lock().unwrap_or_else(|e| e.into_inner());
    let (answer, cited) = linkify_citations(&raw_answer, &evidence.known_ids());
    let ordered = evidence.ordered(&cited);
    drop(evidence);
    let trace_lines = shared
        .trace
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let trace = if trace_lines.is_empty() {
        language.agent_no_tool_calls().to_string()
    } else {
        format!(
            "{}\n{}",
            language.agent_trace_heading(),
            trace_lines.join("\n")
        )
    };
    Ok(AgentAnswer {
        answer,
        evidence: ordered,
        trace,
    })
}
