use crate::{
    llm::{
        ChatTurn, build_context, clean_email_body, contextualize_search_query, endpoint_for_log,
        openai_base_url, parse_judge_response, validate_endpoint,
    },
    model::{AssignedLabel, SearchHit, default_label_definitions},
};

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
    let context = build_context(&hits, 2500);

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
        ChatTurn {
            role: "user".into(),
            content: "Tell me about the database migration schedule.".into(),
        },
        ChatTurn {
            role: "assistant".into(),
            content: "The migration is planned for Q4 as discussed in [Email 1].".into(),
        },
    ];

    let q1 = "Who is the lead for Project Apollo?";
    assert_eq!(contextualize_search_query(q1, &history), q1);

    let q2 = "When was that?";
    let enriched = contextualize_search_query(q2, &history);
    assert!(enriched.starts_with("When was that?"));
    assert!(enriched.contains("database"));
    assert!(enriched.contains("migration"));
}
