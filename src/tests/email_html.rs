use crate::{
    app::email_html::{
        format_email_body_html, format_plain_text_email_html, html_to_plain_text, is_html_email,
        sanitize_html_email,
    },
    llm::clean_email_body,
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

    assert!(sanitized.contains("Hello <strong>Team</strong>"));
    assert!(sanitized.contains("text-align: center; font-weight: bold"));
    assert!(sanitized.contains(
        r#"<a href="https://example.com/report?id=1&amp;lang=fr" class="email-link" target="_blank" rel="noopener noreferrer">View Report</a>"#
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
