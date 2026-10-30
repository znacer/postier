use std::path::Path;

use crate::{
    db::Index,
    model::{AssignedLabel, Email, LabelFilter, MailScope, MailSort},
};

#[test]
fn indexes_deduplicates_and_searches() {
    let mut db = Index::open(Path::new(":memory:")).unwrap();
    let mail = Email {
        subject: "Project Atlas decision".into(),
        sender: "a@example.test".into(),
        body: "Launch postponed until June".into(),
        ..Default::default()
    };
    assert_eq!(db.index_batch(&[mail.clone(), mail]).unwrap(), 1);
    let hits = db.search("Atlas postponed", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].subject, "Project Atlas decision");
}

#[test]
fn stores_embeddings_by_message_and_model() {
    let mut db = Index::open(Path::new(":memory:")).unwrap();
    db.index_batch(&[
        Email {
            subject: "Alpha".into(),
            body: "first document".into(),
            ..Default::default()
        },
        Email {
            subject: "Beta".into(),
            body: "second document".into(),
            ..Default::default()
        },
    ])
    .unwrap();
    let missing = db.documents_missing_embeddings("embed-v1").unwrap();
    assert_eq!(missing.len(), 2);
    db.store_embeddings("embed-v1", &[(missing[0].id, vec![1.0, 0.0])])
        .unwrap();
    assert_eq!(
        db.documents_missing_embeddings("embed-v1").unwrap().len(),
        1
    );
    assert_eq!(
        db.documents_missing_embeddings("another-model")
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn hybrid_search_can_retrieve_a_semantic_only_match() {
    let mut db = Index::open(Path::new(":memory:")).unwrap();
    db.index_batch(&[
        Email {
            subject: "Exact keyword".into(),
            body: "quasar".into(),
            ..Default::default()
        },
        Email {
            subject: "Semantic result".into(),
            body: "meaning without the query token".into(),
            ..Default::default()
        },
    ])
    .unwrap();
    let documents = db.documents_missing_embeddings("embed-v1").unwrap();
    let exact_id = documents
        .iter()
        .find(|document| document.text.contains("Exact keyword"))
        .unwrap()
        .id;
    let semantic_id = documents
        .iter()
        .find(|document| document.text.contains("Semantic result"))
        .unwrap()
        .id;
    db.store_embeddings(
        "embed-v1",
        &[(exact_id, vec![0.0, 1.0]), (semantic_id, vec![1.0, 0.0])],
    )
    .unwrap();
    let hits = db
        .search_hybrid("quasar", &[1.0, 0.0], "embed-v1", 10)
        .unwrap();
    assert!(hits.iter().any(|hit| hit.id == semantic_id));
}

#[test]
fn manages_multiple_sources_and_folder_filters() {
    let mut db = Index::open(Path::new(":memory:")).unwrap();
    let first = Email {
        source_key: "/archives/first.pst".into(),
        folder: "Mailbox/Inbox".into(),
        subject: "Shared project".into(),
        body: "project lighthouse".into(),
        ..Default::default()
    };
    let second = Email {
        source_key: "/archives/second.pst".into(),
        ..first.clone()
    };
    assert_eq!(db.index_batch(&[first, second]).unwrap(), 2);
    assert_eq!(db.sources().unwrap().len(), 2);
    assert_eq!(db.search("lighthouse", 10).unwrap().len(), 2);

    db.set_source_enabled("/archives/first.pst", false).unwrap();
    let hits = db.search("lighthouse", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].source_key, "/archives/second.pst");

    db.set_folder_enabled("/archives/second.pst", "Mailbox/Inbox", false)
        .unwrap();
    assert!(db.search("lighthouse", 10).unwrap().is_empty());
    db.set_folder_enabled("/archives/second.pst", "Mailbox/Inbox", true)
        .unwrap();

    assert_eq!(db.remove_source("/archives/first.pst").unwrap(), 1);
    let sources = db.sources().unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].name, "second.pst");
    assert_eq!(db.count().unwrap(), 1);
}

#[test]
fn manages_labels_and_filters_explorer_messages() {
    let mut db = Index::open(Path::new(":memory:")).unwrap();
    db.index_batch(&[
        Email {
            source_key: "/archives/main.pst".into(),
            folder: "Mailbox/Inbox".into(),
            subject: "Approve Q3 budget".into(),
            sender: "cfo@example.test".into(),
            sent_at: Some("2026-03-01T10:00:00Z".into()),
            body: "Please approve the budget today.".into(),
            ..Default::default()
        },
        Email {
            source_key: "/archives/main.pst".into(),
            folder: "Mailbox/Archive".into(),
            subject: "Weekly digest".into(),
            sender: "news@example.test".into(),
            sent_at: Some("2026-03-02T10:00:00Z".into()),
            body: "General company updates.".into(),
            ..Default::default()
        },
    ])
    .unwrap();

    let all = db
        .list_messages(
            &MailScope::AllMessages,
            &LabelFilter::Any,
            "",
            MailSort::DateDesc,
            100,
        )
        .unwrap();
    assert_eq!(all.len(), 2);
    let budget_id = all
        .iter()
        .find(|m| m.subject.contains("Approve"))
        .unwrap()
        .id;

    db.set_message_labels(
        budget_id,
        &[AssignedLabel {
            label: "Action Required".into(),
            confidence: Some(0.93),
            source: "laya".into(),
        }],
    )
    .unwrap();

    let action_only = db
        .list_messages(
            &MailScope::AllMessages,
            &LabelFilter::Labeled("Action Required".into()),
            "",
            MailSort::DateDesc,
            100,
        )
        .unwrap();
    assert_eq!(action_only.len(), 1);
    assert_eq!(action_only[0].id, budget_id);

    let unlabeled = db
        .list_messages(
            &MailScope::AllMessages,
            &LabelFilter::Unlabeled,
            "",
            MailSort::DateDesc,
            100,
        )
        .unwrap();
    assert_eq!(unlabeled.len(), 1);
    assert_eq!(unlabeled[0].subject, "Weekly digest");

    let selected_ids: Vec<i64> = all.iter().map(|m| m.id).collect();
    assert_eq!(
        db.set_manual_label_for_messages(&selected_ids, "FYI", true)
            .unwrap(),
        2
    );
    let batch_records = db.get_messages_by_ids(&selected_ids).unwrap();
    assert_eq!(batch_records.len(), 2);
    assert!(
        batch_records
            .iter()
            .all(|m| m.labels.iter().any(|l| l.label == "FYI"))
    );
}

#[test]
fn search_populates_rich_metadata_labels_and_expands_threads() {
    let mut db = Index::open(Path::new(":memory:")).unwrap();
    db.index_batch(&[
        Email {
            source_key: "/archives/team.pst".into(),
            folder: "Mailbox/Inbox".into(),
            subject: "Server Migration".into(),
            sender: "lead@example.test".into(),
            recipients: "devs@example.test".into(),
            sent_at: Some("2026-04-01T09:00:00Z".into()),
            body: "We are migrating the database server tonight.".into(),
            attachment_names: "migration_guide.pdf".into(),
            conversation_id: "thread-migration-42".into(),
            message_id: "msg-001@example.test".into(),
        },
        Email {
            source_key: "/archives/team.pst".into(),
            folder: "Mailbox/Inbox".into(),
            subject: "Re: Server Migration".into(),
            sender: "ops@example.test".into(),
            recipients: "lead@example.test".into(),
            sent_at: Some("2026-04-01T10:00:00Z".into()),
            body: "Backup completed successfully. Ready to proceed.".into(),
            attachment_names: "backup_checksums.txt".into(),
            conversation_id: "thread-migration-42".into(),
            message_id: "msg-002@example.test".into(),
        },
    ])
    .unwrap();

    let records = db.search("migrating", 10).unwrap();
    assert_eq!(records.len(), 1);
    let id1 = records[0].id;
    db.set_message_labels(
        id1,
        &[AssignedLabel {
            label: "Urgent".into(),
            confidence: Some(0.99),
            source: "laya".into(),
        }],
    )
    .unwrap();

    let mut hits = db.search("migrating", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].score > 0.0);
    assert_eq!(hits[0].attachment_names, "migration_guide.pdf");
    assert_eq!(hits[0].conversation_id, "thread-migration-42");
    assert_eq!(hits[0].message_id, "msg-001@example.test");
    assert_eq!(hits[0].labels.len(), 1);
    assert_eq!(hits[0].labels[0].label, "Urgent");

    let (total, thread_messages) = db.get_thread("thread-migration-42", 10).unwrap();
    assert_eq!(total, 2);
    assert_eq!(thread_messages.len(), 2);
    assert!(
        thread_messages
            .iter()
            .any(|h| h.sender == "ops@example.test")
    );
    assert!(
        thread_messages
            .iter()
            .any(|h| h.attachment_names == "backup_checksums.txt")
    );
}
