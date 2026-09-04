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
    let response = call.send()?.error_for_status()?.json::<Response>()?;
    response
        .choices
        .into_iter()
        .next()
        .map(|x| x.message.content)
        .context("LLM returned no answer")
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
    use super::validate_endpoint;

    #[test]
    fn permits_http_only_for_exact_ipv4_loopback() {
        assert!(validate_endpoint("http://127.0.0.1:11434/v1/chat/completions").is_ok());
        assert!(validate_endpoint("http://localhost:11434/v1/chat/completions").is_err());
        assert!(validate_endpoint("http://192.168.1.20/v1/chat/completions").is_err());
        assert!(validate_endpoint("https://llm.internal.example/v1/chat/completions").is_ok());
    }
}
