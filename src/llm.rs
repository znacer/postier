use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
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

#[derive(Serialize)]
struct Request<'a> {
    model: &'a str,
    stream: bool,
    messages: Vec<Message<'a>>,
}
#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}
#[derive(Deserialize)]
struct Response {
    choices: Vec<Choice>,
}
#[derive(Deserialize)]
struct Choice {
    message: Answer,
}
#[derive(Deserialize)]
struct Answer {
    content: String,
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
    let client = Client::builder()
        .https_only(url.scheme() == "https")
        .build()?;
    let request = Request {
        model: &config.model,
        stream: false,
        messages: vec![
            Message {
                role: "system",
                content: context,
            },
            Message {
                role: "user",
                content: question,
            },
        ],
    };
    let mut call = client.post(url).json(&request);
    if !config.bearer_token.is_empty() {
        call = call.bearer_auth(&config.bearer_token);
    }
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

    let response = match call.send() {
        Ok(response) => response,
        Err(error) => {
            debug_log(format_args!("LLM transport error: {error:#}"));
            return Err(error).context("send request to LLM endpoint");
        }
    };
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("unknown")
        .to_owned();
    let allowed_methods = response
        .headers()
        .get(reqwest::header::ALLOW)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("not provided")
        .to_owned();
    let request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("not provided")
        .to_owned();
    let body = response.text().context("read LLM response body")?;

    debug_log(format_args!(
        "LLM response: status={status}; content-type={content_type:?}; allow={allowed_methods:?}; x-request-id={request_id:?}; body_bytes={}",
        body.len()
    ));

    if !status.is_success() {
        let body_preview = truncate_for_log(&body, 2_000);
        debug_log(format_args!("LLM error body: {body_preview}"));
        if status == reqwest::StatusCode::METHOD_NOT_ALLOWED {
            bail!(
                "LLM endpoint returned 405 Method Not Allowed for POST {endpoint_for_log}. Check that the URL is the chat-completions endpoint (often /v1/chat/completions). Server Allow header: {allowed_methods}. Response: {body_preview}"
            );
        }
        bail!(
            "LLM endpoint returned {status} for POST {endpoint_for_log}. Response: {body_preview}"
        );
    }

    let response: Response = serde_json::from_str(&body).map_err(|error| {
        debug_log(format_args!(
            "LLM JSON decode error: {error}; body: {}",
            truncate_for_log(&body, 2_000)
        ));
        error
    })?;
    response
        .choices
        .into_iter()
        .next()
        .map(|x| x.message.content)
        .context("LLM returned no answer")
}

fn endpoint_for_log(url: &url::Url) -> String {
    let host = url.host_str().unwrap_or("<missing-host>");
    let port = url
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    format!("{}://{host}{port}{}", url.scheme(), url.path())
}

fn truncate_for_log(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let preview: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{preview}… [truncated]")
    } else {
        preview
    }
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
    use super::{endpoint_for_log, truncate_for_log, validate_endpoint};

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
    fn truncates_debug_response_on_character_boundaries() {
        assert_eq!(truncate_for_log("éclair", 2), "éc… [truncated]");
        assert_eq!(truncate_for_log("ok", 2), "ok");
    }
}
