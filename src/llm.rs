use anyhow::{Context, Result, bail};
use rig::{completion::Prompt, embeddings::EmbeddingModel, prelude::*, providers::openai};
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

pub fn build_context(hits: &[SearchHit], max_chars: usize) -> String {
    let mut out = String::from(
        "Use only the email evidence below. Cite sources as [Email N]. If the evidence is insufficient, say so.\n\n",
    );
    for (i, hit) in hits.iter().enumerate() {
        let header = format!(
            "[Email {}]\nDate: {}\nFrom: {}\nTo: {}\nFolder: {}\nSubject: {}\nBody:\n",
            i + 1,
            hit.sent_at.as_deref().unwrap_or("unknown"),
            hit.sender,
            hit.recipients,
            hit.folder,
            hit.subject
        );
        if out.len() + header.len() >= max_chars {
            break;
        }
        out.push_str(&header);
        let remaining = max_chars.saturating_sub(out.len() + 2);
        out.push_str(&hit.excerpt(remaining));
        out.push_str("\n\n");
        if out.len() >= max_chars {
            break;
        }
    }
    out
}

pub fn ask(config: &LlmConfig, question: &str, context: &str) -> Result<String> {
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
        .preamble(
            "Answer the user's question using only the supplied email evidence. Cite sources as [Email N]. If the evidence is insufficient, say so.",
        )
        .context(context)
        .build();
    debug_log(format_args!(
        "LLM request: POST {endpoint_for_log}; model={:?}; auth={}; question_bytes={}; context_bytes={}",
        config.model,
        if config.bearer_token.is_empty() {
            "none"
        } else {
            "bearer (hidden)"
        },
        question.len(),
        context.len()
    ));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("start Rig runtime")?;
    runtime
        .block_on(async { agent.prompt(question).await })
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
}
