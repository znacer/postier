use crate::{
    laya::LayaModel,
    model::{EmailRecord, default_label_definitions},
};

#[test]
fn packaged_laya_gguf_loads_and_labels_emails() {
    let model = LayaModel::embedded();
    let defs = default_label_definitions();

    let decision_mail = EmailRecord {
        id: 1,
        source_key: "test.pst".into(),
        message_id: "m1".into(),
        conversation_id: String::new(),
        folder: "Inbox".into(),
        subject: "Project Atlas launch decision approved".into(),
        sender: "director@example.test".into(),
        recipients: "team@example.test".into(),
        sent_at: Some("2026-03-10T09:00:00Z".into()),
        body: "We decided and approved the final launch resolution for June.".into(),
        attachment_names: String::new(),
        labels: Vec::new(),
    };
    let labels = model.label_email(&decision_mail, &defs).unwrap();
    assert!(!labels.is_empty());
    assert_eq!(labels[0].label, "Decision");
    assert!(labels[0].confidence.unwrap_or(0.0) > 0.1);

    let urgent_mail = EmailRecord {
        id: 2,
        source_key: "test.pst".into(),
        message_id: "m2".into(),
        conversation_id: String::new(),
        folder: "Inbox".into(),
        subject: "URGENT critical outage blocker".into(),
        sender: "ops@example.test".into(),
        recipients: "oncall@example.test".into(),
        sent_at: Some("2026-03-11T09:00:00Z".into()),
        body: "Immediate escalation required for this critical production outage today.".into(),
        attachment_names: String::new(),
        labels: Vec::new(),
    };
    let urgent_labels = model.label_email(&urgent_mail, &defs).unwrap();
    assert!(!urgent_labels.is_empty());
    assert_eq!(urgent_labels[0].label, "Urgent");
}
