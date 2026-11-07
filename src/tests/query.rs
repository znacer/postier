use std::path::Path;

use crate::{
    db::{
        Index,
        query::{StatsGroup, parse_query},
    },
    model::Email,
};

#[test]
fn parses_complex_queries_with_filters_and_phrases() {
    let q = r#"budget "Q3 forecast" from:alice after:2024-03 label:Urgent has:attachment"#;
    let parsed = parse_query(q);

    assert_eq!(parsed.terms, vec!["budget"]);
    assert_eq!(parsed.phrases, vec!["q3 forecast"]);
    assert_eq!(parsed.filters.from.as_deref(), Some("alice"));
    assert_eq!(parsed.filters.date_from.as_deref(), Some("2024-03"));
    assert_eq!(parsed.filters.label.as_deref(), Some("Urgent"));
    assert_eq!(parsed.filters.has_attachment, Some(true));

    let fts = parsed.fts_all().unwrap();
    assert!(fts.contains("\"q3 forecast\""));
    assert!(fts.contains("\"budget\""));
}

#[test]
fn parses_french_filter_aliases() {
    let q = "de:jean à:marie après:2025-01 avant:2025-06 dossier:Projets étiquette:Action avec:pj";
    let parsed = parse_query(q);

    assert_eq!(parsed.filters.from.as_deref(), Some("jean"));
    assert_eq!(parsed.filters.to.as_deref(), Some("marie"));
    assert_eq!(parsed.filters.date_from.as_deref(), Some("2025-01"));
    assert_eq!(parsed.filters.date_to.as_deref(), Some("2025-06"));
    assert_eq!(parsed.filters.folder.as_deref(), Some("Projets"));
    assert_eq!(parsed.filters.label.as_deref(), Some("Action"));
    assert_eq!(parsed.filters.has_attachment, Some(true));
}

#[test]
fn search_collapses_multiple_messages_in_thread_to_best_match() {
    let mut db = Index::open(Path::new(":memory:")).unwrap();
    db.index_batch(&[
        Email {
            subject: "Roadmap Discussion".into(),
            sender: "alice@example.test".into(),
            conversation_id: "thread-101".into(),
            sent_at: Some("2026-05-01T09:00:00Z".into()),
            body: "Let's review the product roadmap for Q3.".into(),
            ..Default::default()
        },
        Email {
            subject: "Re: Roadmap Discussion".into(),
            sender: "bob@example.test".into(),
            conversation_id: "thread-101".into(),
            sent_at: Some("2026-05-01T10:00:00Z".into()),
            body: "I reviewed the roadmap and approve the proposed schedule.".into(),
            ..Default::default()
        },
        Email {
            subject: "Another topic on roadmap".into(),
            sender: "carol@example.test".into(),
            conversation_id: "thread-102".into(),
            sent_at: Some("2026-05-02T11:00:00Z".into()),
            body: "Different project roadmap details.".into(),
            ..Default::default()
        },
    ])
    .unwrap();

    let parsed = parse_query("roadmap");
    let hits = db.search_emails(&parsed, 10).unwrap();

    // 2 distinct threads returned, not 3 raw messages!
    assert_eq!(hits.len(), 2);
    let thread101 = hits
        .iter()
        .find(|h| h.conversation_id == "thread-101")
        .unwrap();
    assert_eq!(thread101.thread_matches, 2);
}

#[test]
fn email_stats_aggregates_counts_and_groups() {
    let mut db = Index::open(Path::new(":memory:")).unwrap();
    db.index_batch(&[
        Email {
            subject: "Invoice #1".into(),
            sender: "vendorA@example.test".into(),
            sent_at: Some("2026-01-15T09:00:00Z".into()),
            folder: "Invoices".into(),
            body: "Payment due for services.".into(),
            ..Default::default()
        },
        Email {
            subject: "Invoice #2".into(),
            sender: "vendorA@example.test".into(),
            sent_at: Some("2026-01-20T09:00:00Z".into()),
            folder: "Invoices".into(),
            body: "Second invoice for hardware.".into(),
            ..Default::default()
        },
        Email {
            subject: "Invoice #3".into(),
            sender: "vendorB@example.test".into(),
            sent_at: Some("2026-02-10T09:00:00Z".into()),
            folder: "Invoices".into(),
            body: "Consulting fees invoice.".into(),
            ..Default::default()
        },
    ])
    .unwrap();

    // 1. Total count
    let parsed = parse_query("invoice");
    let (total, _) = db.email_stats(&parsed, None, 10).unwrap();
    assert_eq!(total, 3);

    // 2. Group by sender
    let (total_senders, sender_groups) = db
        .email_stats(&parsed, Some(StatsGroup::Sender), 10)
        .unwrap();
    assert_eq!(total_senders, 3);
    assert_eq!(sender_groups.len(), 2);
    assert_eq!(sender_groups[0].key, "vendorA@example.test");
    assert_eq!(sender_groups[0].count, 2);
    assert_eq!(sender_groups[1].key, "vendorB@example.test");
    assert_eq!(sender_groups[1].count, 1);

    // 3. Group by month (chronological order)
    let (_, month_groups) = db
        .email_stats(&parsed, Some(StatsGroup::Month), 10)
        .unwrap();
    assert_eq!(month_groups.len(), 2);
    assert_eq!(month_groups[0].key, "2026-01");
    assert_eq!(month_groups[0].count, 2);
    assert_eq!(month_groups[1].key, "2026-02");
    assert_eq!(month_groups[1].count, 1);
}

#[test]
fn get_thread_returns_chronological_messages_with_limit_preservation() {
    let mut db = Index::open(Path::new(":memory:")).unwrap();
    let thread_id = "thread-chronology";
    for i in 1..=5 {
        db.index_batch(&[Email {
            subject: format!("Discussion part {i}"),
            sender: format!("user{i}@example.test"),
            conversation_id: thread_id.into(),
            sent_at: Some(format!("2026-03-0{i}T10:00:00Z")),
            body: format!("Message content {i}"),
            ..Default::default()
        }])
        .unwrap();
    }

    let (total, messages) = db.get_thread(thread_id, 3).unwrap();
    assert_eq!(total, 5);
    // Limit is 3: first message preserved, plus the latest 2 messages
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].subject, "Discussion part 1");
    assert_eq!(messages[1].subject, "Discussion part 4");
    assert_eq!(messages[2].subject, "Discussion part 5");
}
