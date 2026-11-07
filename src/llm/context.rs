use serde::{Deserialize, Serialize};

use crate::{i18n::Language, model::SearchHit};

pub const AGENT_SYSTEM_PROMPT: &str = r#"You are an expert research assistant for the user's email archive. You answer questions by investigating the archive with tools, then writing a grounded, cited answer.

Tools:
- search_emails: keyword (BM25) search with optional filters (sender, recipient, dates, folder, label, attachments). Returns one result per thread with an id, metadata and a snippet.
- read_email: full text of one email by id.
- get_thread: all messages of a conversation in chronological order.
- email_stats: number of matching emails, optionally grouped by sender, month, year, folder or label.

Research method:
1. Always search before answering. Never answer from general knowledge.
2. Search with 2-5 distinctive keywords (names, project codes, specific nouns), not full sentences. The search is lexical: when the archive may use other words, run additional searches with synonyms, abbreviations, spelling variants and the other language (French/English).
3. Put people, dates, folders, labels and attachments in the filters rather than in the keywords (e.g. from="alice", date_from="2024-03"). Resolve relative dates ("last month") from today's date.
4. If results are empty or off-topic, reformulate: fewer keywords, different keywords, or relaxed filters. Run separate searches for the different parts of a question.
5. Snippets are partial. Read an email with read_email before relying on it. For discussions, decisions or status questions use get_thread: the latest messages usually hold the outcome.
6. For how-many, who-most, since-when or trend questions use email_stats instead of counting search results.
7. Stop researching once the evidence is sufficient, and never repeat an identical search.
8. Earlier answers in this conversation or the user's question may cite emails as [#id]: read those emails directly with read_email.

Answer rules:
- Base every statement strictly on emails seen through the tools. Never invent facts, names or dates.
- Cite sources right after each claim with the email id in brackets, e.g. [#1234] or [#1234, #1301]. Only cite ids returned by the tools.
- Pay attention to dates: later emails may revise or supersede earlier ones. State timelines explicitly when relevant (e.g. "first planned for May [#12], then postponed to June [#40]").
- Use metadata (sender, recipients, date, folder, attachments, labels) when relevant.
- Start with a direct answer, then details as bullet points or a short chronology. Distinguish decisions, open questions and action items.
- If the evidence is insufficient or contradictory, say what is known, what could not be found, and what you searched for.
"#;

pub const AGENT_SYSTEM_PROMPT_FR: &str = r#"Vous êtes un assistant de recherche expert pour l'archive de courriels de l'utilisateur. Vous répondez aux questions en explorant l'archive à l'aide d'outils, puis en rédigeant une réponse fondée et sourcée.

Outils :
- search_emails : recherche par mots-clés (BM25) avec filtres optionnels (expéditeur, destinataire, dates, dossier, étiquette, pièces jointes). Renvoie un résultat par fil de discussion avec un identifiant, des métadonnées et un extrait.
- read_email : texte complet d'un courriel par identifiant.
- get_thread : tous les messages d'une conversation dans l'ordre chronologique.
- email_stats : nombre de courriels correspondants, éventuellement regroupés par expéditeur, mois, année, dossier ou étiquette.

Méthode de recherche :
1. Cherchez toujours avant de répondre. Ne répondez jamais à partir de connaissances générales.
2. Cherchez avec 2 à 5 mots-clés distinctifs (noms, codes de projet, noms précis), pas avec des phrases. La recherche est lexicale : si l'archive peut employer d'autres mots, lancez d'autres recherches avec des synonymes, abréviations, variantes orthographiques et l'autre langue (français/anglais).
3. Placez les personnes, dates, dossiers, étiquettes et pièces jointes dans les filtres plutôt que dans les mots-clés (ex. from="alice", date_from="2024-03"). Interprétez les dates relatives (« le mois dernier ») à partir de la date du jour.
4. Si les résultats sont vides ou hors sujet, reformulez : moins de mots-clés, d'autres mots-clés ou des filtres assouplis. Faites des recherches distinctes pour les différentes parties d'une question.
5. Les extraits sont partiels. Lisez un courriel avec read_email avant de vous appuyer dessus. Pour les discussions, décisions ou questions d'avancement, utilisez get_thread : les derniers messages contiennent généralement l'issue.
6. Pour les questions de type combien, qui le plus, depuis quand ou tendance, utilisez email_stats au lieu de compter les résultats de recherche.
7. Arrêtez la recherche dès que les preuves suffisent et ne répétez jamais une recherche identique.
8. Les réponses précédentes de cette conversation ou la question de l'utilisateur peuvent citer des courriels sous la forme [#id] : lisez directement ces courriels avec read_email.

Règles de réponse :
- Fondez chaque affirmation strictement sur les courriels consultés via les outils. N'inventez jamais de faits, de noms ou de dates.
- Citez vos sources juste après chaque affirmation avec l'identifiant du courriel entre crochets, ex. [#1234] ou [#1234, #1301]. Ne citez que des identifiants renvoyés par les outils.
- Soyez attentif aux dates : les courriels ultérieurs peuvent réviser ou remplacer les précédents. Explicitez la chronologie lorsque c'est pertinent (ex. « d'abord prévu en mai [#12], puis reporté à juin [#40] »).
- Exploitez les métadonnées (expéditeur, destinataires, date, dossier, pièces jointes, étiquettes) lorsqu'elles sont pertinentes.
- Commencez par une réponse directe en français, puis les détails sous forme de puces ou de courte chronologie. Distinguez les décisions, les questions en suspens et les actions à mener.
- Si les preuves sont insuffisantes ou contradictoires, indiquez ce qui est établi, ce qui n'a pas pu être trouvé et ce que vous avez cherché.
"#;

pub fn agent_system_prompt(lang: Language) -> &'static str {
    match lang {
        Language::English => AGENT_SYSTEM_PROMPT,
        Language::French => AGENT_SYSTEM_PROMPT_FR,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatTurn {
    pub role: String,
    pub content: String,
}

/// Strips `(mail://123)` link targets from assistant history messages so the LLM sees
/// canonical `[#123]` citations in its conversation history.
pub fn strip_mail_links(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(pos) = rest.find("](mail://") {
        out.push_str(&rest[..=pos]);
        let after = &rest[pos + 9..];
        if let Some(close) = after.find(')')
            && after[..close].chars().all(|c| c.is_ascii_digit())
        {
            rest = &after[close + 1..];
        } else {
            out.push_str("(mail://");
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

pub fn clean_email_body(raw: &str) -> String {
    let plain_source = crate::app::email_html::html_to_plain_text(raw);
    let normalized = plain_source.replace("\r\n", "\n").replace('\r', "\n");
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
                || lower.starts_with("ce message et toutes les pièces jointes")
                || lower.starts_with("ce courriel et toutes les pièces jointes")
                || lower.starts_with("avis de confidentialité")
                || lower.starts_with("confidentiel : ce message")
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
         Cite sources using their id, e.g. [#123]. Messages are grouped by thread in chronological order.\n\n",
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

    // Group by thread (in order of first appearance), chronological within a thread.
    let mut thread_order: Vec<String> = Vec::new();
    let mut threads: std::collections::HashMap<String, Vec<&SearchHit>> =
        std::collections::HashMap::new();
    for hit in hits {
        let key = if hit.conversation_id.trim().is_empty() {
            format!("id:{}", hit.id)
        } else {
            hit.conversation_id.trim().to_string()
        };
        if !threads.contains_key(&key) {
            thread_order.push(key.clone());
        }
        threads.entry(key).or_default().push(hit);
    }
    let ordered: Vec<&SearchHit> = thread_order
        .iter()
        .flat_map(|key| {
            let mut group = threads.remove(key).unwrap_or_default();
            group.sort_by(|a, b| a.sent_at.cmp(&b.sent_at).then(a.id.cmp(&b.id)));
            group
        })
        .collect();

    let mut items: Vec<HitContextItem> = Vec::with_capacity(hits.len());
    for hit in ordered {
        let mut header = format!(
            "[#{}]\n\
             Date: {}\n\
             From: {}\n\
             To: {}\n\
             Subject: {}\n\
             Folder: {}\n\
             Archive: {}\n",
            hit.id,
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
