use anyhow::{Context, Result, bail};
use rig::{
    completion::{Chat, Prompt},
    embeddings::EmbeddingModel,
    prelude::*,
    providers::openai,
};
use serde::{Deserialize, Serialize};

use crate::model::{AssignedLabel, EmailRecord, LabelDefinition, LabelerBackend, SearchHit};

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
    #[serde(default)]
    pub labeler_backend: LabelerBackend,
    #[serde(default)]
    pub laya_gguf_path: String,
}

fn default_context_chars() -> usize {
    16_000
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
            labeler_backend: LabelerBackend::Laya,
            laya_gguf_path: String::new(),
        }
    }
}

pub const AGENT_SYSTEM_PROMPT: &str = r#"You are Dungbeetle, an expert email archive intelligence assistant.
Your task is to answer user questions accurately, objectively, and concisely using ONLY the provided email evidence.

Key Instructions:
1. Grounding & Citations:
   - Base all answers strictly on the email evidence provided. Never hallucinate facts, dates, or assume information not present in the emails.
   - Always cite your sources using bracketed citations like [Email 1], [Email 2] for every substantive claim, quote, decision, or action item.
   - If multiple emails corroborate or discuss a point, cite all relevant emails (e.g., [Email 1, Email 3]).

2. Chronology & Evolution:
   - Pay close attention to email dates and chronological progression. Discussions evolve over time, and later emails often revise, supersede, or finalize proposals made in earlier messages.
   - Explicitly note timelines and sequence when answering questions about status, plans, or outcomes (e.g., "Initially scheduled for May in [Email 1], but postponed to June in [Email 2]").

3. Metadata & Context Awareness:
   - Leverage metadata (From, To, Subject, Folder, Archive, Attachments, and Labels such as Urgent or Action Required) when relevant.
   - If the user asks about files, agreements, or attachments, check the Attachments metadata.
   - Identify senders and recipients clearly.

4. Formatting & Synthesis:
   - Begin with a direct, synthesized summary answer.
   - Use structured bullet points, clear headings, or chronological summaries for complex topics.
   - Distinguish clearly between concrete decisions, open questions, and pending action items.

5. Evidence Limits:
   - If the provided email evidence is insufficient, incomplete, or contradictory, explicitly state what is known and what cannot be determined from the available records.
"#;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatTurn {
    pub role: String,
    pub content: String,
}

pub fn clean_email_body(raw: &str) -> String {
    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized.lines().map(|l| l.trim_end()).collect();
    if lines.is_empty() {
        return String::new();
    }

    let mut out_lines: Vec<String> = Vec::with_capacity(lines.len());
    let mut consecutive_empty = 0;
    let mut in_deep_quote = false;
    let mut in_extended_quote = false;
    let mut quote_lines_count = 0;

    for (line_idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        // 1. Detect legal disclaimer footers (especially in the bottom portion of the message)
        if line_idx > lines.len().saturating_sub(25) || line_idx > lines.len() / 2 {
            let lower = trimmed.to_lowercase();
            if lower.starts_with("confidentiality notice")
                || lower.starts_with("this e-mail and any attachments")
                || lower.starts_with("this email and any attachments")
                || lower.starts_with("this email, including any attachments")
                || lower.starts_with("this message contains confidential information")
                || lower.starts_with("this communication is intended only for")
                || lower.starts_with("the information contained in this email")
                || lower.starts_with("privileged/confidential information")
                || lower.starts_with("please consider the environment before printing")
                || lower.starts_with("this message is for the designated recipient only")
            {
                out_lines.push("[Legal disclaimer omitted]".into());
                break;
            }
        }

        // 2. Normalize horizontal divider lines
        if trimmed.len() >= 10
            && (trimmed.chars().all(|c| c == '-')
                || trimmed.chars().all(|c| c == '=')
                || trimmed.chars().all(|c| c == '_'))
        {
            out_lines.push("---".into());
            consecutive_empty = 0;
            continue;
        }

        // 3. Compact deep nested quotes (> > >) and extended quote chains
        let is_quote_line = trimmed.starts_with('>');
        if is_quote_line {
            if in_extended_quote {
                continue;
            }
            let depth = trimmed
                .chars()
                .take_while(|&c| c == '>' || c == ' ')
                .filter(|&c| c == '>')
                .count();
            if depth > 2 {
                if !in_deep_quote {
                    out_lines.push("[... deep nested quoted reply omitted ...]".into());
                    in_deep_quote = true;
                }
                continue;
            }
            in_deep_quote = false;
            quote_lines_count += 1;
            if quote_lines_count > 6 {
                out_lines.push("[... extended quoted reply chain condensed ...]".into());
                in_extended_quote = true;
                continue;
            }
        } else {
            in_deep_quote = false;
            in_extended_quote = false;
            quote_lines_count = 0;
        }

        // 4. Collapse consecutive empty lines
        if trimmed.is_empty() {
            consecutive_empty += 1;
            if consecutive_empty <= 2 {
                out_lines.push(String::new());
            }
        } else {
            consecutive_empty = 0;
            out_lines.push(line.to_string());
        }
    }

    let result = out_lines.join("\n");
    result.trim().to_string()
}

pub fn build_context(hits: &[SearchHit], max_chars: usize) -> String {
    if hits.is_empty() {
        return "No relevant emails were found matching the query.".into();
    }

    // 1. Collect date span & archives for overview
    let mut dates: Vec<&str> = hits
        .iter()
        .filter_map(|h| h.sent_at.as_deref())
        .filter(|d| !d.is_empty())
        .collect();
    dates.sort();
    let date_summary = match (dates.first(), dates.last()) {
        (Some(first), Some(last)) if first == last => format!("Date: {first}"),
        (Some(first), Some(last)) => format!("Dates: {first} to {last}"),
        _ => "Dates: unknown".into(),
    };

    let mut archives: Vec<&str> = hits.iter().map(|h| h.archive_name()).collect();
    archives.sort();
    archives.dedup();
    let archive_summary = if archives.is_empty() {
        String::new()
    } else {
        format!(" | Archives: {}", archives.join(", "))
    };

    let overview = format!(
        "# Retrieved Email Evidence ({} messages | {}{})\n\
         Cite sources using [Email N] format. Take note of timestamps and chronological sequence.\n\n",
        hits.len(),
        date_summary,
        archive_summary
    );

    // 2. Prepare headers and cleaned bodies for each hit
    struct HitContextItem<'a> {
        header: String,
        cleaned_body: String,
        #[allow(dead_code)]
        hit: &'a SearchHit,
    }

    let mut items: Vec<HitContextItem> = Vec::with_capacity(hits.len());
    for (i, hit) in hits.iter().enumerate() {
        let mut header = format!(
            "[Email {}] (Relevance: Rank #{} | Score: {:.3})\n\
             Date: {}\n\
             From: {}\n\
             To: {}\n\
             Subject: {}\n\
             Folder: {}\n\
             Archive: {}\n",
            i + 1,
            i + 1,
            hit.score,
            hit.sent_at.as_deref().unwrap_or("unknown"),
            if hit.sender.is_empty() {
                "(unknown sender)"
            } else {
                &hit.sender
            },
            if hit.recipients.is_empty() {
                "(unknown recipients)"
            } else {
                &hit.recipients
            },
            if hit.subject.is_empty() {
                "(no subject)"
            } else {
                &hit.subject
            },
            if hit.folder.is_empty() {
                "(root)"
            } else {
                &hit.folder
            },
            hit.archive_name()
        );

        if !hit.message_id.trim().is_empty() {
            header.push_str(&format!("Message-ID: {}\n", hit.message_id.trim()));
        }

        if !hit.conversation_id.trim().is_empty() {
            header.push_str(&format!("Thread: {}\n", hit.conversation_id.trim()));
        }

        if !hit.attachment_names.trim().is_empty() {
            header.push_str(&format!("Attachments: {}\n", hit.attachment_names.trim()));
        }

        if !hit.labels.is_empty() {
            let label_list = hit
                .labels
                .iter()
                .map(|l| {
                    if let Some(conf) = l.confidence {
                        format!("{} ({:.0}%)", l.label, conf * 100.0)
                    } else {
                        l.label.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            header.push_str(&format!("Labels: {}\n", label_list));
        }

        header.push_str("Body:\n");
        let cleaned = clean_email_body(&hit.body);
        items.push(HitContextItem {
            header,
            cleaned_body: cleaned,
            hit,
        });
    }

    // 3. Multi-hit Fair Budget Allocation (water-filling across items)
    let min_body_bytes = 250;
    let mut count_to_include = items.len();
    while count_to_include > 1 {
        let headers_len = overview.len()
            + items[..count_to_include]
                .iter()
                .map(|it| it.header.len() + 2)
                .sum::<usize>();
        let budget = max_chars.saturating_sub(headers_len);
        if budget >= count_to_include * min_body_bytes {
            break;
        }
        count_to_include -= 1;
    }

    let active_items = &items[..count_to_include];
    let active_headers_len = overview.len()
        + active_items
            .iter()
            .map(|it| it.header.len() + 2)
            .sum::<usize>();
    let mut remaining_budget = max_chars.saturating_sub(active_headers_len);

    let mut quotas = vec![0usize; count_to_include];
    while remaining_budget > 0 {
        let needy_indices: Vec<usize> = active_items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| (quotas[i] < item.cleaned_body.len()).then_some(i))
            .collect();
        if needy_indices.is_empty() {
            break;
        }
        let share = (remaining_budget / needy_indices.len()).max(1);
        let mut allocated_this_round = 0usize;
        for i in needy_indices {
            if remaining_budget == 0 {
                break;
            }
            let needed = active_items[i].cleaned_body.len() - quotas[i];
            let add = needed.min(share).min(remaining_budget);
            quotas[i] += add;
            remaining_budget -= add;
            allocated_this_round += add;
        }
        if allocated_this_round == 0 {
            break;
        }
    }
    if remaining_budget > 0 && !quotas.is_empty() {
        let bonus = remaining_budget / quotas.len();
        for q in &mut quotas {
            *q += bonus;
        }
    }

    // 4. Assemble context string within max_chars byte budget
    let mut out = overview;
    for (i, item) in active_items.iter().enumerate() {
        if out.len() + item.header.len() + 2 > max_chars {
            break;
        }
        out.push_str(&item.header);

        let remaining_for_item = max_chars.saturating_sub(out.len() + 2);
        let quota = quotas[i].min(remaining_for_item);
        let body_bytes = item.cleaned_body.len();
        if body_bytes <= quota {
            out.push_str(&item.cleaned_body);
        } else {
            let footer_reserve = 48;
            let target_bytes = quota.saturating_sub(footer_reserve);
            let mut kept_bytes = 0usize;
            for ch in item.cleaned_body.chars() {
                let ch_len = ch.len_utf8();
                if kept_bytes + ch_len > target_bytes {
                    break;
                }
                kept_bytes += ch_len;
            }
            let truncated_slice = &item.cleaned_body[..kept_bytes];
            let omitted_chars = item.cleaned_body[kept_bytes..].chars().count();
            let footer = format!("\n[... truncated ({omitted_chars} chars omitted) ...]");
            out.push_str(truncated_slice);
            if out.len() + footer.len() + 2 <= max_chars {
                out.push_str(&footer);
            }
        }
        out.push_str("\n\n");
    }

    out
}

pub fn contextualize_search_query(query: &str, history: &[ChatTurn]) -> String {
    let trimmed = query.trim();
    if history.is_empty() {
        return trimmed.to_string();
    }

    let words: Vec<&str> = trimmed.split_whitespace().collect();
    let is_short = words.len() <= 4;
    let lower = trimmed.to_lowercase();
    let has_pronoun = lower.contains(" that")
        || lower.starts_with("that")
        || lower.contains(" this")
        || lower.starts_with("this")
        || lower.contains(" it")
        || lower.starts_with("it")
        || lower.contains(" they")
        || lower.starts_with("they")
        || lower.contains(" he")
        || lower.starts_with("he ")
        || lower.contains(" she")
        || lower.starts_with("she ")
        || lower.contains(" there")
        || lower.contains(" then")
        || lower.starts_with("when was")
        || lower.starts_with("who was")
        || lower.starts_with("who sent")
        || lower.starts_with("what was");

    if !is_short && !has_pronoun {
        return trimmed.to_string();
    }

    if let Some(prev_user) = history
        .iter()
        .rev()
        .find(|t| t.role.eq_ignore_ascii_case("user"))
    {
        let prev_clean = prev_user
            .content
            .replace(['?', '.', ',', '!', '"', '\'', ';', ':'], " ");
        let stopwords = [
            "the",
            "a",
            "an",
            "is",
            "are",
            "was",
            "were",
            "what",
            "when",
            "where",
            "who",
            "which",
            "how",
            "can",
            "could",
            "would",
            "should",
            "find",
            "show",
            "tell",
            "about",
            "all",
            "any",
            "email",
            "emails",
            "message",
            "messages",
            "in",
            "on",
            "at",
            "to",
            "from",
            "for",
            "of",
            "my",
            "our",
            "your",
            "me",
            "summarize",
            "list",
            "do",
            "does",
            "did",
        ];
        let key_terms: Vec<&str> = prev_clean
            .split_whitespace()
            .filter(|w| w.len() >= 3 && !stopwords.contains(&w.to_lowercase().as_str()))
            .take(3)
            .collect();
        if !key_terms.is_empty() {
            let extra = key_terms.join(" ");
            if !lower.contains(&extra.to_lowercase()) {
                return format!("{trimmed} {extra}");
            }
        }
    }

    trimmed.to_string()
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

    let agent = client
        .agent(&config.model)
        .preamble(AGENT_SYSTEM_PROMPT)
        .context(context)
        .build();

    let mut rig_messages = Vec::new();
    for turn in history {
        if turn.role.eq_ignore_ascii_case("user") {
            rig_messages.push(rig::completion::Message::user(&turn.content));
        } else if turn.role.eq_ignore_ascii_case("assistant") {
            rig_messages.push(rig::completion::Message::assistant(&turn.content));
        }
    }

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

fn parse_judge_response(raw: &str, definitions: &[LabelDefinition]) -> Result<Vec<AssignedLabel>> {
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
    let http_client = reqwest::Client::builder()
        .https_only(url.scheme() == "https")
        .build()
        .context("build secure HTTP client")?;
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

fn openai_base_url(endpoint: &url::Url) -> Result<url::Url> {
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

fn endpoint_for_log(url: &url::Url) -> String {
    let host = url.host_str().unwrap_or("<missing-host>");
    let port = url
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    format!("{}://{host}{port}{}", url.scheme(), url.path())
}

fn debug_log(args: std::fmt::Arguments<'_>) {
    #[cfg(debug_assertions)]
    eprintln!("[dungbeetle] {args}");
    #[cfg(not(debug_assertions))]
    let _ = args;
}

fn validate_endpoint(endpoint: &str) -> Result<url::Url> {
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

#[cfg(test)]
mod tests {
    use super::{endpoint_for_log, openai_base_url, parse_judge_response, validate_endpoint};
    use crate::model::default_label_definitions;

    #[test]
    fn permits_http_only_for_exact_ipv4_loopback() {
        assert!(validate_endpoint("http://127.0.0.1:11434/v1/chat/completions").is_ok());
        assert!(validate_endpoint("http://localhost:11434/v1/chat/completions").is_err());
        assert!(validate_endpoint("http://192.168.1.20/v1/chat/completions").is_err());
        assert!(validate_endpoint("https://llm.internal.example/v1/chat/completions").is_ok());
    }

    #[test]
    fn debug_endpoint_omits_query_parameters_and_userinfo() {
        let url = url::Url::parse(
            "https://user:secret@example.test:8443/v1/chat/completions?api_key=hidden",
        )
        .unwrap();
        assert_eq!(
            endpoint_for_log(&url),
            "https://example.test:8443/v1/chat/completions"
        );
    }

    #[test]
    fn derives_rig_base_url_from_chat_completions_endpoint() {
        let endpoint =
            url::Url::parse("https://llm.example.test/openai/v1/chat/completions?secret=hidden")
                .unwrap();
        assert_eq!(
            openai_base_url(&endpoint).unwrap().as_str(),
            "https://llm.example.test/openai/v1"
        );
        let invalid = url::Url::parse("https://llm.example.test/v1/responses").unwrap();
        assert!(openai_base_url(&invalid).is_err());
    }

    #[test]
    fn parses_llm_judge_json_and_fenced_responses() {
        let defs = default_label_definitions();
        let raw =
            "```json\n{\"labels\": [{\"label\": \"Action Required\", \"confidence\": 0.94}]}\n```";
        let out = parse_judge_response(raw, &defs).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].label, "Action Required");
        assert_eq!(out[0].source, "llm-judge");
    }

    #[test]
    fn cleans_email_body_disclaimers_and_deep_quotes() {
        let raw = "Hello team,\n\nWe need to review the deployment.\n\n\
                   > On Monday, Alice wrote:\n\
                   > > Please check the logs\n\
                   > > > Deep nested thread line 1\n\
                   > > > Deep nested thread line 2\n\n\
                   --------------------------------------\n\
                   CONFIDENTIALITY NOTICE: This email and any attachments are confidential...";
        let cleaned = super::clean_email_body(raw);
        assert!(cleaned.contains("Hello team,"));
        assert!(cleaned.contains("We need to review the deployment."));
        assert!(cleaned.contains("[... deep nested quoted reply omitted ...]"));
        assert!(!cleaned.contains("Deep nested thread line 1"));
        assert!(cleaned.contains("[Legal disclaimer omitted]"));
        assert!(!cleaned.contains("CONFIDENTIALITY NOTICE"));
        assert!(cleaned.contains("---"));
    }

    #[test]
    fn build_context_includes_metadata_and_prevents_starvation() {
        use crate::model::{AssignedLabel, SearchHit};

        let hit1 = SearchHit {
            id: 1,
            source_key: "sample.pst".into(),
            subject: "Project Atlas".into(),
            sender: "alice@example.test".into(),
            recipients: "bob@example.test".into(),
            sent_at: Some("2025-03-10T10:00:00Z".into()),
            folder: "Inbox".into(),
            body: "A—".repeat(2500),
            attachment_names: "specs.pdf, budget.xlsx".into(),
            conversation_id: "atlas-thread".into(),
            message_id: "m1@example.test".into(),
            labels: vec![AssignedLabel {
                label: "Urgent".into(),
                confidence: Some(0.95),
                source: "laya".into(),
            }],
            score: 0.95,
        };

        let hit2 = SearchHit {
            id: 2,
            source_key: "sample.pst".into(),
            subject: "Re: Project Atlas".into(),
            sender: "bob@example.test".into(),
            recipients: "alice@example.test".into(),
            sent_at: Some("2025-03-11T12:00:00Z".into()),
            folder: "Sent Items".into(),
            body: "Deployment is confirmed for Friday.".into(),
            attachment_names: String::new(),
            conversation_id: "atlas-thread".into(),
            message_id: "m2@example.test".into(),
            labels: vec![AssignedLabel {
                label: "Decision".into(),
                confidence: Some(0.88),
                source: "llm-judge".into(),
            }],
            score: 0.82,
        };

        let hits = vec![hit1, hit2];
        let context = super::build_context(&hits, 2500);

        assert!(context.len() <= 2500);
        assert!(context.contains("# Retrieved Email Evidence"));
        assert!(context.contains("Dates: 2025-03-10T10:00:00Z to 2025-03-11T12:00:00Z"));
        assert!(context.contains("Archives: sample.pst"));

        assert!(context.contains("[Email 1]"));
        assert!(context.contains("From: alice@example.test"));
        assert!(context.contains("Attachments: specs.pdf, budget.xlsx"));
        assert!(context.contains("Thread: atlas-thread"));
        assert!(context.contains("Labels: Urgent (95%)"));
        assert!(context.contains("Message-ID: m1@example.test"));

        // Hit 2 is not starved even when Hit 1 contains multi-byte UTF-8 characters!
        assert!(context.contains("[Email 2]"));
        assert!(context.contains("From: bob@example.test"));
        assert!(context.contains("Deployment is confirmed for Friday."));
        assert!(context.contains("Labels: Decision (88%)"));
    }

    #[test]
    fn contextualizes_anaphoric_follow_up_queries() {
        let history = vec![
            super::ChatTurn {
                role: "user".into(),
                content: "Tell me about the database migration schedule.".into(),
            },
            super::ChatTurn {
                role: "assistant".into(),
                content: "The migration is planned for Q4 as discussed in [Email 1].".into(),
            },
        ];

        let q1 = "Who is the lead for Project Apollo?";
        assert_eq!(super::contextualize_search_query(q1, &history), q1);

        let q2 = "When was that?";
        let enriched = super::contextualize_search_query(q2, &history);
        assert!(enriched.starts_with("When was that?"));
        assert!(enriched.contains("database"));
        assert!(enriched.contains("migration"));
    }
}
