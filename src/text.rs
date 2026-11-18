//! Text normalisation for the BM25 index and for query building.
//!
//! - [`index_body`] turns a raw (possibly HTML) email body into the plain text that is
//!   indexed: no markup, no quoted reply chains, no signatures.
//! - [`index_stems`] produces the Snowball-stemmed copy of the text stored in the
//!   `stems` FTS column so that `postponed` matches `postpone` (English and French).
//! - [`best_passages`] extracts the passages of a long email that match a query.

use std::{
    cmp::Reverse,
    collections::{BTreeSet, HashSet},
    sync::OnceLock,
};

use rust_stemmers::{Algorithm, Stemmer};

/// Words ignored when building queries and when filling the `stems` column.
const EN_STOPWORDS: &[&str] = &[
    "a",
    "about",
    "above",
    "after",
    "again",
    "all",
    "also",
    "am",
    "an",
    "and",
    "any",
    "are",
    "as",
    "at",
    "be",
    "been",
    "before",
    "being",
    "below",
    "between",
    "both",
    "but",
    "by",
    "can",
    "could",
    "did",
    "do",
    "does",
    "doing",
    "down",
    "during",
    "each",
    "few",
    "for",
    "from",
    "further",
    "had",
    "has",
    "have",
    "having",
    "he",
    "her",
    "here",
    "hers",
    "him",
    "his",
    "how",
    "i",
    "if",
    "in",
    "into",
    "is",
    "it",
    "its",
    "just",
    "me",
    "more",
    "most",
    "my",
    "no",
    "nor",
    "not",
    "now",
    "of",
    "off",
    "on",
    "once",
    "only",
    "or",
    "other",
    "our",
    "ours",
    "out",
    "over",
    "own",
    "same",
    "she",
    "should",
    "so",
    "some",
    "such",
    "than",
    "that",
    "the",
    "their",
    "theirs",
    "them",
    "then",
    "there",
    "these",
    "they",
    "this",
    "those",
    "through",
    "to",
    "too",
    "under",
    "until",
    "up",
    "very",
    "was",
    "we",
    "were",
    "what",
    "when",
    "where",
    "which",
    "while",
    "who",
    "whom",
    "why",
    "will",
    "with",
    "would",
    "you",
    "your",
    "yours",
    "please",
    "tell",
    "show",
    "find",
    "give",
    "list",
    "summarize",
    "summarise",
    "email",
    "emails",
    "mail",
    "mails",
    "message",
    "messages",
    "anything",
    "something",
    "know",
];

const FR_STOPWORDS: &[&str] = &[
    "a",
    "à",
    "afin",
    "ai",
    "aie",
    "ait",
    "as",
    "au",
    "aux",
    "avec",
    "avez",
    "avons",
    "avoir",
    "c",
    "ça",
    "car",
    "ce",
    "ceci",
    "cela",
    "celle",
    "celles",
    "celui",
    "ces",
    "cet",
    "cette",
    "ceux",
    "chaque",
    "comme",
    "comment",
    "d",
    "dans",
    "de",
    "des",
    "donc",
    "dont",
    "du",
    "elle",
    "elles",
    "en",
    "entre",
    "es",
    "est",
    "et",
    "été",
    "être",
    "eu",
    "eux",
    "fait",
    "faire",
    "il",
    "ils",
    "j",
    "je",
    "l",
    "la",
    "le",
    "les",
    "leur",
    "leurs",
    "lui",
    "m",
    "ma",
    "mais",
    "me",
    "même",
    "mes",
    "moi",
    "mon",
    "n",
    "ne",
    "ni",
    "nos",
    "notre",
    "nous",
    "on",
    "ont",
    "ou",
    "où",
    "par",
    "pas",
    "peu",
    "plus",
    "pour",
    "qu",
    "quand",
    "que",
    "quel",
    "quelle",
    "quelles",
    "quels",
    "qui",
    "quoi",
    "s",
    "sa",
    "sans",
    "se",
    "ses",
    "si",
    "son",
    "sont",
    "sur",
    "t",
    "ta",
    "te",
    "tes",
    "toi",
    "ton",
    "tous",
    "tout",
    "toute",
    "toutes",
    "très",
    "tu",
    "un",
    "une",
    "vos",
    "votre",
    "vous",
    "y",
    "courriel",
    "courriels",
    "résumer",
    "résume",
    "trouver",
    "trouve",
    "liste",
    "lister",
    "montre",
    "montrer",
    "dis",
    "dire",
    "quelque",
    "chose",
    "sais",
    "savoir",
];

/// Distinctive function words used to guess the language of a document.
const EN_MARKERS: &[&str] = &[
    "the", "and", "is", "are", "was", "were", "of", "to", "with", "for", "this", "that", "have",
    "has", "will", "you", "we", "not", "be", "from", "it", "please", "thanks",
];
const FR_MARKERS: &[&str] = &[
    "le", "la", "les", "des", "du", "et", "est", "sont", "pour", "avec", "dans", "une", "que",
    "qui", "nous", "vous", "pas", "sur", "au", "aux", "ce", "cette", "merci", "bonjour",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextLang {
    English,
    French,
}

fn stemmer(lang: TextLang) -> &'static Stemmer {
    static EN: OnceLock<Stemmer> = OnceLock::new();
    static FR: OnceLock<Stemmer> = OnceLock::new();
    match lang {
        TextLang::English => EN.get_or_init(|| Stemmer::create(Algorithm::English)),
        TextLang::French => FR.get_or_init(|| Stemmer::create(Algorithm::French)),
    }
}

pub fn is_stopword(token: &str) -> bool {
    EN_STOPWORDS.contains(&token) || FR_STOPWORDS.contains(&token)
}

/// Lowercased alphanumeric tokens, split the same way as SQLite's `unicode61` tokenizer.
pub fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect()
}

pub fn detect_language(tokens: &[String]) -> TextLang {
    let mut en = 0usize;
    let mut fr = 0usize;
    for token in tokens.iter().take(2_000) {
        let t = token.as_str();
        if EN_MARKERS.contains(&t) {
            en += 1;
        }
        if FR_MARKERS.contains(&t) {
            fr += 1;
        }
    }
    if fr > en {
        TextLang::French
    } else {
        TextLang::English
    }
}

pub fn stem(token: &str, lang: TextLang) -> String {
    stemmer(lang).stem(token).into_owned()
}

/// English and French stems of a query token (deduplicated, excluding the token itself).
pub fn query_stems(token: &str) -> Vec<String> {
    let mut out = Vec::new();
    for lang in [TextLang::English, TextLang::French] {
        let s = stem(token, lang);
        if s.chars().count() >= 2 && s != token && !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// Strips accents from common Latin characters so `équipe` and `equipe` compare equal.
pub fn fold_diacritics(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'à' | 'â' | 'ä' | 'á' | 'ã' | 'å' => out.push('a'),
            'À' | 'Â' | 'Ä' | 'Á' | 'Ã' | 'Å' => out.push('A'),
            'ç' => out.push('c'),
            'Ç' => out.push('C'),
            'é' | 'è' | 'ê' | 'ë' => out.push('e'),
            'É' | 'È' | 'Ê' | 'Ë' => out.push('E'),
            'î' | 'ï' | 'í' | 'ì' => out.push('i'),
            'Î' | 'Ï' | 'Í' | 'Ì' => out.push('I'),
            'ô' | 'ö' | 'ó' | 'ò' | 'õ' => out.push('o'),
            'Ô' | 'Ö' | 'Ó' | 'Ò' | 'Õ' => out.push('O'),
            'ù' | 'û' | 'ü' | 'ú' => out.push('u'),
            'Ù' | 'Û' | 'Ü' | 'Ú' => out.push('U'),
            'ÿ' | 'ý' => out.push('y'),
            'Ÿ' | 'Ý' => out.push('Y'),
            'ñ' => out.push('n'),
            'Ñ' => out.push('N'),
            'œ' => out.push_str("oe"),
            'Œ' => out.push_str("OE"),
            'æ' => out.push_str("ae"),
            'Æ' => out.push_str("AE"),
            _ => out.push(c),
        }
    }
    out
}

/// Plain text indexed for an email: markup removed and, unless the email is a forward,
/// quoted replies and signatures cut off so every reply in a thread does not match the
/// same words.
pub fn index_body(subject: &str, raw_body: &str) -> String {
    let plain = crate::email_html::html_to_plain_text(raw_body);
    let normalized = plain.replace("\r\n", "\n").replace('\r', "\n");
    let text = if is_forward(subject) {
        normalized
    } else {
        strip_quoted_reply(&normalized)
    };
    collapse_blank_lines(&text)
}

/// Stemmed, stopword-free copy of subject + body for the `stems` FTS column.
#[cfg_attr(not(test), allow(dead_code))]
pub fn index_stems(subject: &str, body_text: &str) -> String {
    index_stems_with_attachments(subject, body_text, "")
}

/// Stemmed, stopword-free copy of subject + body + attachments for the `stems` FTS column.
pub fn index_stems_with_attachments(
    subject: &str,
    body_text: &str,
    attachment_text: &str,
) -> String {
    let mut tokens = tokenize(subject);
    tokens.extend(tokenize(body_text));
    tokens.extend(tokenize(attachment_text));
    let lang = detect_language(&tokens);
    let mut out = String::with_capacity((body_text.len() + attachment_text.len()) / 2);
    for token in tokens {
        if token.chars().count() < 2 || is_stopword(&token) {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&stem(&token, lang));
    }
    out
}

fn is_forward(subject: &str) -> bool {
    let lower = subject.trim_start().to_lowercase();
    ["fw:", "fwd:", "tr:", "fw :", "fwd :", "tr :"]
        .iter()
        .any(|p| lower.starts_with(p))
}

pub fn strip_quoted_reply(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    let mut has_content = false;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('>') {
            continue;
        }
        if has_content && is_reply_boundary(trimmed, &lines[i + 1..]) {
            break;
        }
        if *line == "-- " || trimmed == "--" {
            break;
        }
        if !trimmed.is_empty() {
            has_content = true;
        }
        out.push(line.trim_end());
    }
    out.join("\n")
}

fn is_reply_boundary(trimmed: &str, following: &[&str]) -> bool {
    let lower = trimmed.to_lowercase();
    let starts_original = [
        "-----original message",
        "----- original message",
        "-----message d'origine",
        "----- message d'origine",
        "-----courriel original",
        "----- courriel original",
        "-------- original message",
        "-------- message original",
    ];
    if starts_original.iter().any(|p| lower.starts_with(p)) {
        return true;
    }
    let next = following
        .iter()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_lowercase();
    // "On Mon, 3 Mar 2025, Alice <a@x> wrote:" possibly wrapped on two lines.
    if lower.starts_with("on ")
        && (lower.ends_with("wrote:") || (!lower.contains(':') && next.ends_with("wrote:")))
    {
        return true;
    }
    if lower.starts_with("le ")
        && (lower.ends_with("a écrit :")
            || lower.ends_with("a écrit:")
            || next.ends_with("a écrit :")
            || next.ends_with("a écrit:"))
    {
        return true;
    }
    // Outlook header block: "From: ..." followed by "Sent:" / "Date:" / "To:" lines.
    let is_from =
        lower.starts_with("from:") || lower.starts_with("de :") || lower.starts_with("de:");
    let underscores = trimmed.len() >= 20 && trimmed.chars().all(|c| c == '_');
    if is_from || underscores {
        let header_keys = [
            "sent:",
            "date:",
            "to:",
            "envoyé :",
            "envoyé:",
            "à :",
            "à:",
            "objet :",
            "subject:",
            "from:",
            "de :",
        ];
        return following
            .iter()
            .take(4)
            .map(|l| l.trim().to_lowercase())
            .any(|l| header_keys.iter().any(|k| l.starts_with(k)));
    }
    false
}

fn collapse_blank_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_run = 0;
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

/// Comparison keys for a token: folded token plus folded English/French stems.
fn term_keys(token: &str) -> [String; 3] {
    [
        fold_diacritics(token),
        fold_diacritics(&stem(token, TextLang::English)),
        fold_diacritics(&stem(token, TextLang::French)),
    ]
}

/// Splits text into segments of at most `max` chars, breaking on lines then whitespace.
fn segments(text: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if line.chars().count() <= max {
            out.push(line.to_string());
            continue;
        }
        let mut current = String::new();
        for word in line.split_whitespace() {
            if !current.is_empty() && current.chars().count() + word.chars().count() + 1 > max {
                out.push(std::mem::take(&mut current));
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
        if !current.is_empty() {
            out.push(current);
        }
    }
    out
}

pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// The passages of `text` that best match `query_tokens`, within `max_chars`.
/// Falls back to the beginning of the text when nothing matches.
pub fn best_passages(text: &str, query_tokens: &[String], max_chars: usize) -> String {
    best_passages_matched(text, query_tokens, max_chars).0
}

/// Like [`best_passages`], also returning whether at least one query token matched in `text`.
pub fn best_passages_matched(
    text: &str,
    query_tokens: &[String],
    max_chars: usize,
) -> (String, bool) {
    let segs = segments(text, 280);
    if segs.is_empty() {
        return (String::new(), false);
    }
    let joined_len: usize = segs.iter().map(|s| s.chars().count() + 1).sum();
    let keys: HashSet<String> = query_tokens
        .iter()
        .filter(|t| !is_stopword(t))
        .flat_map(|t| term_keys(t))
        .filter(|k| k.chars().count() >= 2)
        .collect();
    if keys.is_empty() {
        return (truncate_chars(&segs.join(" "), max_chars), false);
    }

    let scores: Vec<usize> = segs
        .iter()
        .map(|seg| {
            let mut matched = HashSet::new();
            for token in tokenize(seg) {
                for key in term_keys(&token) {
                    if keys.contains(&key) {
                        matched.insert(key);
                    }
                }
            }
            matched.len()
        })
        .collect();

    let mut order: Vec<usize> = (0..segs.len()).filter(|&i| scores[i] > 0).collect();
    let has_match = !order.is_empty();
    if joined_len <= max_chars || !has_match {
        return (truncate_chars(&segs.join(" "), max_chars), has_match);
    }
    order.sort_by_key(|&i| (Reverse(scores[i]), i));

    let mut selected = BTreeSet::new();
    let mut used = 0usize;
    'outer: for i in order {
        for j in [Some(i), i.checked_sub(1), Some(i + 1)]
            .into_iter()
            .flatten()
        {
            if j >= segs.len() || selected.contains(&j) {
                continue;
            }
            let len = segs[j].chars().count() + 3;
            if used + len > max_chars {
                if j == i && selected.is_empty() {
                    selected.insert(j);
                }
                if j == i {
                    break 'outer;
                }
                continue;
            }
            used += len;
            selected.insert(j);
        }
    }

    let mut out = String::new();
    let mut prev: Option<usize> = None;
    for i in selected {
        match prev {
            None if i > 0 => out.push_str("… "),
            Some(p) if i != p + 1 => out.push_str(" … "),
            Some(_) => out.push(' '),
            None => {}
        }
        out.push_str(&segs[i]);
        prev = Some(i);
    }
    if prev.is_some_and(|p| p + 1 < segs.len()) {
        out.push_str(" …");
    }
    (truncate_chars(&out, max_chars + 8), true)
}
