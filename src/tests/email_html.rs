use crate::{
    email_html::{
        decode_html_entities, format_email_body_html, format_email_datetime,
        format_plain_text_email_html, html_to_plain_text, is_html_email, parse_contact,
        parse_email_participants, repair_text_encoding, sanitize_html_email,
    },
    i18n::Language,
    llm::clean_email_body,
    pst::{decode_bytes, decode_rfc2047},
};

#[test]
fn detects_html_vs_plain_text_emails_accurately() {
    assert!(is_html_email(
        "<!DOCTYPE html><html><body><p>Hello</p></body></html>"
    ));
    assert!(is_html_email(
        "<div class=\"WordSection1\"><p>Status update</p></div>"
    ));
    assert!(is_html_email("Hello<br>World"));
    assert!(is_html_email("<table><tr><td>Cell</td></tr></table>"));

    // Plain text with angle-bracketed email addresses or URLs must NOT be misclassified as HTML
    assert!(!is_html_email(
        "From: Alice Smith <alice@example.com>\nTo: Bob <bob@example.com>\nSee <https://example.com>"
    ));
    assert!(!is_html_email("Plain text message with x < y and y > z."));
}

#[test]
fn sanitizes_html_emails_and_removes_scripts_styles_and_trackers() {
    let raw_html = r#"<!DOCTYPE html>
<html>
<head>
  <style>body { background: #ffffff; color: #000000; }</style>
  <script>alert('xss');</script>
</head>
<body style="background-color: white; color: black;">
  <!-- [if mso]><v:shape>Ignored</v:shape><![endif] -->
  <div style="color: #000000; text-align: center; font-weight: bold;">
    <p>Hello <strong>Team</strong>,<o:p></o:p></p>
    <a href="https://example.com/report?id=1&amp;lang=fr" onclick="steal()">View Report</a>
    <a href="javascript:alert(1)">Bad Link</a>
    <img src="https://tracker.example.com/pixel.gif" width="1" height="1">
    <img src="https://tracker.example.com/banner.png" alt="External Banner">
    <img src="data:image/png;base64,iVBORw0KGgo=" alt="Inline Logo">
    <img src="cid:image001.png@01DA8F20.123456" alt="Architecture Diagram">
  </div>
</body>
</html>"#;

    let sanitized = sanitize_html_email(raw_html);
    assert!(!sanitized.contains("<script"));
    assert!(!sanitized.contains("alert("));
    assert!(!sanitized.contains("<style"));
    assert!(!sanitized.contains("<o:p"));
    assert!(!sanitized.contains("onclick"));
    assert!(!sanitized.contains("javascript:"));
    assert!(!sanitized.contains("background-color"));
    assert!(!sanitized.contains("pixel.gif"));
    assert!(!sanitized.contains("banner.png"));

    assert!(sanitized.contains("Hello <strong>Team</strong>"));
    assert!(sanitized.contains("text-align: center; font-weight: bold"));
    assert!(sanitized.contains(
        r#"<a href="https://example.com/report?id=1&amp;lang=fr" class="email-link" target="_blank" rel="noopener noreferrer">View Report</a>"#
    ));
    assert!(sanitized.contains(
        r#"<img src="data:image/png;base64,iVBORw0KGgo=" class="email-inline-img" loading="lazy" alt="Inline Logo">"#
    ));
    assert!(sanitized.contains(r#"<span class="email-cid-chip">📎 Architecture Diagram</span>"#));
}

#[test]
fn formats_plain_text_emails_with_reply_headers_quotes_lists_and_links() {
    let raw_plain = "\
Hi everyone,

Please review the milestones at https://portal.example.com/milestones.
Contact alice@example.com if you have questions:
- Finalize schema migration
- Run staging benchmarks

1. Deploy service
2. Verify metrics

On Mon, Mar 10, 2025 at 10:00 AM Bob wrote:
> Looks good to me.
> > Agreed on the timeline.

-----Original Message-----
From: Clara <clara@example.com>
Sent: Sunday, March 9, 2025 4:15 PM
To: Engineering Team
Subject: Q2 Planning

Initial kickoff notes.

--
Alice Martin
Staff Engineer

CONFIDENTIALITY NOTICE: This email and any attachments are confidential.";

    let html = format_plain_text_email_html(raw_plain);

    // Auto-linked URL and mailto
    assert!(html.contains(
        r#"<a class="email-link" href="https://portal.example.com/milestones" target="_blank" rel="noopener noreferrer">https://portal.example.com/milestones</a>."#
    ));
    assert!(html.contains(
        r#"<a class="email-address-link" href="mailto:alice@example.com">alice@example.com</a>"#
    ));

    // Unordered and ordered lists
    assert!(html.contains("<ul class=\"email-list\"><li>Finalize schema migration</li><li>Run staging benchmarks</li></ul>"));
    assert!(
        html.contains(
            "<ol class=\"email-list\"><li>Deploy service</li><li>Verify metrics</li></ol>"
        )
    );

    // Quote attribution & nested blockquotes
    assert!(html.contains(
        "<div class=\"email-quote-attribution\">On Mon, Mar 10, 2025 at 10:00 AM Bob wrote:</div>"
    ));
    assert!(html.contains("<blockquote class=\"email-quote\"><p class=\"email-paragraph\">Looks good to me.</p><blockquote class=\"email-quote\"><p class=\"email-paragraph\">Agreed on the timeline.</p></blockquote></blockquote>"));

    // Structured reply header card
    assert!(html.contains("<div class=\"email-reply-header\">"));
    assert!(html.contains("<div class=\"email-reply-header-title\">Original Message</div>"));
    assert!(html.contains("<span class=\"email-reply-key\">From</span>"));
    assert!(html.contains("<span class=\"email-reply-key\">Subject</span><span class=\"email-reply-val\">Q2 Planning</span>"));

    // Signature & legal disclaimer
    assert!(html.contains("<div class=\"email-signature\">Alice Martin<br>Staff Engineer</div>"));
    assert!(html.contains("<div class=\"email-disclaimer\">CONFIDENTIALITY NOTICE:"));
}

#[test]
fn converts_html_email_to_plain_text_for_llm_and_excerpts() {
    let raw_html = "<html><head><style>.p{color:red;}</style></head><body><p>Hello &amp; welcome,</p><ul><li>First item</li><li>Second item</li></ul><p>CONFIDENTIALITY NOTICE: This email and any attachments are confidential.</p></body></html>";
    let plain = html_to_plain_text(raw_html);
    assert!(plain.contains("Hello & welcome,"));
    assert!(plain.contains("- First item"));
    assert!(plain.contains("- Second item"));
    assert!(!plain.contains("<style>"));

    let cleaned = clean_email_body(raw_html);
    assert!(cleaned.contains("Hello & welcome,"));
    assert!(cleaned.contains("[Legal disclaimer omitted]"));

    let formatted = format_email_body_html(raw_html);
    assert!(formatted.contains("<p>Hello &amp; welcome,</p>"));
}

#[test]
fn parses_email_participants_from_to_and_cc_fields() {
    let sender = parse_contact("\"Alice Martin\" <alice.martin@example.com>");
    assert_eq!(sender.name.as_deref(), Some("Alice Martin"));
    assert_eq!(sender.email.as_deref(), Some("alice.martin@example.com"));
    assert_eq!(sender.display_primary(), "Alice Martin");
    assert_eq!(sender.initials(), "AM");

    let bare = parse_contact("ops-team@example.com");
    assert_eq!(bare.name, None);
    assert_eq!(bare.email.as_deref(), Some("ops-team@example.com"));
    assert_eq!(bare.display_primary(), "ops-team@example.com");
    assert_eq!(bare.initials(), "OT");

    let participants = parse_email_participants(
        "Alice Martin <alice@example.com>",
        "Bob Dupont <bob@example.com>; Clara <clara@example.com>\nCc: David Laurent <david@example.com>, Eve <eve@example.com>",
        "Hi team",
    );
    let from = participants.from.expect("sender should be parsed");
    assert_eq!(from.name.as_deref(), Some("Alice Martin"));
    assert_eq!(from.email.as_deref(), Some("alice@example.com"));
    assert_eq!(participants.to.len(), 2);
    assert_eq!(participants.to[0].display_primary(), "Bob Dupont");
    assert_eq!(participants.to[1].display_primary(), "Clara");
    assert_eq!(participants.cc.len(), 2);
    assert_eq!(participants.cc[0].display_primary(), "David Laurent");
    assert_eq!(participants.cc[1].email.as_deref(), Some("eve@example.com"));

    // Fallback extraction of Cc from inline headers when not stored in `to`
    let fallback_participants = parse_email_participants(
        "Alice <alice@example.com>",
        "Bob <bob@example.com>",
        "From: Alice <alice@example.com>\nTo: Bob <bob@example.com>\nCc: Frank Moreau <frank@example.com>\nSubject: Update\n\nBody text",
    );
    assert_eq!(fallback_participants.cc.len(), 1);
    assert_eq!(
        fallback_participants.cc[0].display_primary(),
        "Frank Moreau"
    );
    assert_eq!(
        fallback_participants.cc[0].email.as_deref(),
        Some("frank@example.com")
    );
}

#[test]
fn formats_email_dates_and_times_in_french_and_english() {
    let rfc3339 = "2025-03-10T14:32:00Z";

    let fr_fn = format_email_datetime;
    let fr = fr_fn(Some(rfc3339), Language::French).expect("should format FR");
    assert_eq!(fr.date_part, "Lun. 10 mars 2025");
    assert_eq!(fr.time_part, "14:32");
    assert_eq!(fr.table_short, "10 mars 2025 · 14:32");
    assert_eq!(fr.full, "Lun. 10 mars 2025 à 14:32");

    let en = fr_fn(Some(rfc3339), Language::English).expect("should format EN");
    assert_eq!(en.date_part, "Mon, Mar 10, 2025");
    assert_eq!(en.time_part, "14:32");
    assert_eq!(en.table_short, "Mar 10, 2025 · 14:32");
    assert_eq!(en.full, "Mon, Mar 10, 2025 at 14:32");

    // SQL / ISO without timezone
    let sql_ts = "2025-12-24 09:05:45";
    let fr_sql = fr_fn(Some(sql_ts), Language::French).expect("should format SQL");
    assert_eq!(fr_sql.date_part, "Mer. 24 déc. 2025");
    assert_eq!(fr_sql.time_part, "09:05");
}

#[test]
fn decodes_french_html_entities_and_win1252_numeric_refs() {
    let raw = "R&eacute;union &agrave; Paris : l&rsquo;&oelig;uvre d&rsquo;&Eacute;milie (&laquo;&nbsp;caf&eacute; &agrave; 5&nbsp;&euro;&nbsp;&raquo;) &#146;ok&#146; &#128; &#156;";
    let decoded = decode_html_entities(raw);
    assert_eq!(
        decoded,
        "Réunion à Paris : l’œuvre d’Émilie (« café à 5 € ») ’ok’ € œ"
    );

    let html = r#"<html><body><p>Bonjour &agrave; tous, r&eacute;sum&eacute; ci-joint.</p><img src="cid:doc1" alt="Pi&egrave;ce jointe"></body></html>"#;
    let plain = html_to_plain_text(html);
    assert_eq!(plain, "Bonjour à tous, résumé ci-joint.");

    let sanitized = sanitize_html_email(html);
    assert!(sanitized.contains("📎 Pièce jointe"));
}

#[test]
fn repairs_mojibake_and_c1_controls_without_corrupting_valid_french() {
    let valid_french = "Voilà l’été à Noël — Café © 2025 « Bonjour » Ça va bien ?";
    assert_eq!(repair_text_encoding(valid_french), valid_french);

    // Classic UTF-8 bytes decoded as Latin-1 (e.g. outlook-pst String8::to_string())
    let utf8_src = "Réunion à l’école — Été € œ";
    let utf8_as_latin1: String = utf8_src.as_bytes().iter().map(|&b| char::from(b)).collect();
    assert_eq!(repair_text_encoding(&utf8_as_latin1), utf8_src);

    // Windows-1252 bytes 0x80..=0x9F decoded as Latin-1 C1 control chars
    let win1252_c1 = "L\u{0092}offre : 50 \u{0080} \u{0096} chef-d\u{0092}\u{009C}uvre";
    assert_eq!(
        repair_text_encoding(win1252_c1),
        "L’offre : 50 € – chef-d’œuvre"
    );

    // Mixed string (valid UTF-8 header + mojibake body)
    let mixed = format!("À: Équipe\n{}", utf8_as_latin1);
    assert_eq!(
        repair_text_encoding(&mixed),
        format!("À: Équipe\n{utf8_src}")
    );
}

#[test]
fn decodes_pst_bytes_across_utf8_win1252_iso8859_and_rfc2047() {
    // Windows-1252 HTML body with French accents and smart quotes (0xE9, 0xE0, 0xE8, 0xE7, 0x92, 0x80)
    let mut win1252_html = b"<html><head><meta http-equiv=\"Content-Type\" content=\"text/html; charset=windows-1252\"></head><body><p>R".to_vec();
    win1252_html.extend_from_slice(&[0xE9]); // é
    win1252_html.extend_from_slice(b"union ");
    win1252_html.extend_from_slice(&[0xE0]); // à
    win1252_html.extend_from_slice(b" Paris : ");
    win1252_html.extend_from_slice(b"l");
    win1252_html.extend_from_slice(&[0x92]); // ’
    win1252_html.extend_from_slice(&[0xE9, 0x74, 0xE9]); // été
    win1252_html.extend_from_slice(b" ");
    win1252_html.extend_from_slice(&[0xE7, 0x61]); // ça
    win1252_html.extend_from_slice(b" co");
    win1252_html.extend_from_slice(&[0xFB]); // û
    win1252_html.extend_from_slice(b"te 5 ");
    win1252_html.extend_from_slice(&[0x80]); // €
    win1252_html.extend_from_slice(b"</p></body></html>");

    let decoded_html = decode_bytes(&win1252_html, Some(1252), true);
    assert!(decoded_html.contains("Réunion à Paris : l’été ça coûte 5 €"));
    assert!(!decoded_html.contains(char::REPLACEMENT_CHARACTER));

    // UTF-8 bytes stored inside String8 even with Windows-1252 codepage
    let utf8_bytes = "Boîte de réception — Éléments envoyés".as_bytes();
    assert_eq!(
        decode_bytes(utf8_bytes, Some(1252), false),
        "Boîte de réception — Éléments envoyés"
    );

    // ISO-8859-15 (codepage 28605): 0xA4 = €, 0xBD = œ, 0xE9 = é
    let iso15 = [
        b"C".as_slice(),
        &[0xF4],
        b"t",
        &[0xE9],
        b" ",
        &[0xBD],
        b"uf 3 ",
        &[0xA4],
    ]
    .concat();
    assert_eq!(decode_bytes(&iso15, Some(28605), false), "Côté œuf 3 €");

    // UTF-16LE with BOM
    let mut utf16 = vec![0xFFu8, 0xFE];
    for unit in "Café à Noël".encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    assert_eq!(decode_bytes(&utf16, None, false), "Café à Noël");

    // RFC 2047 encoded-words in subjects / headers
    assert_eq!(
        decode_rfc2047("Re: =?ISO-8859-1?Q?R=E9union_=E0_Paris?="),
        "Re: Réunion à Paris"
    );
    assert_eq!(
        decode_rfc2047("=?UTF-8?B?Q2Fmw6kgw6AgbOKAmcOpY29sZQ==?="),
        "Café à l’école"
    );
}

#[test]
fn sanitizer_blocks_self_closing_script_svg_data_uri_and_preserves_angle_comparisons() {
    let tricky = concat!(
        r#"<p>1 < 2 and 3 > 2</p>"#,
        r#"<script />alert('xss1');</script>"#,
        r#"<svg onload="alert(2)"><circle /></svg>"#,
        r#"<svg />"#,
        r#"<img src="data:image/svg+xml;base64,PHN2Zz4=" alt="BadSVG">"#,
        r#"<img src="data:image/webp;base64,UklGRg==" alt="GoodWebP">"#,
        r#"<p>After SVG</p>"#,
    );
    let sanitized = sanitize_html_email(tricky);
    assert!(
        sanitized.contains("<p>1 &lt; 2 and 3 &gt; 2</p>"),
        "{sanitized}"
    );
    assert!(!sanitized.contains("alert("), "{sanitized}");
    assert!(!sanitized.contains("svg+xml"), "{sanitized}");
    assert!(
        sanitized.contains("data:image/webp;base64,UklGRg=="),
        "{sanitized}"
    );
    assert!(sanitized.contains("<p>After SVG</p>"), "{sanitized}");

    // Unclosed <script /> must not leak trailing text
    let unclosed = r#"<p>Safe</p><script />alert('unclosed')"#;
    let sanitized_unclosed = sanitize_html_email(unclosed);
    assert!(sanitized_unclosed.contains("<p>Safe</p>"));
    assert!(!sanitized_unclosed.contains("alert("));
}
