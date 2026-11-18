use rig::{
    completion::Message,
    message::{AssistantContent, DocumentSourceKind, ToolResultContent, UserContent},
};

use super::{ChatTurn, LlmConfig, to_rig_history};
use crate::{
    model::{EmailRecord, LabelDefinition},
    text,
};

pub const MIN_CONTEXT_TOKENS: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptDensity {
    Full,
    Compact,
    UltraCompact,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveBudgets {
    pub max_context_tokens: Option<usize>,
    pub prompt_density: PromptDensity,
    pub output_reserve_tokens: usize,
    pub input_budget_tokens: usize,
    pub judge_output_reserve_tokens: usize,
    pub max_turns: usize,
    pub default_search_results: usize,
    pub max_search_results: usize,
    pub snippet_chars: usize,
    pub read_email_max_chars: usize,
    pub thread_max_messages: usize,
    pub thread_total_chars: usize,
    pub max_context_chars: usize,
    pub history_turns: usize,
    pub history_budget_tokens: usize,
    pub stats_max_groups: usize,
    pub judge_email_excerpt_chars: usize,
    pub hit_recipients_max_chars: usize,
}

/// Allocates `remaining` units fairly across items of given `lengths`, filling smaller
/// items completely first so small bodies or small attachments are never starved.
pub fn water_fill_quotas(lengths: &[usize], mut remaining: usize) -> Vec<usize> {
    let mut quotas = vec![0usize; lengths.len()];
    while remaining > 0 {
        let needy: Vec<usize> = lengths
            .iter()
            .enumerate()
            .filter_map(|(i, &len)| (quotas[i] < len).then_some(i))
            .collect();
        if needy.is_empty() {
            break;
        }
        let share = (remaining / needy.len()).max(1);
        let mut allocated = 0usize;
        for i in needy {
            if remaining == 0 {
                break;
            }
            let needed = lengths[i] - quotas[i];
            let add = needed.min(share).min(remaining);
            quotas[i] += add;
            remaining -= add;
            allocated += add;
        }
        if allocated == 0 {
            break;
        }
    }
    quotas
}

/// Conservative token estimator for mixed English/French prose and JSON payloads.
/// Uses UTF-8 byte length / 3 (rounded up) so accented French text and JSON
/// punctuation are never undercounted.
pub fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    text.len().div_ceil(3)
}

pub fn estimate_tool_def_tokens(
    name: &str,
    description: &str,
    parameters: &serde_json::Value,
) -> usize {
    let params_str = parameters.to_string();
    estimate_tokens(name) + estimate_tokens(description) + estimate_tokens(&params_str) + 14
}

pub fn estimate_message_tokens(msg: &Message) -> usize {
    const MESSAGE_OVERHEAD: usize = 8;
    match msg {
        Message::System { content } => MESSAGE_OVERHEAD + estimate_tokens(content),
        Message::User { content } => {
            let inner: usize = content
                .iter()
                .map(|item| match item {
                    UserContent::Text(t) => estimate_tokens(&t.text),
                    UserContent::ToolResult(tr) => {
                        let body: usize = tr
                            .content
                            .iter()
                            .map(|c| match c {
                                ToolResultContent::Text(t) => estimate_tokens(&t.text),
                                ToolResultContent::Json { value } => {
                                    estimate_tokens(&value.to_string())
                                }
                                ToolResultContent::Image(_) => 256,
                            })
                            .sum();
                        16 + estimate_tokens(&tr.name) + body
                    }
                    UserContent::Image(_) => 256,
                    UserContent::Audio(_) | UserContent::Video(_) => 256,
                    UserContent::Document(d) => {
                        let data_tokens = match &d.data {
                            DocumentSourceKind::Url(s)
                            | DocumentSourceKind::Base64(s)
                            | DocumentSourceKind::FileId(s)
                            | DocumentSourceKind::String(s) => estimate_tokens(s),
                            DocumentSourceKind::Raw(bytes) => bytes.len().div_ceil(3),
                            DocumentSourceKind::Unknown => 0,
                        };
                        12 + data_tokens
                    }
                })
                .sum();
            MESSAGE_OVERHEAD + inner
        }
        Message::Assistant { content, .. } => {
            let inner: usize = content
                .iter()
                .map(|item| match item {
                    AssistantContent::Text(t) => estimate_tokens(&t.text),
                    AssistantContent::ToolCall(tc) => {
                        16 + estimate_tokens(&tc.id)
                            + estimate_tokens(&tc.function.name)
                            + estimate_tokens(&tc.function.arguments.to_string())
                    }
                    AssistantContent::Reasoning(r) => r
                        .content
                        .iter()
                        .map(|rc| match rc {
                            rig::message::ReasoningContent::Text { text, .. } => {
                                estimate_tokens(text)
                            }
                            rig::message::ReasoningContent::Summary(s) => estimate_tokens(s),
                            rig::message::ReasoningContent::Encrypted(s) => estimate_tokens(s),
                            rig::message::ReasoningContent::Redacted { data } => {
                                estimate_tokens(data)
                            }
                        })
                        .sum(),
                    AssistantContent::Image(_) => 256,
                })
                .sum();
            MESSAGE_OVERHEAD + inner
        }
    }
}

/// Truncates `text` cleanly at a UTF-8 char boundary so `estimate_tokens(&out) <= max_tokens`
/// (assuming `max_tokens >= estimate_tokens(suffix)`).
pub fn truncate_to_token_budget(text: &str, max_tokens: usize, suffix: &str) -> String {
    if estimate_tokens(text) <= max_tokens {
        return text.to_string();
    }
    let max_bytes = max_tokens.saturating_mul(3);
    let suffix_bytes = suffix.len();
    if max_bytes <= suffix_bytes {
        let mut out = String::new();
        for ch in suffix.chars() {
            if out.len() + ch.len_utf8() > max_bytes {
                break;
            }
            out.push(ch);
        }
        return out;
    }
    let body_budget = max_bytes - suffix_bytes;
    let mut kept = 0usize;
    let mut last_ws = None;
    for (idx, ch) in text.char_indices() {
        let end = idx + ch.len_utf8();
        if end > body_budget {
            break;
        }
        kept = end;
        if ch.is_whitespace() {
            last_ws = Some(idx);
        }
    }
    if let Some(ws) = last_ws
        && ws >= kept * 4 / 5
    {
        kept = ws;
    }
    let mut out = String::with_capacity(kept + suffix.len());
    out.push_str(text[..kept].trim_end());
    out.push_str(suffix);
    out
}

pub fn cap_output_tokens(
    window: usize,
    total_input: usize,
    max_cap: usize,
    min_floor: usize,
) -> usize {
    let safety = (window / 20).clamp(12, 64);
    let remaining = window.saturating_sub(total_input);
    remaining
        .saturating_sub(safety)
        .min(max_cap)
        .max(remaining.min(min_floor))
        .max(16)
}

impl EffectiveBudgets {
    pub fn from_config(config: &LlmConfig) -> Self {
        let raw_turns = config.agent_max_turns_limit();
        let raw_default_search = config.context_email_limit();
        let raw_max_search = config.max_search_results_limit();
        let raw_snippet = config.snippet_chars_limit();
        let raw_read_email = config.read_email_max_chars_limit();
        let raw_thread_msgs = config.thread_max_messages_limit();
        let raw_thread_chars = config.thread_total_chars_limit();
        let raw_context_chars = config.max_context_chars.max(400);
        let raw_history_turns = config.history_turns_limit();
        let raw_judge_excerpt = config.judge_email_excerpt_chars.max(200);

        let Some(window) = config.max_context_tokens_limit() else {
            return Self {
                max_context_tokens: None,
                prompt_density: PromptDensity::Full,
                output_reserve_tokens: 0,
                input_budget_tokens: usize::MAX,
                judge_output_reserve_tokens: 0,
                max_turns: raw_turns,
                default_search_results: raw_default_search,
                max_search_results: raw_max_search,
                snippet_chars: raw_snippet,
                read_email_max_chars: raw_read_email,
                thread_max_messages: raw_thread_msgs,
                thread_total_chars: raw_thread_chars,
                max_context_chars: raw_context_chars,
                history_turns: raw_history_turns,
                history_budget_tokens: usize::MAX,
                stats_max_groups: 100,
                judge_email_excerpt_chars: raw_judge_excerpt,
                hit_recipients_max_chars: 200,
            };
        };

        let prompt_density = if window <= 1536 {
            PromptDensity::UltraCompact
        } else if window < 6144 {
            PromptDensity::Compact
        } else {
            PromptDensity::Full
        };

        let output_reserve_tokens = (window / 5).clamp(160, 2048).min(window / 2);
        let input_budget_tokens = window.saturating_sub(output_reserve_tokens);
        let judge_output_reserve_tokens = (window / 5).clamp(128, 384).min(window / 2);

        let estimated_overhead = match prompt_density {
            PromptDensity::UltraCompact => 430,
            PromptDensity::Compact => 710,
            PromptDensity::Full => 2160,
        };
        let working_tokens = input_budget_tokens
            .saturating_sub(estimated_overhead)
            .max(120);

        let read_email_max_chars =
            raw_read_email.min((working_tokens.saturating_mul(3) * 55 / 100).max(280));
        let thread_total_chars =
            raw_thread_chars.min((working_tokens.saturating_mul(3) * 60 / 100).max(280));

        let snippet_chars = raw_snippet.min((working_tokens / 4).max(80));
        let per_hit_tokens = (160 + snippet_chars).div_ceil(3).max(40);
        let target_search_tokens = (working_tokens * 45 / 100).max(120);
        let search_cap = (target_search_tokens / per_hit_tokens).max(1);
        let default_search_results = raw_default_search.min(search_cap).max(1);
        let max_search_results = raw_max_search
            .min((search_cap.saturating_mul(2)).max(2))
            .max(default_search_results);

        let thread_max_messages = raw_thread_msgs
            .min((thread_total_chars / 180).clamp(2, 200))
            .max(1);

        let rag_overhead = match prompt_density {
            PromptDensity::UltraCompact => 140,
            PromptDensity::Compact => 190,
            PromptDensity::Full => 320,
        };
        let rag_available = input_budget_tokens.saturating_sub(rag_overhead).max(120);
        let max_context_chars =
            raw_context_chars.min((rag_available.saturating_mul(3) * 70 / 100).max(360));

        let (history_turns, history_budget_tokens) = if working_tokens < 600 {
            (raw_history_turns.min(1), 120)
        } else if working_tokens < 1400 {
            (raw_history_turns.min(1), 250)
        } else if working_tokens < 3000 {
            (raw_history_turns.min(2), 500)
        } else {
            (raw_history_turns, working_tokens / 4)
        };

        let stats_max_groups = if working_tokens < 600 {
            8
        } else if working_tokens < 1400 {
            12
        } else if working_tokens < 3000 {
            20
        } else {
            50
        };

        let judge_available = window
            .saturating_sub(judge_output_reserve_tokens)
            .saturating_sub(260)
            .max(80);
        let judge_email_excerpt_chars =
            raw_judge_excerpt.min((judge_available.saturating_mul(3)).max(200));

        let hit_recipients_max_chars = match prompt_density {
            PromptDensity::UltraCompact => 80,
            PromptDensity::Compact => 120,
            PromptDensity::Full => 200,
        };

        // For ultra-low windows, keep max_turns bounded so the agent doesn't spin endlessly
        // forgetting older tool outputs; still leave at least 4 turns for search + read + answer.
        let max_turns = if window <= 1536 {
            raw_turns.min(6)
        } else if window < 4096 {
            raw_turns.min(8)
        } else {
            raw_turns
        };

        Self {
            max_context_tokens: Some(window),
            prompt_density,
            output_reserve_tokens,
            input_budget_tokens,
            judge_output_reserve_tokens,
            max_turns,
            default_search_results,
            max_search_results,
            snippet_chars,
            read_email_max_chars,
            thread_max_messages,
            thread_total_chars,
            max_context_chars,
            history_turns,
            history_budget_tokens,
            stats_max_groups,
            judge_email_excerpt_chars,
            hit_recipients_max_chars,
        }
    }
}

pub fn trim_chat_turns_to_budget(
    turns: &[ChatTurn],
    max_turns: usize,
    total_token_budget: usize,
) -> Vec<ChatTurn> {
    if max_turns == 0 || total_token_budget < 24 || turns.is_empty() {
        return Vec::new();
    }

    // Group into `(user, assistant)` pairs (fallback: singletons if unpaired).
    let mut pairs: Vec<Vec<ChatTurn>> = Vec::new();
    let mut i = 0;
    while i < turns.len() {
        if i + 1 < turns.len()
            && turns[i].role.eq_ignore_ascii_case("user")
            && turns[i + 1].role.eq_ignore_ascii_case("assistant")
        {
            pairs.push(vec![turns[i].clone(), turns[i + 1].clone()]);
            i += 2;
        } else {
            pairs.push(vec![turns[i].clone()]);
            i += 1;
        }
    }

    let kept_pairs: Vec<Vec<ChatTurn>> = pairs.into_iter().rev().take(max_turns).collect();
    let mut selected: Vec<Vec<ChatTurn>> = Vec::new();
    let mut used = 0usize;
    let per_msg_cap = (total_token_budget / 2).max(40);

    for pair in kept_pairs {
        let mut trimmed_pair = Vec::with_capacity(pair.len());
        let mut pair_tokens = 0usize;
        for turn in pair {
            let content = if turn.role.eq_ignore_ascii_case("assistant") {
                super::strip_mail_links(&turn.content)
            } else {
                turn.content.clone()
            };
            let content = truncate_to_token_budget(&content, per_msg_cap, "…");
            pair_tokens += estimate_tokens(&content) + 8;
            trimmed_pair.push(ChatTurn {
                role: turn.role,
                content,
            });
        }
        if used + pair_tokens <= total_token_budget {
            used += pair_tokens;
            selected.push(trimmed_pair);
        } else {
            break;
        }
    }

    selected.into_iter().rev().flatten().collect()
}

fn is_tool_call_message(msg: &Message) -> bool {
    matches!(
        msg,
        Message::Assistant { content, .. }
            if content.iter().any(|c| matches!(c, AssistantContent::ToolCall(_)))
    )
}

fn is_tool_result_message(msg: &Message) -> bool {
    matches!(
        msg,
        Message::User { content }
            if content.iter().any(|c| matches!(c, UserContent::ToolResult(_)))
    )
}

fn compact_tool_result_message(msg: &mut Message, max_tokens_per_result: usize) -> bool {
    let Message::User { content } = msg else {
        return false;
    };
    let mut changed = false;
    for item in content.iter_mut() {
        if let UserContent::ToolResult(tr) = item {
            let current_tokens: usize = tr
                .content
                .iter()
                .map(|c| match c {
                    ToolResultContent::Text(t) => estimate_tokens(&t.text),
                    ToolResultContent::Json { value } => estimate_tokens(&value.to_string()),
                    ToolResultContent::Image(_) => 256,
                })
                .sum();
            if current_tokens > max_tokens_per_result {
                let mut combined = String::new();
                for c in &tr.content {
                    match c {
                        ToolResultContent::Text(t) => {
                            if !combined.is_empty() {
                                combined.push('\n');
                            }
                            combined.push_str(&t.text);
                        }
                        ToolResultContent::Json { value } => {
                            if !combined.is_empty() {
                                combined.push('\n');
                            }
                            combined.push_str(&value.to_string());
                        }
                        ToolResultContent::Image(_) => {}
                    }
                }
                let prefix = "[Earlier tool result compacted] ";
                let text_budget = max_tokens_per_result.saturating_sub(estimate_tokens(prefix));
                let preview = truncate_to_token_budget(&combined, text_budget.max(12), "…");
                tr.content = vec![ToolResultContent::text(format!("{prefix}{preview}"))];
                changed = true;
            }
        }
    }
    changed
}

fn truncate_plain_message(msg: &mut Message, max_tokens: usize) -> bool {
    match msg {
        Message::User { content } => {
            if content
                .iter()
                .any(|c| matches!(c, UserContent::ToolResult(_)))
            {
                return false;
            }
            let mut changed = false;
            for item in content.iter_mut() {
                if let UserContent::Text(t) = item
                    && estimate_tokens(&t.text) > max_tokens
                {
                    t.text = truncate_to_token_budget(&t.text, max_tokens, "…");
                    changed = true;
                }
            }
            changed
        }
        Message::Assistant { content, .. } => {
            if content
                .iter()
                .any(|c| matches!(c, AssistantContent::ToolCall(_)))
            {
                return false;
            }
            let mut changed = false;
            for item in content.iter_mut() {
                if let AssistantContent::Text(t) = item
                    && estimate_tokens(&t.text) > max_tokens
                {
                    t.text = truncate_to_token_budget(&t.text, max_tokens, "…");
                    changed = true;
                }
            }
            changed
        }
        Message::System { .. } => false,
    }
}

/// Trims/compacts `history` so `overhead_tokens + prompt + history <= input_budget`
/// while preserving valid tool-call / tool-result pairing and the current user question.
/// Returns `(patched_history, was_modified)`.
pub fn trim_rig_history_for_completion(
    history: &[Message],
    prompt: &Message,
    input_budget: usize,
    overhead_tokens: usize,
    prior_history_budget: usize,
) -> (Vec<Message>, bool) {
    let prompt_tokens = estimate_message_tokens(prompt);
    let total_tokens = |hist: &[Message]| -> usize {
        overhead_tokens + prompt_tokens + hist.iter().map(estimate_message_tokens).sum::<usize>()
    };

    if history.is_empty() {
        return (Vec::new(), false);
    }

    // Identify where the current run starts:
    // On Turn 2+, `history` has at least one `Assistant(ToolCall)` after `User(question)`.
    // On Turn 1, there are no tool calls in `history`, so all of `history` is prior turns.
    let first_tool_call_idx = history.iter().position(is_tool_call_message);
    let run_start = match first_tool_call_idx {
        Some(idx) => history[..idx]
            .iter()
            .rposition(|m| matches!(m, Message::User { .. }) && !is_tool_result_message(m))
            .unwrap_or(0),
        None => history.len(),
    };

    let mut prior: Vec<Message> = history[..run_start].to_vec();
    let mut current: Vec<Message> = history[run_start..].to_vec();
    let mut modified = false;

    // 1. Enforce `prior_history_budget` on prior conversation turns.
    let prior_tokens = |p: &[Message]| p.iter().map(estimate_message_tokens).sum::<usize>();
    if prior_tokens(&prior) > prior_history_budget {
        let per_prior_cap = (prior_history_budget / 2).max(40);
        for msg in &mut prior {
            if estimate_message_tokens(msg) > per_prior_cap
                && truncate_plain_message(msg, per_prior_cap)
            {
                modified = true;
            }
        }
        while prior_tokens(&prior) > prior_history_budget && !prior.is_empty() {
            // Drop oldest (User, Assistant) pair if paired, else oldest message.
            let drop_n = if prior.len() >= 2
                && matches!(prior[0], Message::User { .. })
                && matches!(prior[1], Message::Assistant { .. })
            {
                2
            } else {
                1
            };
            prior.drain(..drop_n);
            modified = true;
        }
    }

    let combined_len = |p: &[Message], c: &[Message]| -> usize {
        overhead_tokens
            + prompt_tokens
            + p.iter().map(estimate_message_tokens).sum::<usize>()
            + c.iter().map(estimate_message_tokens).sum::<usize>()
    };

    if combined_len(&prior, &current) <= input_budget {
        if !modified && total_tokens(history) <= input_budget {
            return (history.to_vec(), false);
        }
        let mut out = prior;
        out.extend(current);
        return (out, modified);
    }

    // 2. Drop oldest prior conversation turns until within budget (or prior is empty).
    while combined_len(&prior, &current) > input_budget && !prior.is_empty() {
        let drop_n = if prior.len() >= 2
            && matches!(prior[0], Message::User { .. })
            && matches!(prior[1], Message::Assistant { .. })
        {
            2
        } else {
            1
        };
        prior.drain(..drop_n);
        modified = true;
    }

    if combined_len(&prior, &current) <= input_budget {
        let mut out = prior;
        out.extend(current);
        return (out, true);
    }

    // 3. Inside `current`, protect the trailing Assistant(ToolCall) when `prompt` is a ToolResult.
    let protected_tail_start =
        if is_tool_result_message(prompt) && current.last().is_some_and(is_tool_call_message) {
            current.len() - 1
        } else {
            current.len()
        };

    // 3a. Compact older ToolResult messages inside `current[..protected_tail_start]`.
    for idx in 0..protected_tail_start {
        if is_tool_result_message(&current[idx])
            && compact_tool_result_message(&mut current[idx], 48)
        {
            modified = true;
            if combined_len(&prior, &current) <= input_budget {
                let mut out = prior;
                out.extend(current);
                return (out, true);
            }
        }
    }

    // 3b. Drop oldest atomic `(Assistant(ToolCall), User(ToolResult))` pairs strictly after
    // `current[0]` (the user's active question) and strictly before `protected_tail_start`.
    let mut scan_start = if current
        .first()
        .is_some_and(|m| matches!(m, Message::User { .. }) && !is_tool_result_message(m))
    {
        1
    } else {
        0
    };
    while combined_len(&prior, &current) > input_budget {
        let cur_tail =
            if is_tool_result_message(prompt) && current.last().is_some_and(is_tool_call_message) {
                current.len() - 1
            } else {
                current.len()
            };
        if scan_start + 1 < cur_tail
            && is_tool_call_message(&current[scan_start])
            && is_tool_result_message(&current[scan_start + 1])
        {
            current.drain(scan_start..scan_start + 2);
            modified = true;
        } else if scan_start < cur_tail {
            // Single non-tool message (e.g. feedback retry turn)
            if is_tool_call_message(&current[scan_start]) {
                // Never orphan a tool call; skip past it if malformed
                scan_start += 1;
            } else {
                current.remove(scan_start);
                modified = true;
            }
        } else {
            break;
        }
    }

    // 4. Last resort: truncate the user's question in `current[0]` if still over budget.
    if combined_len(&prior, &current) > input_budget
        && let Some((first, tail)) = current.split_first_mut()
    {
        let rest_tokens: usize = overhead_tokens
            + prompt_tokens
            + tail.iter().map(estimate_message_tokens).sum::<usize>()
            + 8;
        let allowed_for_first = input_budget.saturating_sub(rest_tokens).max(24);
        if truncate_plain_message(first, allowed_for_first) {
            modified = true;
        }
    }

    let mut out = prior;
    out.extend(current);
    (out, modified)
}

pub fn build_rag_request_budgets(
    config: &LlmConfig,
    question: &str,
    context: &str,
    history: &[ChatTurn],
) -> (&'static str, String, Vec<Message>, Option<u64>) {
    let budgets = EffectiveBudgets::from_config(config);
    let preamble = super::context::rag_system_prompt(config.language, budgets.prompt_density);
    let Some(window) = budgets.max_context_tokens else {
        return (preamble, context.to_string(), to_rig_history(history), None);
    };

    let preamble_tokens = estimate_tokens(preamble) + 12;
    let question_tokens = estimate_tokens(question) + 12;
    let remaining_after_fixed = budgets
        .input_budget_tokens
        .saturating_sub(preamble_tokens + question_tokens);
    let hist_cap = budgets.history_budget_tokens.min(remaining_after_fixed / 3);
    let trimmed_turns = trim_chat_turns_to_budget(history, budgets.history_turns, hist_cap);
    let rig_messages = to_rig_history(&trimmed_turns);
    let history_tokens: usize = rig_messages.iter().map(estimate_message_tokens).sum();

    let context_budget = budgets
        .input_budget_tokens
        .saturating_sub(preamble_tokens + question_tokens + history_tokens + 16);
    let fitted_context = if estimate_tokens(context) > context_budget {
        truncate_to_token_budget(
            context,
            context_budget.max(40),
            "\n[... context truncated to fit LLM token limit ...]",
        )
    } else {
        context.to_string()
    };

    let total_input =
        preamble_tokens + question_tokens + history_tokens + estimate_tokens(&fitted_context) + 16;
    let max_output = cap_output_tokens(window, total_input, 4096, 64);

    (
        preamble,
        fitted_context,
        rig_messages,
        Some(max_output as u64),
    )
}

pub fn build_judge_prompt_and_max_tokens(
    config: &LlmConfig,
    email: &EmailRecord,
    definitions: &[LabelDefinition],
) -> (&'static str, String, Option<u64>) {
    let budgets = EffectiveBudgets::from_config(config);
    let preamble = match budgets.prompt_density {
        PromptDensity::UltraCompact => "Email classification judge. Output valid JSON only.",
        PromptDensity::Compact | PromptDensity::Full => {
            "You are an expert email classification judge. Evaluate the email strictly against the supplied label criteria and output valid JSON only."
        }
    };
    let footer = match budgets.prompt_density {
        PromptDensity::UltraCompact => {
            "\n\nReply ONLY with JSON: {\"labels\": [{\"label\": \"<name>\", \"confidence\": 0.92}]} (1-2 labels)."
        }
        PromptDensity::Compact | PromptDensity::Full => {
            "\n\nRespond ONLY with JSON in the exact format: {\"labels\": [{\"label\": \"<exact label name>\", \"confidence\": 0.92}]} selecting 1 or 2 matching labels from the list above."
        }
    };

    let build_rubric = |desc_limit: Option<usize>| -> String {
        let mut rubric = String::from("Available labels and criteria:\n");
        for def in definitions {
            let desc = match desc_limit {
                Some(limit) => text::truncate_chars(&def.description, limit),
                None => def.description.clone(),
            };
            if desc.is_empty() {
                rubric.push_str(&format!("- {}\n", def.name));
            } else {
                rubric.push_str(&format!("- {}: {}\n", def.name, desc));
            }
        }
        rubric
    };

    let Some(window) = budgets.max_context_tokens else {
        let rubric = build_rubric(None);
        let excerpt_chars = config.judge_email_excerpt_chars.max(200);
        let prompt = format!(
            "{rubric}\nEmail to evaluate:\nDate: {}\nFrom: {}\nTo: {}\nFolder: {}\nSubject: {}\nBody:\n{}{footer}",
            email.sent_at.as_deref().unwrap_or("unknown"),
            email.sender,
            email.recipients,
            email.folder,
            email.subject,
            email.excerpt(excerpt_chars)
        );
        return (preamble, prompt, None);
    };

    let input_budget = window.saturating_sub(budgets.judge_output_reserve_tokens);
    let sender = match budgets.prompt_density {
        PromptDensity::UltraCompact => text::truncate_chars(&email.sender, 80),
        PromptDensity::Compact => text::truncate_chars(&email.sender, 140),
        PromptDensity::Full => email.sender.clone(),
    };
    let recipients = match budgets.prompt_density {
        PromptDensity::UltraCompact => text::truncate_chars(&email.recipients, 100),
        PromptDensity::Compact => text::truncate_chars(&email.recipients, 160),
        PromptDensity::Full => email.recipients.clone(),
    };
    let folder = match budgets.prompt_density {
        PromptDensity::UltraCompact => text::truncate_chars(&email.folder, 60),
        PromptDensity::Compact => text::truncate_chars(&email.folder, 100),
        PromptDensity::Full => email.folder.clone(),
    };
    let subject = match budgets.prompt_density {
        PromptDensity::UltraCompact => text::truncate_chars(&email.subject, 100),
        PromptDensity::Compact => text::truncate_chars(&email.subject, 160),
        PromptDensity::Full => email.subject.clone(),
    };

    let header_block = format!(
        "\nEmail to evaluate:\nDate: {}\nFrom: {sender}\nTo: {recipients}\nFolder: {folder}\nSubject: {subject}\nBody:\n",
        email.sent_at.as_deref().unwrap_or("unknown"),
    );

    let fixed_tokens =
        estimate_tokens(preamble) + estimate_tokens(&header_block) + estimate_tokens(footer) + 20;
    let min_body_tokens = (input_budget / 3).clamp(60, 220);

    // Shrink rubric descriptions if needed so at least `min_body_tokens` remain for the email body.
    let mut rubric = build_rubric(None);
    for desc_cap in [160usize, 100, 60, 32, 0] {
        if fixed_tokens + estimate_tokens(&rubric) + min_body_tokens <= input_budget {
            break;
        }
        rubric = build_rubric(Some(desc_cap));
    }

    let body_token_budget = input_budget
        .saturating_sub(fixed_tokens + estimate_tokens(&rubric))
        .max(24);
    let max_excerpt_chars = budgets
        .judge_email_excerpt_chars
        .min(body_token_budget.saturating_mul(3).max(60));
    let raw_excerpt = email.excerpt(max_excerpt_chars);
    let body_excerpt = if estimate_tokens(&raw_excerpt) > body_token_budget {
        truncate_to_token_budget(&raw_excerpt, body_token_budget, "...")
    } else {
        raw_excerpt
    };

    let prompt = format!("{rubric}{header_block}{body_excerpt}{footer}");
    let total_input = estimate_tokens(preamble) + estimate_tokens(&prompt) + 20;
    let max_output =
        cap_output_tokens(window, total_input, budgets.judge_output_reserve_tokens, 48);

    (preamble, prompt, Some(max_output as u64))
}
