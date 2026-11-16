use crate::{
    i18n::Language,
    llm::{
        ChatTurn, EMBEDDED_CONFIG_TOML, EffectiveBudgets, LlmConfig, MIN_CONTEXT_TOKENS,
        PromptDensity, build_context, build_judge_prompt_and_max_tokens, build_rag_request_budgets,
        clean_email_body, endpoint_for_log, estimate_message_tokens, estimate_tokens,
        extract_citation_ids, linkify_citations, openai_base_url, parse_judge_response,
        strip_mail_links, trim_chat_turns_to_budget, trim_rig_history_for_completion,
        validate_endpoint,
    },
    model::{
        AssignedLabel, EmailRecord, LabelDefinition, LabelerBackend, SearchHit,
        default_label_definitions,
    },
};

#[test]
fn embedded_config_toml_parses_and_provides_shipped_defaults() {
    let parsed = LlmConfig::from_toml_str(EMBEDDED_CONFIG_TOML).unwrap();
    let default_cfg = LlmConfig::default();
    assert_eq!(parsed, default_cfg);
    assert!(!default_cfg.endpoint.trim().is_empty());
    assert!(!default_cfg.model.trim().is_empty());
    assert_eq!(default_cfg.max_context_tokens, 16_384);
    assert_eq!(default_cfg.max_context_tokens_limit(), Some(16_384));
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
        max_context_tokens = 2048
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
    assert_eq!(cfg.max_context_tokens, 2048);
    assert_eq!(cfg.max_context_tokens_limit(), Some(2048));
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

    // Flat alias `max_tokens` and special `0` / tiny values
    let flat_alias = LlmConfig::from_toml_str("max_tokens = 4096").unwrap();
    assert_eq!(flat_alias.max_context_tokens_limit(), Some(4096));

    let unlimited = LlmConfig::from_toml_str("[llm]\nmax_context_tokens = 0").unwrap();
    assert_eq!(unlimited.max_context_tokens_limit(), None);

    let clamped = LlmConfig::from_toml_str("[llm]\nmax_context_tokens = 128").unwrap();
    assert_eq!(clamped.max_context_tokens_limit(), Some(MIN_CONTEXT_TOKENS));

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
        from_json.max_context_tokens,
        LlmConfig::default().max_context_tokens
    );
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
fn effective_budgets_scale_across_context_window_tiers() {
    let mut cfg = LlmConfig {
        max_context_tokens: 0,
        ..Default::default()
    };

    // 0 -> unlimited (uses raw config limits verbatim)
    let unlim = EffectiveBudgets::from_config(&cfg);
    assert_eq!(unlim.max_context_tokens, None);
    assert_eq!(unlim.prompt_density, PromptDensity::Full);
    assert_eq!(unlim.read_email_max_chars, cfg.read_email_max_chars_limit());
    assert_eq!(unlim.default_search_results, cfg.context_email_limit());
    assert_eq!(unlim.history_turns, cfg.history_turns_limit());

    // 1024 -> UltraCompact with tight caps
    cfg.max_context_tokens = 1024;
    let b1k = EffectiveBudgets::from_config(&cfg);
    assert_eq!(b1k.max_context_tokens, Some(1024));
    assert_eq!(b1k.prompt_density, PromptDensity::UltraCompact);
    assert!(b1k.max_turns <= 6);
    assert!(b1k.default_search_results < cfg.context_email_limit());
    assert!(b1k.read_email_max_chars < 1500);
    assert!(b1k.thread_total_chars < 1500);
    assert!(b1k.history_turns <= 1);
    assert!(b1k.stats_max_groups <= 8);

    // 2048 & 4096 -> Compact, monotonically increasing with window size
    cfg.max_context_tokens = 2048;
    let b2k = EffectiveBudgets::from_config(&cfg);
    assert_eq!(b2k.prompt_density, PromptDensity::Compact);
    assert!(b2k.read_email_max_chars >= b1k.read_email_max_chars);
    assert!(b2k.default_search_results >= b1k.default_search_results);

    cfg.max_context_tokens = 4096;
    let b4k = EffectiveBudgets::from_config(&cfg);
    assert_eq!(b4k.prompt_density, PromptDensity::Compact);
    assert!(b4k.read_email_max_chars >= b2k.read_email_max_chars);
    assert!(b4k.thread_total_chars >= b2k.thread_total_chars);

    // 8192 & 16384 -> Full
    cfg.max_context_tokens = 8192;
    let b8k = EffectiveBudgets::from_config(&cfg);
    assert_eq!(b8k.prompt_density, PromptDensity::Full);
    assert!(b8k.read_email_max_chars >= b4k.read_email_max_chars);

    cfg.max_context_tokens = 16_384;
    let b16k = EffectiveBudgets::from_config(&cfg);
    assert_eq!(b16k.prompt_density, PromptDensity::Full);
    assert_eq!(b16k.default_search_results, cfg.context_email_limit());
    assert_eq!(b16k.history_turns, cfg.history_turns_limit());
}

#[test]
fn trim_chat_turns_to_budget_keeps_newest_within_token_cap() {
    let turns = vec![
        ChatTurn {
            role: "user".into(),
            content: "First old question about archived contracts.".repeat(6),
        },
        ChatTurn {
            role: "assistant".into(),
            content: "First answer referencing [#10](mail://10) and details.".repeat(6),
        },
        ChatTurn {
            role: "user".into(),
            content: "Latest question about Atlas?".into(),
        },
        ChatTurn {
            role: "assistant".into(),
            content: "Confirmed in [#42](mail://42).".into(),
        },
    ];

    let trimmed = trim_chat_turns_to_budget(&turns, 3, 90);
    assert_eq!(trimmed.len(), 2);
    assert_eq!(trimmed[0].content, "Latest question about Atlas?");
    assert_eq!(trimmed[1].content, "Confirmed in [#42].");

    let empty = trim_chat_turns_to_budget(&turns, 3, 10);
    assert!(empty.is_empty());
}

#[test]
fn trim_rig_history_preserves_tool_pairing_and_fits_budget() {
    use rig::completion::Message;
    use rig::message::{AssistantContent, UserContent};

    let huge_tool_output =
        "Project Atlas update log entry with lots of French text éàç. ".repeat(80);
    let history = vec![
        Message::user("Old turn question that should be evicted first."),
        Message::assistant("Old turn answer that should also be evicted."),
        Message::user("What happened with Atlas in March?"),
        Message::Assistant {
            id: None,
            content: vec![AssistantContent::tool_call(
                "call_1",
                "search_emails",
                serde_json::json!({"query": "Atlas"}),
            )],
        },
        Message::tool_result("call_1", "search_emails", huge_tool_output),
        Message::Assistant {
            id: None,
            content: vec![AssistantContent::tool_call(
                "call_2",
                "read_email",
                serde_json::json!({"id": 42}),
            )],
        },
    ];
    let prompt = Message::tool_result("call_2", "read_email", "Deployment approved on March 12.");

    let overhead = 380;
    let input_budget = 720;
    let (patched, changed) =
        trim_rig_history_for_completion(&history, &prompt, input_budget, overhead, 120);

    assert!(changed);
    let total = overhead
        + estimate_message_tokens(&prompt)
        + patched.iter().map(estimate_message_tokens).sum::<usize>();
    assert!(
        total <= input_budget,
        "expected trimmed total {total} <= input_budget {input_budget}"
    );

    // Current question must remain at start, and active trailing call_2 for `prompt` must remain intact.
    assert!(
        patched
            .first()
            .and_then(Message::rag_text)
            .is_some_and(|t| t.contains("Atlas in March"))
    );
    let has_call_2 = matches!(
        patched.last(),
        Some(Message::Assistant { content, .. })
            if content.iter().any(|c| matches!(c, AssistantContent::ToolCall(tc) if tc.function.name == "read_email"))
    );
    assert!(
        has_call_2,
        "active ToolCall for prompt must never be dropped"
    );

    // Verify no orphaned ToolCall exists without an immediately following ToolResult (or prompt for last).
    for (idx, msg) in patched.iter().enumerate() {
        let is_call = matches!(
            msg,
            Message::Assistant { content, .. }
                if content.iter().any(|c| matches!(c, AssistantContent::ToolCall(_)))
        );
        if is_call && idx + 1 < patched.len() {
            let next_is_result = matches!(
                &patched[idx + 1],
                Message::User { content }
                    if content.iter().any(|c| matches!(c, UserContent::ToolResult(_)))
            );
            assert!(next_is_result, "ToolCall at {idx} was orphaned");
        }
    }
}

#[test]
fn rag_and_judge_builders_fit_inside_1024_token_window() {
    let mut cfg = LlmConfig {
        max_context_tokens: 1024,
        ..Default::default()
    };

    let long_context =
        "Email body paragraph with details about Q1 milestone and French accents éàù. ".repeat(150);
    let history = vec![
        ChatTurn {
            role: "user".into(),
            content: "Previous question?".into(),
        },
        ChatTurn {
            role: "assistant".into(),
            content: "Previous answer.".into(),
        },
    ];
    let question = "What was decided for Q1?";
    let (rag_preamble, fitted_context, trimmed_hist, rag_max_out) =
        build_rag_request_budgets(&cfg, question, &long_context, &history);
    let max_out = rag_max_out.expect("finite max_context_tokens must set max_output") as usize;
    let rag_input_est = estimate_tokens(rag_preamble)
        + estimate_tokens(question)
        + estimate_tokens(&fitted_context)
        + trimmed_hist
            .iter()
            .map(estimate_message_tokens)
            .sum::<usize>()
        + 40;
    assert!(
        rag_input_est + max_out <= 1024,
        "RAG request {rag_input_est} + {max_out} exceeded 1024"
    );

    let email = EmailRecord {
        id: 1,
        source_key: "archive.pst".into(),
        message_id: "m1@example.test".into(),
        conversation_id: "conv1".into(),
        folder: "Inbox/Projects/Atlas".into(),
        subject: "Decision required on budget".into(),
        sender: "Alice <alice@example.test>".into(),
        recipients: "Bob <bob@example.test>".into(),
        sent_at: Some("2025-03-10T09:00:00Z".into()),
        body: "Very long email text with lots of context and details. ".repeat(300),
        attachment_names: String::new(),
        labels: Vec::new(),
    };
    let defs: Vec<LabelDefinition> = (1..=6)
        .map(|i| LabelDefinition {
            name: format!("Label{i}"),
            description:
                "Detailed criteria explaining when this label applies to business emails. "
                    .repeat(4),
            color: "#3b82f6".into(),
        })
        .collect();

    let (judge_preamble, judge_prompt, judge_max_out) =
        build_judge_prompt_and_max_tokens(&cfg, &email, &defs);
    let j_max_out = judge_max_out.expect("finite window must set judge max_output") as usize;
    let j_input_est = estimate_tokens(judge_preamble) + estimate_tokens(&judge_prompt) + 20;
    assert!(
        j_input_est + j_max_out <= 1024,
        "Judge request {j_input_est} + {j_max_out} exceeded 1024"
    );

    // Unlimited (0) leaves max_tokens unset
    cfg.max_context_tokens = 0;
    let (_, _, _, rag_unlim_out) =
        build_rag_request_budgets(&cfg, question, &long_context, &history);
    let (_, _, judge_unlim_out) = build_judge_prompt_and_max_tokens(&cfg, &email, &defs);
    assert_eq!(rag_unlim_out, None);
    assert_eq!(judge_unlim_out, None);
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
