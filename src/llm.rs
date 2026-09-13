use anyhow::{Context, Result, bail};
use rig::{completion::Prompt, prelude::*, providers::openai};
use serde::{Deserialize, Serialize};

use crate::model::SearchHit;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LlmConfig {
    pub endpoint: String,
    pub model: String,
    #[serde(default)]
    pub bearer_token: String,
    #[serde(default = "default_context_chars")]
    pub max_context_chars: usize,
}

fn default_context_chars() -> usize {
    16_000
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            endpoint: "https://llm.internal.example/v1/chat/completions".into(),
            model: "private-model".into(),
            bearer_token: String::new(),
            max_context_chars: default_context_chars(),
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
    use super::{endpoint_for_log, openai_base_url, validate_endpoint};

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
}
