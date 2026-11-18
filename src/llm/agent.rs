//! Tool-calling research agent over the local BM25 index.
//!
//! The model gets five tools (`search_emails`, `read_email`, `read_attachment`,
//! `get_thread`, `email_stats`) and loops — search, read, refine — until it can
//! answer with `[#id]` citations.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result};
use rig::{
    agent::{
        AgentHook, CompletionCallAction, CompletionCallEvent, HookContext, RequestPatch,
        ToolResultAction, ToolResultEvent,
    },
    completion::PromptError,
    prelude::*,
    providers::openai,
    tool::PortableTool,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{
    ChatTurn, LlmConfig, PromptDensity, agent_system_prompt_for_density,
    budget::{
        cap_output_tokens, estimate_message_tokens, estimate_tokens, estimate_tool_def_tokens,
        trim_rig_history_for_completion, truncate_to_token_budget, water_fill_quotas,
    },
    build_context, build_http_client, clean_email_body, debug_log, endpoint_for_log,
    openai_base_url, to_rig_history, validate_endpoint,
};
use crate::{
    attachments::combine_attachment_text,
    db::{
        Index,
        query::{self, ParsedQuery, SearchFilters, StatsGroup},
    },
    i18n::Language,
    model::SearchHit,
    text,
};

/// Default total model calls per question (each tool round costs one call, plus the answer).
pub const MAX_MODEL_CALLS: usize = 12;

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
    prompt_density: PromptDensity,
    default_limit: usize,
    max_search_results: usize,
    snippet_chars: usize,
    read_email_max_chars: usize,
    thread_max_messages: usize,
    thread_total_chars: usize,
    stats_max_groups: usize,
    hit_recipients_max_chars: usize,
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

struct TokenBudgetHook {
    window_tokens: Option<usize>,
    input_budget_tokens: usize,
    history_budget_tokens: usize,
    overhead_tokens: usize,
    single_tool_result_max_tokens: usize,
}

impl AgentHook for TokenBudgetHook {
    async fn on_completion_call(
        &self,
        _ctx: &HookContext,
        event: CompletionCallEvent<'_>,
    ) -> CompletionCallAction {
        let Some(window) = self.window_tokens else {
            return CompletionCallAction::continue_run();
        };

        let (patched_history, history_changed) = trim_rig_history_for_completion(
            event.history,
            event.prompt,
            self.input_budget_tokens,
            self.overhead_tokens,
            self.history_budget_tokens,
        );

        let effective_history = if history_changed {
            patched_history.as_slice()
        } else {
            event.history
        };
        let prompt_tokens = estimate_message_tokens(event.prompt);
        let history_tokens: usize = effective_history.iter().map(estimate_message_tokens).sum();
        let total_input = self.overhead_tokens + prompt_tokens + history_tokens;
        let max_output = cap_output_tokens(window, total_input, 4096, 64);

        let mut patch = RequestPatch::new().max_tokens(max_output as u64);
        if history_changed {
            debug_log(format_args!(
                "compacted agent history on turn {} ({} -> {} messages; est_input={total_input}/{window}, max_tokens={max_output})",
                event.turn,
                event.history.len(),
                patched_history.len()
            ));
            patch = patch.history(patched_history);
        }
        CompletionCallAction::patch(patch)
    }

    async fn on_tool_result(
        &self,
        _ctx: &HookContext,
        event: ToolResultEvent<'_>,
    ) -> ToolResultAction {
        if self.window_tokens.is_none() {
            return ToolResultAction::keep();
        }
        let rendered = event.presentation.render();
        if estimate_tokens(&rendered) <= self.single_tool_result_max_tokens {
            return ToolResultAction::keep();
        }
        let truncated = truncate_to_token_budget(
            &rendered,
            self.single_tool_result_max_tokens,
            "\n... [tool output truncated to fit token limit]",
        );
        ToolResultAction::rewrite(truncated)
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

fn filter_properties(density: PromptDensity) -> Map<String, Value> {
    let mut props = Map::new();
    match density {
        PromptDensity::Full => {
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
                json!({"type": "string", "description": "Label name or substring, e.g. \"Urgent\" or \"Decision\"."}),
            );
            props.insert(
                "has_attachment".into(),
                json!({"type": "boolean", "description": "Only emails with (true) or without (false) attachments."}),
            );
        }
        PromptDensity::Compact => {
            props.insert(
                "from".into(),
                json!({"type": "string", "description": "Sender substring."}),
            );
            props.insert(
                "to".into(),
                json!({"type": "string", "description": "Recipient substring."}),
            );
            props.insert(
                "date_from".into(),
                json!({"type": "string", "description": "Start YYYY(-MM-DD)."}),
            );
            props.insert(
                "date_to".into(),
                json!({"type": "string", "description": "End YYYY(-MM-DD)."}),
            );
            props.insert(
                "folder".into(),
                json!({"type": "string", "description": "Folder substring."}),
            );
            props.insert(
                "label".into(),
                json!({"type": "string", "description": "Label substring."}),
            );
            props.insert(
                "has_attachment".into(),
                json!({"type": "boolean", "description": "Has attachments."}),
            );
        }
        PromptDensity::UltraCompact => {
            props.insert("from".into(), json!({"type": "string"}));
            props.insert("to".into(), json!({"type": "string"}));
            props.insert(
                "date_from".into(),
                json!({"type": "string", "description": "YYYY-MM-DD"}),
            );
            props.insert(
                "date_to".into(),
                json!({"type": "string", "description": "YYYY-MM-DD"}),
            );
            props.insert("folder".into(), json!({"type": "string"}));
            props.insert("label".into(), json!({"type": "string"}));
            props.insert("has_attachment".into(), json!({"type": "boolean"}));
        }
    }
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

fn hit_header(hit: &SearchHit, shared: &Shared) -> Map<String, Value> {
    let mut obj = Map::new();
    obj.insert("id".into(), json!(hit.id));
    obj.insert(
        "date".into(),
        json!(hit.sent_at.as_deref().unwrap_or("unknown")),
    );
    let subject = match shared.prompt_density {
        PromptDensity::Full => hit.subject.clone(),
        PromptDensity::Compact | PromptDensity::UltraCompact => {
            text::truncate_chars(&hit.subject, 160)
        }
    };
    obj.insert("from".into(), json!(hit.sender));
    obj.insert(
        "to".into(),
        json!(text::truncate_chars(
            &hit.recipients,
            shared.hit_recipients_max_chars
        )),
    );
    obj.insert("subject".into(), json!(subject));
    obj.insert("folder".into(), json!(hit.folder));
    if !hit.conversation_id.trim().is_empty() {
        obj.insert("thread_id".into(), json!(hit.conversation_id.trim()));
    }
    if !hit.attachment_names.trim().is_empty() {
        let attachments = match shared.prompt_density {
            PromptDensity::Full => hit.attachment_names.trim().to_string(),
            PromptDensity::Compact | PromptDensity::UltraCompact => {
                text::truncate_chars(hit.attachment_names.trim(), 140)
            }
        };
        obj.insert("attachments".into(), json!(attachments));
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

/// Formats body + attachments into `(body_value, optional_attachments_array)` using
/// water-filling across their char lengths so small attachments are returned intact.
fn format_body_and_attachments(
    body_text: &str,
    attachments: &[crate::model::AttachmentText],
    total_char_budget: usize,
    density: PromptDensity,
) -> (String, Option<Vec<Value>>, bool) {
    let body_chars = body_text.chars().count();
    let non_empty: Vec<&crate::model::AttachmentText> = attachments
        .iter()
        .filter(|a| !a.text.trim().is_empty())
        .collect();
    if non_empty.is_empty() {
        return (
            text::truncate_chars(body_text, total_char_budget),
            None,
            false,
        );
    }

    // Reserve minimal JSON overhead per attachment item so we fit cleanly inside budget.
    let meta_reserve = non_empty.len().saturating_mul(48);
    let content_budget = total_char_budget
        .saturating_sub(meta_reserve)
        .max(total_char_budget.min(120));
    let mut lengths = Vec::with_capacity(1 + non_empty.len());
    lengths.push(body_chars);
    let att_char_counts: Vec<usize> = non_empty.iter().map(|a| a.text.chars().count()).collect();
    lengths.extend(att_char_counts.iter().copied());

    let quotas = water_fill_quotas(&lengths, content_budget);
    let body_out = text::truncate_chars(body_text, quotas[0]);
    let mut any_truncated = false;
    let mut items = Vec::with_capacity(non_empty.len());
    for (idx, att) in non_empty.iter().enumerate() {
        let total = att_char_counts[idx];
        let quota = quotas[idx + 1];
        let truncated = total > quota;
        if truncated {
            any_truncated = true;
        }
        let content = text::truncate_chars(&att.text, quota);
        let mut entry = Map::new();
        entry.insert("filename".into(), json!(att.filename));
        if density == PromptDensity::Full && !att.mime_type.is_empty() {
            entry.insert("mime_type".into(), json!(att.mime_type));
        }
        if att.size_bytes > 0 && density != PromptDensity::UltraCompact {
            entry.insert("size_bytes".into(), json!(att.size_bytes));
        }
        entry.insert("content".into(), json!(content));
        if truncated {
            entry.insert("truncated".into(), json!(true));
            entry.insert("total_chars".into(), json!(total));
        }
        items.push(Value::Object(entry));
    }
    (body_out, Some(items), any_truncated)
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
            .min(shared.max_search_results);
        let described = describe_query(&parsed);
        (shared.progress)(shared.language.agent_progress_search(&described));

        let hits = shared.db().search_emails(&parsed, limit)?;
        let tokens = parsed.tokens();
        let results: Vec<Value> = hits
            .iter()
            .map(|hit| {
                let mut obj = hit_header(hit, shared);
                if hit.thread_matches > 1 {
                    obj.insert("thread_matches".into(), json!(hit.thread_matches));
                    if shared.prompt_density == PromptDensity::Full {
                        obj.insert(
                            "thread_hint".into(),
                            json!("Multiple matching messages in this thread; call get_thread to read the full conversation."),
                        );
                    }
                }
                let body_indexed = indexed_text(hit);
                obj.insert(
                    "snippet".into(),
                    json!(text::best_passages(
                        &body_indexed,
                        &tokens,
                        shared.snippet_chars
                    )),
                );
                let combined_att = combine_attachment_text(&hit.attachments);
                if !combined_att.is_empty() {
                    let att_budget = if body_indexed.chars().count() < 120 {
                        shared.snippet_chars
                    } else {
                        (shared.snippet_chars * 3 / 4)
                            .max(80)
                            .min(shared.snippet_chars)
                    };
                    obj.insert(
                        "attachment_snippet".into(),
                        json!(text::best_passages(&combined_att, &tokens, att_budget)),
                    );
                }
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
        match self.0.prompt_density {
            PromptDensity::Full => {
                "Keyword (BM25) search over the email archive including common attachments (Word, \
                 PowerPoint, LibreOffice, text files), with optional filters. Matching is lexical \
                 with stemming (English and French): use 2-5 distinctive keywords rather than \
                 sentences, and use the filters for people, dates, folders, labels and \
                 attachments. Use \"double quotes\" in the query for exact phrases. Returns one \
                 result per thread (best match), each with an id, metadata, thread_id and snippets. \
                 Snippets are partial: use read_email or get_thread before relying on an email."
                    .into()
            }
            PromptDensity::Compact => {
                "BM25 keyword search over emails & attachments (EN/FR stemming) with filters. Use 2-5 \
                 keywords; returns 1 hit per thread with id, thread_id and snippets."
                    .into()
            }
            PromptDensity::UltraCompact => {
                "Search emails & attachments by keywords/filters. Returns id, thread_id, snippets."
                    .into()
            }
        }
    }

    fn parameters(&self) -> Value {
        let mut props = filter_properties(self.0.prompt_density);
        match self.0.prompt_density {
            PromptDensity::Full => {
                props.insert(
                    "query".into(),
                    json!({"type": "string", "description": "Keywords to search for. May be empty when filters are given (returns the most recent matching emails)."}),
                );
                props.insert(
                    "limit".into(),
                    json!({"type": "integer", "description": format!("Maximum number of threads to return (default {}, max {}).", self.0.default_limit, self.0.max_search_results)}),
                );
            }
            PromptDensity::Compact => {
                props.insert(
                    "query".into(),
                    json!({"type": "string", "description": "Keywords (optional if filters given)."}),
                );
                props.insert(
                    "limit".into(),
                    json!({"type": "integer", "description": format!("Max threads (default {}, max {}).", self.0.default_limit, self.0.max_search_results)}),
                );
            }
            PromptDensity::UltraCompact => {
                props.insert("query".into(), json!({"type": "string"}));
                props.insert(
                    "limit".into(),
                    json!({"type": "integer", "description": format!("Max {}", self.0.max_search_results)}),
                );
            }
        }
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
        let mut obj = hit_header(&hit, shared);
        let cleaned = clean_email_body(&hit.body);
        let (body_out, att_items, any_truncated) = format_body_and_attachments(
            &cleaned,
            &hit.attachments,
            shared.read_email_max_chars,
            shared.prompt_density,
        );
        obj.insert("body".into(), json!(body_out));
        if let Some(items) = att_items {
            obj.insert("attachments_content".into(), json!(items));
            if any_truncated && shared.prompt_density != PromptDensity::UltraCompact {
                obj.insert(
                    "attachment_hint".into(),
                    json!("An attachment was truncated; call read_attachment with email_id and filename (or query/offset) to inspect more."),
                );
            }
        }
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
        match self.0.prompt_density {
            PromptDensity::Full => {
                "Full content, metadata, and extracted attachment text of one email by id (as \
                 returned by search_emails or cited as [#id] in earlier answers). Small attachments \
                 are included in full."
                    .into()
            }
            PromptDensity::Compact => {
                "Read email body, metadata and extracted attachments by id.".into()
            }
            PromptDensity::UltraCompact => "Read email & attachments by id.".into(),
        }
    }

    fn parameters(&self) -> Value {
        let id_schema = match self.0.prompt_density {
            PromptDensity::Full => json!({"type": "integer", "description": "Email id."}),
            PromptDensity::Compact | PromptDensity::UltraCompact => json!({"type": "integer"}),
        };
        json!({
            "type": "object",
            "properties": {"id": id_schema},
            "required": ["id"]
        })
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.run(args)
    }
}

// ---------------------------------------------------------------------------
// read_attachment

#[derive(Deserialize)]
pub struct ReadAttachmentArgs {
    email_id: Value,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    offset: Option<Value>,
}

pub struct ReadAttachmentTool(Arc<Shared>);

impl ReadAttachmentTool {
    fn run(&self, args: ReadAttachmentArgs) -> Result<Value, ToolFailure> {
        let shared = &self.0;
        let Some(email_id) = parse_id(&args.email_id) else {
            return Ok(json!({"error": "`email_id` must be a numeric email id."}));
        };
        let requested_name = args
            .filename
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        (shared.progress)(
            shared
                .language
                .agent_progress_attachment(email_id, requested_name),
        );

        let Some(hit) = shared.db().get_hit(email_id)? else {
            shared.record(format!("read_attachment #{email_id} → not found"));
            return Ok(json!({"error": format!("No email with id {email_id}.")}));
        };

        if hit.attachments.is_empty() {
            shared.record(format!(
                "read_attachment #{email_id} → no extracted attachments"
            ));
            return Ok(if hit.attachment_names.trim().is_empty() {
                json!({"error": format!("Email #{email_id} has no attachments.")})
            } else {
                json!({
                    "error": format!("Attachments on email #{email_id} are not in a supported text/document format or had no extractable text."),
                    "attachments": hit.attachment_names,
                })
            });
        }

        let target = if let Some(name_query) = requested_name {
            let lower = name_query.to_ascii_lowercase();
            hit.attachments
                .iter()
                .find(|a| a.filename.eq_ignore_ascii_case(name_query))
                .or_else(|| {
                    hit.attachments
                        .iter()
                        .find(|a| a.filename.to_ascii_lowercase().contains(&lower))
                })
        } else if hit.attachments.len() == 1 {
            hit.attachments.first()
        } else {
            None
        };

        let Some(att) = target else {
            let available: Vec<Value> = hit
                .attachments
                .iter()
                .map(|a| {
                    json!({
                        "filename": a.filename,
                        "size_bytes": a.size_bytes,
                        "total_chars": a.text.chars().count(),
                    })
                })
                .collect();
            shared.record(format!(
                "read_attachment #{email_id} → listed {} attachment(s)",
                available.len()
            ));
            shared.remember([hit], false);
            return Ok(json!({
                "email_id": email_id,
                "error": if requested_name.is_some() {
                    "Requested attachment filename was not found on this email."
                } else {
                    "Email has multiple attachments; specify `filename` from `available_attachments`."
                },
                "available_attachments": available,
            }));
        };

        let total_chars = att.text.chars().count();
        let mut obj = Map::new();
        obj.insert("email_id".into(), json!(email_id));
        obj.insert("filename".into(), json!(att.filename));
        if !att.mime_type.is_empty() && shared.prompt_density == PromptDensity::Full {
            obj.insert("mime_type".into(), json!(att.mime_type));
        }
        obj.insert("size_bytes".into(), json!(att.size_bytes));
        obj.insert("total_chars".into(), json!(total_chars));

        if let Some(q) = args
            .query
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let tokens = query::parse_query(q).tokens();
            let (passages, matched) =
                text::best_passages_matched(&att.text, &tokens, shared.read_email_max_chars);
            obj.insert("query".into(), json!(q));
            obj.insert("matched".into(), json!(matched));
            obj.insert("content".into(), json!(passages));
            shared.record(format!(
                "read_attachment #{email_id} \"{}\" query:\"{q}\"",
                att.filename
            ));
        } else {
            let offset = args
                .offset
                .as_ref()
                .and_then(parse_id)
                .map(|n| n.max(0) as usize)
                .unwrap_or(0)
                .min(total_chars);
            let slice: String = att
                .text
                .chars()
                .skip(offset)
                .take(shared.read_email_max_chars)
                .collect();
            let returned_chars = slice.chars().count();
            let next_offset = offset + returned_chars;
            obj.insert("offset".into(), json!(offset));
            obj.insert("returned_chars".into(), json!(returned_chars));
            obj.insert("content".into(), json!(slice));
            if next_offset < total_chars {
                obj.insert("truncated".into(), json!(true));
                obj.insert("next_offset".into(), json!(next_offset));
            } else {
                obj.insert("truncated".into(), json!(false));
            }
            shared.record(format!(
                "read_attachment #{email_id} \"{}\" ({returned_chars}/{total_chars} chars)",
                att.filename
            ));
        }

        shared.remember([hit], true);
        Ok(Value::Object(obj))
    }
}

impl PortableTool for ReadAttachmentTool {
    const NAME: &'static str = "read_attachment";
    type Args = ReadAttachmentArgs;
    type Output = Value;
    type Error = ToolFailure;

    fn description(&self) -> String {
        match self.0.prompt_density {
            PromptDensity::Full => {
                "Read or search extracted text from an email's attachment (Word, PowerPoint, \
                 LibreOffice, text). Use when an attachment was truncated in read_email, or pass \
                 `query` to jump directly to matching sections."
                    .into()
            }
            PromptDensity::Compact => {
                "Read or search extracted attachment text on an email by `email_id`.".into()
            }
            PromptDensity::UltraCompact => "Read email attachment by email_id.".into(),
        }
    }

    fn parameters(&self) -> Value {
        match self.0.prompt_density {
            PromptDensity::Full => json!({
                "type": "object",
                "properties": {
                    "email_id": {"type": "integer", "description": "Email id."},
                    "filename": {"type": "string", "description": "Attachment filename (optional if email has only one extracted attachment)."},
                    "query": {"type": "string", "description": "Optional keywords to extract matching passages inside a long attachment."},
                    "offset": {"type": "integer", "description": "Character offset to continue reading (default 0)."}
                },
                "required": ["email_id"]
            }),
            PromptDensity::Compact | PromptDensity::UltraCompact => json!({
                "type": "object",
                "properties": {
                    "email_id": {"type": "integer"},
                    "filename": {"type": "string"},
                    "query": {"type": "string"},
                    "offset": {"type": "integer"}
                },
                "required": ["email_id"]
            }),
        }
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
            db.get_thread(&thread_id, shared.thread_max_messages)?
        };
        drop(db);
        (shared.progress)(shared.language.agent_progress_thread(total));
        if messages.is_empty() {
            shared.record(format!("get_thread {thread_id} → not found"));
            return Ok(json!({"error": format!("No thread with id {thread_id}.")}));
        }

        // Fit messages into `shared.thread_total_chars`: reserve ~140 chars per message header
        // and keep the root + most recent messages if budget is too tight for all of them.
        const HEADER_OVERHEAD_PER_MSG: usize = 140;
        const MIN_BODY_PER_MSG: usize = 100;
        let max_affordable_msgs =
            (shared.thread_total_chars / (HEADER_OVERHEAD_PER_MSG + MIN_BODY_PER_MSG)).max(1);
        let kept_messages: Vec<SearchHit> = if messages.len() <= max_affordable_msgs {
            messages
        } else if max_affordable_msgs == 1 {
            vec![messages.last().expect("non-empty").clone()]
        } else {
            let mut slice = Vec::with_capacity(max_affordable_msgs);
            slice.push(messages[0].clone());
            let tail_start = messages.len() - (max_affordable_msgs - 1);
            slice.extend(messages[tail_start..].iter().cloned());
            slice
        };

        let header_budget = kept_messages.len().saturating_mul(HEADER_OVERHEAD_PER_MSG);
        let body_budget = shared
            .thread_total_chars
            .saturating_sub(header_budget)
            .max(kept_messages.len().saturating_mul(MIN_BODY_PER_MSG));
        let per_message = (body_budget / kept_messages.len()).max(MIN_BODY_PER_MSG);

        let mut any_att_truncated = false;
        let items: Vec<Value> = kept_messages
            .iter()
            .map(|hit| {
                let mut obj = hit_header(hit, shared);
                obj.remove("thread_id");
                let body = indexed_text(hit);
                let (body_out, att_items, truncated) = format_body_and_attachments(
                    &body,
                    &hit.attachments,
                    per_message,
                    shared.prompt_density,
                );
                if truncated {
                    any_att_truncated = true;
                }
                obj.insert("body".into(), json!(body_out));
                if let Some(att) = att_items {
                    obj.insert("attachments_content".into(), json!(att));
                }
                Value::Object(obj)
            })
            .collect();
        let mut out = json!({
            "thread_id": thread_id,
            "total_messages": total,
            "messages": items,
        });
        if any_att_truncated && shared.prompt_density != PromptDensity::UltraCompact {
            out["attachment_hint"] = json!(
                "One or more attachments were truncated; call read_attachment with email_id and filename (or query/offset) to inspect more."
            );
        }
        if total > kept_messages.len() {
            out["note"] = json!(format!(
                "Showing the first message and the {} most recent (of {total}) within token budget. Use read_email for a full message.",
                kept_messages.len().saturating_sub(1)
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
        shared.remember(kept_messages, true);
        Ok(out)
    }
}

impl PortableTool for GetThreadTool {
    const NAME: &'static str = "get_thread";
    type Args = ThreadArgs;
    type Output = Value;
    type Error = ToolFailure;

    fn description(&self) -> String {
        match self.0.prompt_density {
            PromptDensity::Full => {
                "All messages of a conversation in chronological order (quoted replies removed). Pass \
                 the thread_id from search results, or an email_id to open that email's thread. Use it \
                 for discussions, decisions and status questions: the latest messages usually hold the \
                 outcome."
                    .into()
            }
            PromptDensity::Compact => {
                "Read chronological messages of a thread by `thread_id` or `email_id`.".into()
            }
            PromptDensity::UltraCompact => "Get thread messages by thread_id or email_id.".into(),
        }
    }

    fn parameters(&self) -> Value {
        match self.0.prompt_density {
            PromptDensity::Full => json!({
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string", "description": "Thread id from search results."},
                    "email_id": {"type": "integer", "description": "Any email id of the thread (alternative to thread_id)."}
                }
            }),
            PromptDensity::Compact | PromptDensity::UltraCompact => json!({
                "type": "object",
                "properties": {
                    "thread_id": {"type": "string"},
                    "email_id": {"type": "integer"}
                }
            }),
        }
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
        let default_top = 20.min(shared.stats_max_groups);
        let top = args
            .top
            .as_ref()
            .and_then(parse_id)
            .map(|n| n.clamp(1, shared.stats_max_groups as i64) as usize)
            .unwrap_or(default_top);
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
        match self.0.prompt_density {
            PromptDensity::Full => {
                "Counts emails matching keywords and/or filters, optionally grouped by sender, month, \
                 year, folder or label (top groups by count; months/years in chronological order). \
                 Use it for how-many / who-most / when / trend questions instead of counting search \
                 results. Keyword matching here requires all keywords."
                    .into()
            }
            PromptDensity::Compact => {
                "Count matching emails, optionally grouped by sender, month, year, folder or label."
                    .into()
            }
            PromptDensity::UltraCompact => "Count/group matching emails.".into(),
        }
    }

    fn parameters(&self) -> Value {
        let mut props = filter_properties(self.0.prompt_density);
        match self.0.prompt_density {
            PromptDensity::Full => {
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
                    json!({"type": "integer", "description": format!("Maximum number of groups (default {}, max {}).", 20.min(self.0.stats_max_groups), self.0.stats_max_groups)}),
                );
            }
            PromptDensity::Compact | PromptDensity::UltraCompact => {
                props.insert(
                    "group_by".into(),
                    json!({"type": "string", "enum": ["sender", "month", "year", "folder", "label"]}),
                );
                props.insert("query".into(), json!({"type": "string"}));
                props.insert(
                    "top".into(),
                    json!({"type": "integer", "description": format!("Max {}", self.0.stats_max_groups)}),
                );
            }
        }
        json!({"type": "object", "properties": props})
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.run(args)
    }
}

// ---------------------------------------------------------------------------
// citations

/// Extracts all candidate email citation IDs (e.g. `[#123]`, `[#123, #456]`, `[Email #123]`)
/// referenced in `answer`, including pre-linked `[#123](mail://123)`.
pub fn extract_citation_ids(answer: &str) -> Vec<i64> {
    let mut ids = Vec::new();
    let mut rest = answer;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close) = after
            .find(']')
            .filter(|&c| c <= 80 && !after[..c].contains('\n'))
        else {
            rest = after;
            continue;
        };
        let inner = &after[..close];
        if let Some(parsed) = parse_citation_list(inner) {
            for id in parsed {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        rest = &after[close + 1..];
    }
    ids
}

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
        let mut following = &after[close + 1..];
        let ids = if let Some(after_mail) = following.strip_prefix("(mail://") {
            if let Some(paren_close) = after_mail.find(')')
                && after_mail[..paren_close]
                    .chars()
                    .all(|c| c.is_ascii_digit())
            {
                following = &after_mail[paren_close + 1..];
                parse_citation_list(inner)
            } else {
                None
            }
        } else if following.starts_with('(') {
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
    let budgets = config.effective_budgets();
    let db = Index::open(db_path)?;
    let configured_labels: Vec<String> = db
        .label_definitions()
        .unwrap_or_default()
        .into_iter()
        .map(|d| d.name)
        .collect();
    let shared = Arc::new(Shared {
        db: Mutex::new(db),
        language,
        prompt_density: budgets.prompt_density,
        default_limit: budgets.default_search_results,
        max_search_results: budgets.max_search_results,
        snippet_chars: budgets.snippet_chars,
        read_email_max_chars: budgets.read_email_max_chars,
        thread_max_messages: budgets.thread_max_messages,
        thread_total_chars: budgets.thread_total_chars,
        stats_max_groups: budgets.stats_max_groups,
        hit_recipients_max_chars: budgets.hit_recipients_max_chars,
        evidence: Mutex::new(Evidence::default()),
        trace: Mutex::new(Vec::new()),
        progress: progress.clone(),
    });

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

    let labels_note = if configured_labels.is_empty() {
        String::new()
    } else {
        let joined = configured_labels.join(", ");
        let capped = match budgets.prompt_density {
            PromptDensity::Full => joined,
            PromptDensity::Compact => text::truncate_chars(&joined, 160),
            PromptDensity::UltraCompact => text::truncate_chars(&joined, 90),
        };
        match language {
            Language::English => format!("\nLabels: {capped}."),
            Language::French => format!("\nÉtiquettes : {capped}."),
        }
    };
    let preamble = format!(
        "{}\n\n{} {}.{labels_note}",
        agent_system_prompt_for_density(language, budgets.prompt_density),
        language.agent_today_label(),
        chrono::Local::now().format("%Y-%m-%d (%A)"),
    );

    let search_tool = SearchEmailsTool(shared.clone());
    let read_tool = ReadEmailTool(shared.clone());
    let attachment_tool = ReadAttachmentTool(shared.clone());
    let thread_tool = GetThreadTool(shared.clone());
    let stats_tool = EmailStatsTool(shared.clone());

    let overhead_tokens = estimate_tokens(&preamble)
        + 12
        + estimate_tool_def_tokens(
            <SearchEmailsTool as PortableTool>::NAME,
            &PortableTool::description(&search_tool),
            &PortableTool::parameters(&search_tool),
        )
        + estimate_tool_def_tokens(
            <ReadEmailTool as PortableTool>::NAME,
            &PortableTool::description(&read_tool),
            &PortableTool::parameters(&read_tool),
        )
        + estimate_tool_def_tokens(
            <ReadAttachmentTool as PortableTool>::NAME,
            &PortableTool::description(&attachment_tool),
            &PortableTool::parameters(&attachment_tool),
        )
        + estimate_tool_def_tokens(
            <GetThreadTool as PortableTool>::NAME,
            &PortableTool::description(&thread_tool),
            &PortableTool::parameters(&thread_tool),
        )
        + estimate_tool_def_tokens(
            <EmailStatsTool as PortableTool>::NAME,
            &PortableTool::description(&stats_tool),
            &PortableTool::parameters(&stats_tool),
        );

    let agent = client
        .agent(&config.model)
        .preamble(&preamble)
        .tool(search_tool)
        .tool(read_tool)
        .tool(attachment_tool)
        .tool(thread_tool)
        .tool(stats_tool)
        .build();

    let max_turns = budgets.max_turns;
    let working_tokens = budgets
        .input_budget_tokens
        .saturating_sub(overhead_tokens)
        .max(160);
    let single_tool_result_max_tokens = (working_tokens * 65 / 100).max(160);

    let hook = TokenBudgetHook {
        window_tokens: budgets.max_context_tokens,
        input_budget_tokens: budgets.input_budget_tokens,
        history_budget_tokens: budgets.history_budget_tokens,
        overhead_tokens,
        single_tool_result_max_tokens,
    };

    debug_log(format_args!(
        "Agent request: POST {endpoint_for_log}; model={:?}; max_turns={max_turns}; history_turns={}; question_bytes={}; window_tokens={:?}; overhead_tok={overhead_tokens}",
        config.model,
        history.len(),
        question.len(),
        budgets.max_context_tokens
    ));
    progress(language.agent_progress_thinking().to_string());

    let rig_history = to_rig_history(history);
    let runtime = super::runtime()?;
    let outcome = runtime.block_on(async {
        let runner = agent
            .runner(question)
            .history(rig_history)
            .max_turns(max_turns)
            .add_hook(hook);
        tokio::select! {
            result = runner.run() => Some(result),
            _ = wait_for_cancel(&cancel) => None,
        }
    });

    let raw_answer = match outcome {
        None => anyhow::bail!("{}", language.agent_cancelled()),
        Some(Ok(response)) => response.output,
        Some(Err(PromptError::MaxTurnsError { .. })) => {
            progress(language.agent_progress_finalizing().to_string());
            let hits = shared
                .evidence
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .for_context();
            shared.record("research budget exhausted → answering with collected evidence".into());
            let context = build_context(&hits, budgets.max_context_chars);
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

    // Resolve any cited email IDs from earlier conversation turns that exist in enabled sources
    let candidate_ids = extract_citation_ids(&raw_answer);
    for id in candidate_ids {
        let already_known = shared
            .evidence
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .hits
            .contains_key(&id);
        if !already_known && let Ok(Some(hit)) = shared.db().get_hit(id) {
            shared.remember([hit], true);
        }
    }

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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::model::{AttachmentText, Email};

    fn test_shared(db: Index, read_max: usize, snippet_chars: usize) -> Arc<Shared> {
        Arc::new(Shared {
            db: Mutex::new(db),
            language: Language::English,
            prompt_density: PromptDensity::Full,
            default_limit: 5,
            max_search_results: 10,
            snippet_chars,
            read_email_max_chars: read_max,
            thread_max_messages: 10,
            thread_total_chars: 4000,
            stats_max_groups: 20,
            hit_recipients_max_chars: 200,
            evidence: Mutex::new(Evidence::default()),
            trace: Mutex::new(Vec::new()),
            progress: Arc::new(|_| {}),
        })
    }

    #[test]
    fn agent_tools_expose_small_and_large_attachments() {
        let mut db = Index::open(Path::new(":memory:")).unwrap();
        let long_doc = format!(
            "{} FINAL_CLAUSE: Penalty cap is 15000 EUR. {}",
            "Standard boilerplate clause. ".repeat(40),
            "Closing terms. ".repeat(20)
        );
        db.index_batch(&[Email {
            source_key: "sample.pst".into(),
            message_id: "m-att-1".into(),
            conversation_id: "conv-att-1".into(),
            folder: "Inbox".into(),
            subject: "Vendor terms".into(),
            sender: "alice@example.test".into(),
            recipients: "bob@example.test".into(),
            sent_at: Some("2026-04-10T10:00:00Z".into()),
            body: "See attached.".into(),
            attachment_names: "note.txt, agreement.docx".into(),
            attachments: vec![
                AttachmentText {
                    filename: "note.txt".into(),
                    mime_type: "text/plain".into(),
                    size_bytes: 34,
                    text: "Small note: approved by CFO today.".into(),
                },
                AttachmentText {
                    filename: "agreement.docx".into(),
                    mime_type:
                        "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                            .into(),
                    size_bytes: 4096,
                    text: long_doc,
                },
            ],
        }])
        .unwrap();

        let shared = test_shared(db, 450, 200);

        // 1. search_emails finds attachment-only term and returns attachment_snippet
        let search_out = SearchEmailsTool(shared.clone())
            .run(SearchArgs {
                query: Some("FINAL_CLAUSE".into()),
                filters: SearchFilters::default(),
                limit: None,
            })
            .unwrap();
        assert_eq!(search_out["result_count"], 1);
        let first = &search_out["results"][0];
        let email_id = first["id"].as_i64().unwrap();
        assert!(
            first["attachment_snippet"]
                .as_str()
                .unwrap()
                .contains("FINAL_CLAUSE")
        );

        // 2. read_email keeps the small attachment in full while truncating the large one
        let read_out = ReadEmailTool(shared.clone())
            .run(ReadArgs {
                id: json!(email_id),
            })
            .unwrap();
        let atts = read_out["attachments_content"].as_array().unwrap();
        assert_eq!(atts.len(), 2);
        assert_eq!(atts[0]["filename"], "note.txt");
        assert_eq!(atts[0]["content"], "Small note: approved by CFO today.");
        assert!(atts[0].get("truncated").is_none());
        assert_eq!(atts[1]["filename"], "agreement.docx");
        assert_eq!(atts[1]["truncated"], json!(true));

        // 3. read_attachment without filename lists available attachments when multiple exist
        let list_out = ReadAttachmentTool(shared.clone())
            .run(ReadAttachmentArgs {
                email_id: json!(email_id),
                filename: None,
                query: None,
                offset: None,
            })
            .unwrap();
        assert_eq!(list_out["available_attachments"].as_array().unwrap().len(), 2);

        // 4. read_attachment can query directly inside the large attachment and reports match status
        let att_query_out = ReadAttachmentTool(shared.clone())
            .run(ReadAttachmentArgs {
                email_id: json!(email_id),
                filename: Some("agreement.docx".into()),
                query: Some("FINAL_CLAUSE".into()),
                offset: None,
            })
            .unwrap();
        assert_eq!(att_query_out["matched"], json!(true));
        assert!(
            att_query_out["content"]
                .as_str()
                .unwrap()
                .contains("Penalty cap is 15000 EUR")
        );

        let att_miss_out = ReadAttachmentTool(shared.clone())
            .run(ReadAttachmentArgs {
                email_id: json!(email_id),
                filename: Some("agreement.docx".into()),
                query: Some("nonexistent_token_xyz".into()),
                offset: None,
            })
            .unwrap();
        assert_eq!(att_miss_out["matched"], json!(false));

        // 5. read_attachment supports offset pagination through large attachments
        let page1 = ReadAttachmentTool(shared.clone())
            .run(ReadAttachmentArgs {
                email_id: json!(email_id),
                filename: Some("agreement.docx".into()),
                query: None,
                offset: Some(json!(0)),
            })
            .unwrap();
        assert_eq!(page1["truncated"], json!(true));
        let next_off = page1["next_offset"].as_u64().unwrap();
        assert!(next_off > 0);
        let page2 = ReadAttachmentTool(shared)
            .run(ReadAttachmentArgs {
                email_id: json!(email_id),
                filename: Some("agreement.docx".into()),
                query: None,
                offset: Some(json!(next_off)),
            })
            .unwrap();
        assert!(page2["returned_chars"].as_u64().unwrap() > 0);
    }
}
