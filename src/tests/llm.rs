use crate::{
    i18n::Language,
    llm::{
        EMBEDDED_CONFIG_TOML, LlmConfig, build_context, clean_email_body, endpoint_for_log,
        extract_citation_ids, linkify_citations, openai_base_url, parse_judge_response,
        strip_mail_links, validate_endpoint,
    },
    model::{AssignedLabel, LabelerBackend, SearchHit, default_label_definitions},
};

#[test]
fn embedded_config_toml_parses_and_provides_shipped_defaults() {
    let parsed = LlmConfig::from_toml_str(EMBEDDED_CONFIG_TOML).unwrap();
    let default_cfg = LlmConfig::default();
    assert_eq!(parsed, default_cfg);
    assert!(!default_cfg.endpoint.trim().is_empty());
    assert!(!default_cfg.model.trim().is_empty());
    assert!(default_cfg.agent_max_turns_limit() >= 1);
    assert!(default_cfg.context_email_limit() >= 1);
    assert!(default_cfg.max_search_results_limit() >= default_cfg.context_email_limit());
    assert!(default_cfg.snippet_chars_limit() >= 80);
    assert!(default_cfg.read_email_max_chars_limit() >= 400);
    assert!(default_cfg.thread_max_messages_limit() >= 1);
    assert!(default_cfg.thread_total_chars_limit() >= 1_000);
}

#[test]
fn custom_toml_overrides_llm_agent_laya_and_ui_settings() {
    let custom = r#"
        [llm]
        endpoint = "https://custom.api.example/v1/chat/completions"
        model = "custom-model"
        bearer_token = "sk-shipped-token"
        connect_timeout_secs = 25
        request_timeout_secs = 240

        [agent]
        max_turns = 20
        max_context_emails = 15
        max_search_results = 80
        snippet_chars = 500
        read_email_max_chars = 18000
        thread_max_messages = 60
        thread_total_chars = 20000
        max_context_chars = 24000
        history_turns = 5

        [laya]
        labeler_backend = "LlmJudge"
        judge_email_excerpt_chars = 5000
        batch_label_limit = 100

        [ui]
        language = "English"
        dark_mode = false
        explorer_message_limit = 500
    "#;
    let cfg = LlmConfig::from_toml_str(custom).unwrap();
    assert_eq!(
        cfg.endpoint,
        "https://custom.api.example/v1/chat/completions"
    );
    assert_eq!(cfg.model, "custom-model");
    assert_eq!(cfg.bearer_token, "sk-shipped-token");
    assert_eq!(cfg.connect_timeout_secs, 25);
    assert_eq!(cfg.request_timeout_secs, 240);
    assert_eq!(cfg.agent_max_turns_limit(), 20);
    assert_eq!(cfg.context_email_limit(), 15);
    assert_eq!(cfg.max_search_results_limit(), 80);
    assert_eq!(cfg.snippet_chars_limit(), 500);
    assert_eq!(cfg.read_email_max_chars_limit(), 18_000);
    assert_eq!(cfg.thread_max_messages_limit(), 60);
    assert_eq!(cfg.thread_total_chars_limit(), 20_000);
    assert_eq!(cfg.max_context_chars, 24_000);
    assert_eq!(cfg.history_turns_limit(), 5);
    assert_eq!(cfg.labeler_backend, LabelerBackend::LlmJudge);
    assert_eq!(cfg.judge_email_excerpt_chars, 5_000);
    assert_eq!(cfg.batch_label_limit, 100);
    assert_eq!(cfg.language, Language::English);
    assert!(!cfg.dark_mode);
    assert_eq!(cfg.explorer_message_limit, 500);

    // Partial JSON (such as an existing `credentials.json`) inherits shipped defaults for omitted fields
    let partial_json =
        r#"{"endpoint":"https://override.example/v1/chat/completions","bearer_token":"sk-user"}"#;
    let from_json: LlmConfig = serde_json::from_str(partial_json).unwrap();
    assert_eq!(
        from_json.endpoint,
        "https://override.example/v1/chat/completions"
    );
    assert_eq!(from_json.bearer_token, "sk-user");
    assert_eq!(
        from_json.agent_max_turns,
        LlmConfig::default().agent_max_turns
    );
    assert_eq!(
        from_json.max_search_results,
        LlmConfig::default().max_search_results
    );
}

#[test]
fn permits_http_only_for_exact_ipv4_loopback() {
    assert!(validate_endpoint("http://127.0.0.1:11434/v1/chat/completions").is_ok());
    assert!(validate_endpoint("http://localhost:11434/v1/chat/completions").is_err());
    assert!(validate_endpoint("http://192.168.1.20/v1/chat/completions").is_err());
    assert!(validate_endpoint("https://llm.internal.example/v1/chat/completions").is_ok());
}

#[test]
fn debug_endpoint_omits_query_parameters_and_userinfo() {
    let url =
        url::Url::parse("https://user:secret@example.test:8443/v1/chat/completions?api_key=hidden")
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
    let cleaned = clean_email_body(raw);
    assert!(cleaned.contains("Hello team,"));
    assert!(cleaned.contains("We need to review the deployment."));
    assert!(cleaned.contains("[... deep nested quoted reply omitted ...]"));
    assert!(!cleaned.contains("Deep nested thread line 1"));
    assert!(cleaned.contains("[Legal disclaimer omitted]"));
    assert!(!cleaned.contains("CONFIDENTIALITY NOTICE"));
    assert!(cleaned.contains("---"));

    let raw_fr = "Bonjour,\n\nVoici le rapport.\n\n\
                  Ce message et toutes les pièces jointes sont confidentiels et établis à l'intention exclusive de ses destinataires.";
    let cleaned_fr = clean_email_body(raw_fr);
    assert!(cleaned_fr.contains("Voici le rapport."));
    assert!(cleaned_fr.contains("[Legal disclaimer omitted]"));
    assert!(!cleaned_fr.contains("intention exclusive"));
}

#[test]
fn build_context_includes_metadata_and_prevents_starvation() {
    let hit1 = SearchHit {
        id: 1,
        source_key: "sample.pst".into(),
        subject: "Project Atlas".into(),
        sender: "alice@example.test".into(),
        recipients: "bob@example.test".into(),
        sent_at: Some("2025-03-10T10:00:00Z".into()),
        folder: "Inbox".into(),
        body: "A\u{2014}".repeat(2500),
        attachment_names: "specs.pdf, budget.xlsx".into(),
        conversation_id: "atlas-thread".into(),
        message_id: "m1@example.test".into(),
        labels: vec![AssignedLabel {
            label: "Urgent".into(),
            confidence: Some(0.95),
            source: "laya".into(),
        }],
        score: 0.95,
        ..Default::default()
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
        ..Default::default()
    };

    let hits = vec![hit1, hit2];
    let context = build_context(&hits, 2500);

    assert!(context.len() <= 2500);
    assert!(context.contains("# Retrieved Email Evidence"));
    assert!(context.contains("Dates: 2025-03-10T10:00:00Z to 2025-03-11T12:00:00Z"));
    assert!(context.contains("Archives: sample.pst"));

    assert!(context.contains("[#1]"));
    assert!(context.contains("From: alice@example.test"));
    assert!(context.contains("Attachments: specs.pdf, budget.xlsx"));
    assert!(context.contains("Thread: atlas-thread"));
    assert!(context.contains("Labels: Urgent (95%)"));
    assert!(context.contains("Message-ID: m1@example.test"));

    // Hit 2 is not starved even when Hit 1 contains multi-byte UTF-8 characters!
    assert!(context.contains("[#2]"));
    assert!(context.contains("From: bob@example.test"));
    assert!(context.contains("Deployment is confirmed for Friday."));
    assert!(context.contains("Labels: Decision (88%)"));
}

#[test]
fn linkify_citations_converts_known_ids_into_mail_urls() {
    use std::collections::HashSet;
    let known: HashSet<i64> = [42, 108].into_iter().collect();
    let text = "Decided in [#42] and confirmed in [Email #108], plus pre-linked [#42](mail://42), but not [#999] or [#999](mail://999).";
    let (linked, cited) = linkify_citations(text, &known);
    assert_eq!(
        linked,
        "Decided in [#42](mail://42) and confirmed in [#108](mail://108), plus pre-linked [#42](mail://42), but not #999 or #999."
    );
    assert_eq!(cited, vec![42, 108]);
}

#[test]
fn strips_mail_links_and_extracts_citation_ids() {
    let msg = "See [#42](mail://42) and [#108](mail://108), plus [docs](https://example.test).";
    assert_eq!(
        strip_mail_links(msg),
        "See [#42] and [#108], plus [docs](https://example.test)."
    );
    assert_eq!(extract_citation_ids(msg), vec![42, 108]);
}
