use serde::{Deserialize, Serialize};

use crate::model::SearchHit;

pub const AGENT_SYSTEM_PROMPT: &str = r#"You are Dungbeetle, an expert email archive intelligence assistant.
Your task is to answer user questions accurately, objectively, and concisely using ONLY the provided email evidence.

Key Instructions:
1. Grounding & Citations:
   - Base all answers strictly on the email evidence provided. Never hallucinate facts, dates, or assume information not present in the emails.
   - Always cite your sources using bracketed citations like [Email 1], [Email 2] for every substantive claim, quote, decision, or action item.
   - If multiple emails corroborate or discuss a point, cite all relevant emails (e.g., [Email 1, Email 3]).

2. Chronology & Evolution:
   - Pay close attention to email dates and chronological progression. Discussions evolve over time, and later emails often revise, supersede, or finalize proposals made in earlier messages.
   - Explicitly note timelines and sequence when answering questions about status, plans, or outcomes (e.g., "Initially scheduled for May in [Email 1], but postponed to June in [Email 2]").

3. Metadata & Context Awareness:
   - Leverage metadata (From, To, Subject, Folder, Archive, Attachments, and Labels such as Urgent or Action Required) when relevant.
   - If the user asks about files, agreements, or attachments, check the Attachments metadata.
   - Identify senders and recipients clearly.

4. Formatting & Synthesis:
   - Begin with a direct, synthesized summary answer.
   - Use structured bullet points, clear headings, or chronological summaries for complex topics.
   - Distinguish clearly between concrete decisions, open questions, and pending action items.

5. Evidence Limits:
   - If the provided email evidence is insufficient, incomplete, or contradictory, explicitly state what is known and what cannot be determined from the available records.
"#;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatTurn {
    pub role: String,
    pub content: String,
}

pub fn clean_email_body(raw: &str) -> String {
    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized.lines().map(|l| l.trim_end()).collect();
    if lines.is_empty() {
        return String::new();
    }

    let mut out_lines: Vec<String> = Vec::with_capacity(lines.len());
    let mut consecutive_empty = 0;
    let mut in_deep_quote = false;
    let mut in_extended_quote = false;
    let mut quote_lines_count = 0;

    for (line_idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        // 1. Detect legal disclaimer footers (especially in the bottom portion of the message)
        if line_idx > lines.len().saturating_sub(25) || line_idx > lines.len() / 2 {
            let lower = trimmed.to_lowercase();
            if lower.starts_with("confidentiality notice")
                || lower.starts_with("this e-mail and any attachments")
                || lower.starts_with("this email and any attachments")
                || lower.starts_with("this email, including any attachments")
                || lower.starts_with("this message contains confidential information")
                || lower.starts_with("this communication is intended only for")
                || lower.starts_with("the information contained in this email")
                || lower.starts_with("privileged/confidential information")
                || lower.starts_with("please consider the environment before printing")
                || lower.starts_with("this message is for the designated recipient only")
            {
                out_lines.push("[Legal disclaimer omitted]".into());
                break;
            }
        }

        // 2. Normalize horizontal divider lines
        if trimmed.len() >= 10
            && (trimmed.chars().all(|c| c == '-')
                || trimmed.chars().all(|c| c == '=')
                || trimmed.chars().all(|c| c == '_'))
        {
            out_lines.push("---".into());
            consecutive_empty = 0;
            continue;
        }

        // 3. Compact deep nested quotes (> > >) and extended quote chains
        let is_quote_line = trimmed.starts_with('>');
        if is_quote_line {
            if in_extended_quote {
                continue;
            }
            let depth = trimmed
                .chars()
                .take_while(|&c| c == '>' || c == ' ')
                .filter(|&c| c == '>')
                .count();
            if depth > 2 {
                if !in_deep_quote {
                    out_lines.push("[... deep nested quoted reply omitted ...]".into());
                    in_deep_quote = true;
                }
                continue;
            }
            in_deep_quote = false;
            quote_lines_count += 1;
            if quote_lines_count > 6 {
                out_lines.push("[... extended quoted reply chain condensed ...]".into());
                in_extended_quote = true;
                continue;
            }
        } else {
            in_deep_quote = false;
            in_extended_quote = false;
            quote_lines_count = 0;
        }

        // 4. Collapse consecutive empty lines
        if trimmed.is_empty() {
            consecutive_empty += 1;
            if consecutive_empty <= 2 {
                out_lines.push(String::new());
            }
        } else {
            consecutive_empty = 0;
            out_lines.push(line.to_string());
        }
    }

    let result = out_lines.join("\n");
    result.trim().to_string()
}

pub fn build_context(hits: &[SearchHit], max_chars: usize) -> String {
    if hits.is_empty() {
        return "No relevant emails were found matching the query.".into();
    }

    // 1. Collect date span & archives for overview
    let mut dates: Vec<&str> = hits
        .iter()
        .filter_map(|h| h.sent_at.as_deref())
        .filter(|d| !d.is_empty())
        .collect();
    dates.sort();
    let date_summary = match (dates.first(), dates.last()) {
        (Some(first), Some(last)) if first == last => format!("Date: {first}"),
        (Some(first), Some(last)) => format!("Dates: {first} to {last}"),
        _ => "Dates: unknown".into(),
    };

    let mut archives: Vec<&str> = hits.iter().map(|h| h.archive_name()).collect();
    archives.sort();
    archives.dedup();
    let archive_summary = if archives.is_empty() {
        String::new()
    } else {
        format!(" | Archives: {}", archives.join(", "))
    };

    let overview = format!(
        "# Retrieved Email Evidence ({} messages | {}{})\n\
         Cite sources using [Email N] format. Take note of timestamps and chronological sequence.\n\n",
        hits.len(),
        date_summary,
        archive_summary
    );

    // 2. Prepare headers and cleaned bodies for each hit
    struct HitContextItem<'a> {
        header: String,
        cleaned_body: String,
        #[allow(dead_code)]
        hit: &'a SearchHit,
    }

    let mut items: Vec<HitContextItem> = Vec::with_capacity(hits.len());
    for (i, hit) in hits.iter().enumerate() {
        let mut header = format!(
            "[Email {}] (Relevance: Rank #{} | Score: {:.3})\n\
             Date: {}\n\
             From: {}\n\
             To: {}\n\
             Subject: {}\n\
             Folder: {}\n\
             Archive: {}\n",
            i + 1,
            i + 1,
            hit.score,
            hit.sent_at.as_deref().unwrap_or("unknown"),
            if hit.sender.is_empty() {
                "(unknown sender)"
            } else {
                &hit.sender
            },
            if hit.recipients.is_empty() {
                "(unknown recipients)"
            } else {
                &hit.recipients
            },
            if hit.subject.is_empty() {
                "(no subject)"
            } else {
                &hit.subject
            },
            if hit.folder.is_empty() {
                "(root)"
            } else {
                &hit.folder
            },
            hit.archive_name()
        );

        if !hit.message_id.trim().is_empty() {
            header.push_str(&format!("Message-ID: {}\n", hit.message_id.trim()));
        }

        if !hit.conversation_id.trim().is_empty() {
            header.push_str(&format!("Thread: {}\n", hit.conversation_id.trim()));
        }

        if !hit.attachment_names.trim().is_empty() {
            header.push_str(&format!("Attachments: {}\n", hit.attachment_names.trim()));
        }

        if !hit.labels.is_empty() {
            let label_list = hit
                .labels
                .iter()
                .map(|l| {
                    if let Some(conf) = l.confidence {
                        format!("{} ({:.0}%)", l.label, conf * 100.0)
                    } else {
                        l.label.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            header.push_str(&format!("Labels: {}\n", label_list));
        }

        header.push_str("Body:\n");
        let cleaned = clean_email_body(&hit.body);
        items.push(HitContextItem {
            header,
            cleaned_body: cleaned,
            hit,
        });
    }

    // 3. Multi-hit Fair Budget Allocation (water-filling across items)
    let min_body_bytes = 250;
    let mut count_to_include = items.len();
    while count_to_include > 1 {
        let headers_len = overview.len()
            + items[..count_to_include]
                .iter()
                .map(|it| it.header.len() + 2)
                .sum::<usize>();
        let budget = max_chars.saturating_sub(headers_len);
        if budget >= count_to_include * min_body_bytes {
            break;
        }
        count_to_include -= 1;
    }

    let active_items = &items[..count_to_include];
    let active_headers_len = overview.len()
        + active_items
            .iter()
            .map(|it| it.header.len() + 2)
            .sum::<usize>();
    let mut remaining_budget = max_chars.saturating_sub(active_headers_len);

    let mut quotas = vec![0usize; count_to_include];
    while remaining_budget > 0 {
        let needy_indices: Vec<usize> = active_items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| (quotas[i] < item.cleaned_body.len()).then_some(i))
            .collect();
        if needy_indices.is_empty() {
            break;
        }
        let share = (remaining_budget / needy_indices.len()).max(1);
        let mut allocated_this_round = 0usize;
        for i in needy_indices {
            if remaining_budget == 0 {
                break;
            }
            let needed = active_items[i].cleaned_body.len() - quotas[i];
            let add = needed.min(share).min(remaining_budget);
            quotas[i] += add;
            remaining_budget -= add;
            allocated_this_round += add;
        }
        if allocated_this_round == 0 {
            break;
        }
    }
    if remaining_budget > 0 && !quotas.is_empty() {
        let bonus = remaining_budget / quotas.len();
        for q in &mut quotas {
            *q += bonus;
        }
    }

    // 4. Assemble context string within max_chars byte budget
    let mut out = overview;
    for (i, item) in active_items.iter().enumerate() {
        if out.len() + item.header.len() + 2 > max_chars {
            break;
        }
        out.push_str(&item.header);

        let remaining_for_item = max_chars.saturating_sub(out.len() + 2);
        let quota = quotas[i].min(remaining_for_item);
        let body_bytes = item.cleaned_body.len();
        if body_bytes <= quota {
            out.push_str(&item.cleaned_body);
        } else {
            let footer_reserve = 48;
            let target_bytes = quota.saturating_sub(footer_reserve);
            let mut kept_bytes = 0usize;
            for ch in item.cleaned_body.chars() {
                let ch_len = ch.len_utf8();
                if kept_bytes + ch_len > target_bytes {
                    break;
                }
                kept_bytes += ch_len;
            }
            let truncated_slice = &item.cleaned_body[..kept_bytes];
            let omitted_chars = item.cleaned_body[kept_bytes..].chars().count();
            let footer = format!("\n[... truncated ({omitted_chars} chars omitted) ...]");
            out.push_str(truncated_slice);
            if out.len() + footer.len() + 2 <= max_chars {
                out.push_str(&footer);
            }
        }
        out.push_str("\n\n");
    }

    out
}

pub fn contextualize_search_query(query: &str, history: &[ChatTurn]) -> String {
    let trimmed = query.trim();
    if history.is_empty() {
        return trimmed.to_string();
    }

    let words: Vec<&str> = trimmed.split_whitespace().collect();
    let is_short = words.len() <= 4;
    let lower = trimmed.to_lowercase();
    let has_pronoun = lower.contains(" that")
        || lower.starts_with("that")
        || lower.contains(" this")
        || lower.starts_with("this")
        || lower.contains(" it")
        || lower.starts_with("it")
        || lower.contains(" they")
        || lower.starts_with("they")
        || lower.contains(" he")
        || lower.starts_with("he ")
        || lower.contains(" she")
        || lower.starts_with("she ")
        || lower.contains(" there")
        || lower.contains(" then")
        || lower.starts_with("when was")
        || lower.starts_with("who was")
        || lower.starts_with("who sent")
        || lower.starts_with("what was");

    if !is_short && !has_pronoun {
        return trimmed.to_string();
    }

    if let Some(prev_user) = history
        .iter()
        .rev()
        .find(|t| t.role.eq_ignore_ascii_case("user"))
    {
        let prev_clean = prev_user
            .content
            .replace(['?', '.', ',', '!', '"', '\'', ';', ':'], " ");
        let stopwords = [
            "the",
            "a",
            "an",
            "is",
            "are",
            "was",
            "were",
            "what",
            "when",
            "where",
            "who",
            "which",
            "how",
            "can",
            "could",
            "would",
            "should",
            "find",
            "show",
            "tell",
            "about",
            "all",
            "any",
            "email",
            "emails",
            "message",
            "messages",
            "in",
            "on",
            "at",
            "to",
            "from",
            "for",
            "of",
            "my",
            "our",
            "your",
            "me",
            "summarize",
            "list",
            "do",
            "does",
            "did",
        ];
        let key_terms: Vec<&str> = prev_clean
            .split_whitespace()
            .filter(|w| w.len() >= 3 && !stopwords.contains(&w.to_lowercase().as_str()))
            .take(3)
            .collect();
        if !key_terms.is_empty() {
            let extra = key_terms.join(" ");
            if !lower.contains(&extra.to_lowercase()) {
                return format!("{trimmed} {extra}");
            }
        }
    }

    trimmed.to_string()
}
