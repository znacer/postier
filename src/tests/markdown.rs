use crate::app::markdown::{MdBlock, MdSpan, parse_inline, parse_markdown};

#[test]
fn parses_headings_lists_code_quotes_and_tables() {
    let md = "\
# Executive Summary

Here is the **key decision** from `Project Atlas`:

- [x] Budget approved by CFO
- [ ] Schedule kick-off meeting
  1. Invite engineering
  2. Prepare slides

> Launch is postponed until June.

| Item | Status |
| --- | --- |
| Budget | Approved |
| Launch | June |

```json
{\"status\": \"ok\"}
```
---
";
    let blocks = parse_markdown(md);
    assert!(matches!(
        &blocks[0],
        MdBlock::Heading { level: 1, text } if text == "Executive Summary"
    ));
    assert!(matches!(&blocks[1], MdBlock::Paragraph(_)));
    assert!(matches!(&blocks[2], MdBlock::List(items) if items.len() == 4));
    assert!(matches!(&blocks[3], MdBlock::BlockQuote(lines) if lines.len() == 1));
    assert!(matches!(
        &blocks[4],
        MdBlock::Table { headers, rows } if headers.len() == 2 && rows.len() == 2
    ));
    assert!(matches!(
        &blocks[5],
        MdBlock::CodeBlock { lang, code } if lang == "json" && code.contains("\"status\"")
    ));
    assert_eq!(blocks[6], MdBlock::Rule);
}

#[test]
fn parses_inline_bold_italic_code_strikethrough_and_links() {
    let spans =
        parse_inline("Check **bold**, *italic*, ~~old~~, `inline_code`, and [Mail 42](mail://42).");
    assert!(spans.contains(&MdSpan::Styled {
        text: "bold".into(),
        bold: true,
        italic: false,
        strike: false,
    }));
    assert!(spans.contains(&MdSpan::Styled {
        text: "italic".into(),
        bold: false,
        italic: true,
        strike: false,
    }));
    assert!(spans.contains(&MdSpan::Styled {
        text: "old".into(),
        bold: false,
        italic: false,
        strike: true,
    }));
    assert!(spans.contains(&MdSpan::Code("inline_code".into())));
    assert!(spans.contains(&MdSpan::Link {
        text: "Mail 42".into(),
        url: "mail://42".into(),
    }));
}

#[test]
fn parses_table_row_missing_trailing_pipe_without_empty_leading_cell() {
    let md = "| Item | Status\n| --- | ---\n| Budget | Approved";
    let blocks = parse_markdown(md);
    assert!(matches!(
        &blocks[0],
        MdBlock::Table { headers, rows }
            if headers == &vec!["Item".to_string(), "Status".to_string()]
            && rows.len() == 1
            && rows[0] == vec!["Budget".to_string(), "Approved".to_string()]
    ));
}
