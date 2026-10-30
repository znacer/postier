use crate::text::{
    TextLang, best_passages, detect_language, fold_diacritics, index_body, index_stems,
    query_stems, stem, strip_quoted_reply,
};

#[test]
fn stems_english_and_french_correctly() {
    assert_eq!(stem("postponed", TextLang::English), "postpon");
    assert_eq!(stem("postponing", TextLang::English), "postpon");
    assert_eq!(stem("postponement", TextLang::English), "postpon");

    assert_eq!(stem("facturation", TextLang::French), "factur");
    assert_eq!(stem("factures", TextLang::French), "factur");
    assert_eq!(stem("facturé", TextLang::French), "factur");

    let stems = query_stems("postponed");
    assert!(stems.contains(&"postpon".to_string()));

    let fr_stems = query_stems("factures");
    assert!(fr_stems.contains(&"factur".to_string()));
}

#[test]
fn detects_language_from_tokens() {
    let en_tokens = vec![
        "the".into(),
        "quarterly".into(),
        "report".into(),
        "is".into(),
        "ready".into(),
        "for".into(),
        "review".into(),
    ];
    assert_eq!(detect_language(&en_tokens), TextLang::English);

    let fr_tokens = vec![
        "bonjour".into(),
        "voici".into(),
        "le".into(),
        "rapport".into(),
        "pour".into(),
        "la".into(),
        "réunion".into(),
    ];
    assert_eq!(detect_language(&fr_tokens), TextLang::French);
}

#[test]
fn folds_diacritics_for_accent_insensitive_matching() {
    assert_eq!(fold_diacritics("Équipe"), "Equipe");
    assert_eq!(fold_diacritics("août"), "aout");
    assert_eq!(fold_diacritics("cœur"), "coeur");
    assert_eq!(fold_diacritics("façade"), "facade");
}

#[test]
fn strips_quoted_reply_chains_and_disclaimers() {
    let body = "\
Hi team,

The release is approved. Let's proceed at 14:00.

On Mon, 20 Oct 2026 at 10:30, Bob <bob@example.test> wrote:
> Should we approve the release today?
> We completed QA testing.

--
Alice Smith
VP Engineering
";
    let stripped = strip_quoted_reply(body);
    assert!(stripped.contains("The release is approved. Let's proceed at 14:00."));
    assert!(!stripped.contains("Should we approve the release today?"));
    assert!(!stripped.contains("VP Engineering"));
}

#[test]
fn strips_outlook_header_block_replies() {
    let body = "\
I agree with this plan.

-----Original Message-----
From: Charlie <charlie@example.test>
Sent: Tuesday, October 28, 2026 9:15 AM
To: Alice <alice@example.test>
Subject: Server maintenance

We need to reboot the host tomorrow morning.
";
    let stripped = strip_quoted_reply(body);
    assert!(stripped.contains("I agree with this plan."));
    assert!(!stripped.contains("Server maintenance"));
    assert!(!stripped.contains("reboot the host"));
}

#[test]
fn index_body_preserves_forwards() {
    let forward_subj = "FW: Important client agreement";
    let body = "\
FYI below.

-----Original Message-----
From: Legal <legal@example.test>
Subject: Contract terms

Attached are the finalized contract terms.
";
    let indexed = index_body(forward_subj, body);
    assert!(indexed.contains("Attached are the finalized contract terms."));
}

#[test]
fn index_stems_generates_stemmed_token_string() {
    let stems = index_stems("Project Atlas", "Deployment postponed until June");
    assert!(stems.contains("postpon"));
    assert!(stems.contains("deploy"));
    assert!(stems.contains("atlas"));
}

#[test]
fn best_passages_extracts_matching_segment_from_long_text() {
    let mut paragraphs = Vec::new();
    paragraphs.push("Introduction: General company overview and routine updates.");
    for i in 1..20 {
        paragraphs
            .push("Routine maintenance log line: Everything operating within standard parameters.");
    }
    paragraphs.push("Crucial decision: The Atlas project launch is strictly postponed until June 15 due to vendor delays.");
    for i in 1..10 {
        paragraphs.push("Follow-up housekeeping notices and parking reminders.");
    }
    let full_text = paragraphs.join("\n\n");

    let query_tokens = vec!["Atlas".to_string(), "postponed".to_string()];
    let passage = best_passages(&full_text, &query_tokens, 300);

    assert!(passage.contains("Atlas"));
    assert!(passage.contains("postponed"));
    assert!(passage.contains("June 15"));
    assert!(passage.chars().count() <= 320);
}
