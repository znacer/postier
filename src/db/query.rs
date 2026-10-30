//! BM25 query building and structured search over the local index.
//!
//! A user/agent query such as `budget approval from:alice after:2024-03 "Q3 forecast"` is
//! parsed into free-text terms, exact phrases and SQL filters. Each term is expanded into
//! its exact form plus its English/French Snowball stems (matched against the `stems`
//! column). Retrieval first requires every significant term (AND) and only falls back to
//! any term (OR) when that does not yield enough results. Results are collapsed to one
//! hit per thread so the top-k covers k different conversations.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use chrono::{Datelike, NaiveDate};
use rusqlite::types::Value;
use serde::{Deserialize, Serialize};

use super::{BM25_RANK, Index};
use crate::{model::SearchHit, text};

const ENABLED_SOURCES: &str = "s.enabled = 1
   AND NOT EXISTS (
     SELECT 1 FROM source_folder_settings sf
     WHERE sf.source_key = m.source_key AND sf.folder = m.folder AND sf.enabled = 0
   )";

const HIT_COLUMNS: &str = "m.id, m.source_key, m.subject, m.sender, m.recipients, m.sent_at,
    m.folder, m.body, m.attachment_names, m.conversation_id, m.message_id, m.body_text";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchFilters {
    /// Substring of the sender name/address.
    pub from: Option<String>,
    /// Substring of the To/Cc recipients.
    pub to: Option<String>,
    /// Inclusive lower bound: `YYYY`, `YYYY-MM`, `YYYY-MM-DD` or `DD/MM/YYYY`.
    pub date_from: Option<String>,
    /// Inclusive upper bound, same formats as `date_from`.
    pub date_to: Option<String>,
    /// Substring of the folder path.
    pub folder: Option<String>,
    /// Exact label name (case-insensitive).
    pub label: Option<String>,
    pub has_attachment: Option<bool>,
}

impl SearchFilters {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Fills unset fields of `self` from `other`.
    pub fn or(mut self, other: SearchFilters) -> Self {
        fn pick(a: &mut Option<String>, b: Option<String>) {
            if a.as_deref().is_none_or(|v| v.trim().is_empty()) {
                *a = b.filter(|v| !v.trim().is_empty());
            }
        }
        pick(&mut self.from, other.from);
        pick(&mut self.to, other.to);
        pick(&mut self.date_from, other.date_from);
        pick(&mut self.date_to, other.date_to);
        pick(&mut self.folder, other.folder);
        pick(&mut self.label, other.label);
        if self.has_attachment.is_none() {
            self.has_attachment = other.has_attachment;
        }
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedQuery {
    /// Single lowercase tokens; a trailing `*` requests prefix matching.
    pub terms: Vec<String>,
    /// Multi-token exact phrases (tokens joined by spaces).
    pub phrases: Vec<String>,
    pub filters: SearchFilters,
}

impl ParsedQuery {
    pub fn has_text(&self) -> bool {
        !self.terms.is_empty() || !self.phrases.is_empty()
    }

    /// Every token of the query, used to pick matching passages.
    pub fn tokens(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .terms
            .iter()
            .map(|t| t.trim_end_matches('*').to_string())
            .collect();
        for phrase in &self.phrases {
            out.extend(phrase.split(' ').map(str::to_string));
        }
        out
    }

    fn groups(&self) -> Vec<String> {
        let significant: Vec<&String> = self
            .terms
            .iter()
            .filter(|t| !text::is_stopword(t.trim_end_matches('*')))
            .collect();
        let terms: Vec<&String> = if significant.is_empty() && self.phrases.is_empty() {
            self.terms.iter().collect()
        } else {
            significant
        };
        let mut groups: Vec<String> = self.phrases.iter().map(|p| fts_quote(p)).collect();
        let mut seen = HashSet::new();
        for term in terms {
            if seen.insert(term.clone()) {
                groups.push(term_group(term));
            }
        }
        groups
    }

    /// FTS5 expression requiring every significant term and phrase.
    pub fn fts_all(&self) -> Option<String> {
        let groups = self.groups();
        (!groups.is_empty()).then(|| groups.join(" AND "))
    }

    /// FTS5 expression matching any significant term, or `None` when it would equal
    /// [`Self::fts_all`].
    pub fn fts_any(&self) -> Option<String> {
        let groups = self.groups();
        (groups.len() > 1).then(|| groups.join(" OR "))
    }
}

fn fts_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn term_group(term: &str) -> String {
    if let Some(prefix) = term.strip_suffix('*') {
        return format!("{}*", fts_quote(prefix));
    }
    let mut alternatives = vec![fts_quote(term)];
    for stem in text::query_stems(term) {
        alternatives.push(format!("stems:{}", fts_quote(&stem)));
    }
    if alternatives.len() == 1 {
        alternatives.remove(0)
    } else {
        format!("({})", alternatives.join(" OR "))
    }
}

enum FilterKey {
    From,
    To,
    After,
    Before,
    Folder,
    Label,
    Has,
}

fn filter_key(key: &str) -> Option<FilterKey> {
    Some(match key {
        "from" | "de" | "sender" | "expediteur" | "expéditeur" => FilterKey::From,
        "to" | "à" | "recipient" | "destinataire" | "cc" => FilterKey::To,
        "after" | "since" | "apres" | "après" | "depuis" => FilterKey::After,
        "before" | "until" | "avant" | "jusqu" => FilterKey::Before,
        "folder" | "dossier" | "in" | "dans" => FilterKey::Folder,
        "label" | "etiquette" | "étiquette" | "tag" => FilterKey::Label,
        "has" | "avec" => FilterKey::Has,
        _ => return None,
    })
}

/// Splits input into words, keeping `"quoted phrases"` and `key:"quoted values"` whole.
fn split_words(input: &str) -> Vec<(String, bool)> {
    let chars: Vec<char> = input.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let mut word = String::new();
        let mut quoted = false;
        while i < chars.len() && !chars[i].is_whitespace() {
            if chars[i] == '"' {
                quoted = true;
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    word.push(chars[i]);
                    i += 1;
                }
                i += 1;
            } else {
                word.push(chars[i]);
                i += 1;
            }
        }
        if !word.trim().is_empty() {
            out.push((word, quoted));
        }
    }
    out
}

pub fn parse_query(input: &str) -> ParsedQuery {
    let mut parsed = ParsedQuery::default();
    for (word, quoted) in split_words(input) {
        if let Some((key, value)) = word.split_once(':')
            && !value.trim().is_empty()
            && let Some(kind) = filter_key(&key.to_lowercase())
        {
            let value = value.trim().to_string();
            let f = &mut parsed.filters;
            match kind {
                FilterKey::From => f.from = Some(value),
                FilterKey::To => f.to = Some(value),
                FilterKey::After => f.date_from = Some(value),
                FilterKey::Before => f.date_to = Some(value),
                FilterKey::Folder => f.folder = Some(value),
                FilterKey::Label => f.label = Some(value),
                FilterKey::Has => {
                    let v = value.to_lowercase();
                    if v.starts_with("attach") || v == "pj" || v.starts_with("piece") {
                        f.has_attachment = Some(true);
                    }
                }
            }
            continue;
        }
        let prefix = !quoted && word.ends_with('*');
        let tokens = text::tokenize(&word);
        match tokens.len() {
            0 => {}
            1 if !quoted || !word.contains(' ') => {
                let mut token = tokens.into_iter().next().unwrap_or_default();
                if prefix {
                    token.push('*');
                }
                if !parsed.terms.contains(&token) {
                    parsed.terms.push(token);
                }
            }
            _ => parsed.phrases.push(tokens.join(" ")),
        }
    }
    parsed
}

fn parse_date(value: &str) -> Option<(NaiveDate, DateGranularity)> {
    let v = value.trim();
    if let Ok(d) = NaiveDate::parse_from_str(v, "%Y-%m-%d") {
        return Some((d, DateGranularity::Day));
    }
    if let Ok(d) = NaiveDate::parse_from_str(v, "%d/%m/%Y") {
        return Some((d, DateGranularity::Day));
    }
    if let Ok(d) = NaiveDate::parse_from_str(&format!("{v}-01"), "%Y-%m-%d") {
        return Some((d, DateGranularity::Month));
    }
    if v.len() == 4
        && let Ok(year) = v.parse::<i32>()
    {
        return NaiveDate::from_ymd_opt(year, 1, 1).map(|d| (d, DateGranularity::Year));
    }
    None
}

#[derive(Clone, Copy)]
enum DateGranularity {
    Day,
    Month,
    Year,
}

fn date_lower_bound(value: &str) -> Option<String> {
    parse_date(value).map(|(d, _)| d.format("%Y-%m-%d").to_string())
}

/// Exclusive upper bound for an inclusive `date_to` value.
fn date_upper_bound(value: &str) -> Option<String> {
    let (d, granularity) = parse_date(value)?;
    let next = match granularity {
        DateGranularity::Day => d.succ_opt()?,
        DateGranularity::Month => {
            if d.month() == 12 {
                NaiveDate::from_ymd_opt(d.year() + 1, 1, 1)?
            } else {
                NaiveDate::from_ymd_opt(d.year(), d.month() + 1, 1)?
            }
        }
        DateGranularity::Year => NaiveDate::from_ymd_opt(d.year() + 1, 1, 1)?,
    };
    Some(next.format("%Y-%m-%d").to_string())
}

/// Whether date filter values are understood; returns the invalid ones.
pub fn invalid_dates(filters: &SearchFilters) -> Vec<String> {
    [&filters.date_from, &filters.date_to]
        .into_iter()
        .flatten()
        .filter(|v| !v.trim().is_empty() && parse_date(v).is_none())
        .cloned()
        .collect()
}

fn like_contains(value: &str) -> String {
    format!("%{}%", like_escape(value.trim()))
}

fn like_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Appends ` AND ...` clauses for `filters` (expects the messages table aliased as `m`).
pub fn push_filter_sql(filters: &SearchFilters, sql: &mut String, binds: &mut Vec<Value>) {
    let nonempty = |v: &Option<String>| {
        v.as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    if let Some(from) = nonempty(&filters.from) {
        sql.push_str(" AND m.sender LIKE ? ESCAPE '\\'");
        binds.push(Value::Text(like_contains(&from)));
    }
    if let Some(to) = nonempty(&filters.to) {
        sql.push_str(" AND m.recipients LIKE ? ESCAPE '\\'");
        binds.push(Value::Text(like_contains(&to)));
    }
    if let Some(bound) = nonempty(&filters.date_from).and_then(|v| date_lower_bound(&v)) {
        sql.push_str(" AND m.sent_at >= ?");
        binds.push(Value::Text(bound));
    }
    if let Some(bound) = nonempty(&filters.date_to).and_then(|v| date_upper_bound(&v)) {
        sql.push_str(" AND m.sent_at < ?");
        binds.push(Value::Text(bound));
    }
    if let Some(folder) = nonempty(&filters.folder) {
        sql.push_str(" AND m.folder LIKE ? ESCAPE '\\'");
        binds.push(Value::Text(like_contains(&folder)));
    }
    if let Some(label) = nonempty(&filters.label) {
        sql.push_str(
            " AND EXISTS (SELECT 1 FROM message_labels ml
                          WHERE ml.message_id = m.id AND ml.label LIKE ? ESCAPE '\\')",
        );
        binds.push(Value::Text(like_escape(&label)));
    }
    match filters.has_attachment {
        Some(true) => sql.push_str(" AND m.attachment_names <> ''"),
        Some(false) => sql.push_str(" AND m.attachment_names = ''"),
        None => {}
    }
}

/// `FROM ... WHERE ...` shared by searches and statistics.
fn from_where(fts: Option<&str>, filters: &SearchFilters, binds: &mut Vec<Value>) -> String {
    let mut sql = String::new();
    if let Some(expr) = fts {
        sql.push_str(&format!(
            " FROM messages_fts f
              JOIN messages m ON m.id = f.rowid
              JOIN indexed_sources s ON s.source_key = m.source_key
              WHERE messages_fts MATCH ? AND {ENABLED_SOURCES}"
        ));
        binds.push(Value::Text(expr.to_string()));
    } else {
        sql.push_str(&format!(
            " FROM messages m
              JOIN indexed_sources s ON s.source_key = m.source_key
              WHERE {ENABLED_SOURCES}"
        ));
    }
    push_filter_sql(filters, &mut sql, binds);
    sql
}

fn map_hit(r: &rusqlite::Row<'_>, score: f64) -> rusqlite::Result<SearchHit> {
    Ok(SearchHit {
        id: r.get(0)?,
        source_key: r.get(1)?,
        subject: r.get(2)?,
        sender: r.get(3)?,
        recipients: r.get(4)?,
        sent_at: r.get(5)?,
        folder: r.get(6)?,
        body: r.get(7)?,
        attachment_names: r.get(8)?,
        conversation_id: r.get(9)?,
        message_id: r.get(10)?,
        body_text: r.get(11)?,
        labels: Vec::new(),
        score,
        thread_matches: 1,
    })
}

fn thread_key(hit: &SearchHit) -> String {
    let conv = hit.conversation_id.trim();
    if conv.is_empty() {
        format!("id:{}", hit.id)
    } else {
        conv.to_string()
    }
}

/// Keeps the best-ranked message of each thread, counting matches per thread.
fn collapse_threads(ranked: Vec<SearchHit>, limit: usize) -> Vec<SearchHit> {
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut out: Vec<SearchHit> = Vec::new();
    for hit in ranked {
        let key = thread_key(&hit);
        if let Some(&i) = index.get(&key) {
            out[i].thread_matches += 1;
        } else {
            index.insert(key, out.len());
            out.push(hit);
        }
    }
    out.truncate(limit);
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatsGroup {
    Sender,
    Month,
    Year,
    Folder,
    Label,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StatsBucket {
    pub key: String,
    pub count: u64,
}

impl Index {
    fn query_hits(
        &self,
        fts: Option<&str>,
        filters: &SearchFilters,
        limit: usize,
    ) -> Result<Vec<SearchHit>> {
        let mut binds = Vec::new();
        let rank = if fts.is_some() { BM25_RANK } else { "0.0" };
        let mut sql = format!("SELECT {HIT_COLUMNS}, {rank} AS rank");
        sql.push_str(&from_where(fts, filters, &mut binds));
        if fts.is_some() {
            sql.push_str(" ORDER BY rank, coalesce(m.sent_at, '') DESC LIMIT ?");
        } else {
            sql.push_str(" ORDER BY coalesce(m.sent_at, '') DESC, m.id DESC LIMIT ?");
        }
        binds.push(Value::Integer(limit as i64));
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(binds), |r| {
            let rank: f64 = r.get(12)?;
            map_hit(r, -rank)
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// BM25 search with filters; one result per thread, best first. Without free text,
    /// returns the most recent messages matching the filters.
    pub fn search_emails(&self, query: &ParsedQuery, limit: usize) -> Result<Vec<SearchHit>> {
        let limit = limit.max(1);
        let candidates = (limit * 4).clamp(20, 400);
        let mut ranked = match query.fts_all() {
            None => self.query_hits(None, &query.filters, candidates)?,
            Some(all) => self.query_hits(Some(&all), &query.filters, candidates)?,
        };
        if let Some(any) = query.fts_any() {
            let distinct_threads: HashSet<String> = ranked.iter().map(thread_key).collect();
            if distinct_threads.len() < limit {
                let seen: HashSet<i64> = ranked.iter().map(|h| h.id).collect();
                for hit in self.query_hits(Some(&any), &query.filters, candidates)? {
                    if !seen.contains(&hit.id) {
                        ranked.push(hit);
                    }
                }
            }
        }
        let mut hits = collapse_threads(ranked, limit);
        self.attach_search_hit_labels(&mut hits)?;
        Ok(hits)
    }

    /// Convenience wrapper parsing operators (`from:`, `after:`, ...) out of `query`.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        self.search_emails(&parse_query(query), limit)
    }

    /// Number of matching messages, optionally grouped. Uses the strict (AND) form of
    /// the query so counts stay precise.
    pub fn email_stats(
        &self,
        query: &ParsedQuery,
        group: Option<StatsGroup>,
        top: usize,
    ) -> Result<(u64, Vec<StatsBucket>)> {
        let fts = query.fts_all();
        let mut binds = Vec::new();
        let where_sql = from_where(fts.as_deref(), &query.filters, &mut binds);
        let total: u64 = self.conn.query_row(
            &format!("SELECT count(*) {where_sql}"),
            rusqlite::params_from_iter(binds.clone()),
            |r| r.get(0),
        )?;
        let Some(group) = group else {
            return Ok((total, Vec::new()));
        };
        let key_expr = match group {
            StatsGroup::Sender => "m.sender",
            StatsGroup::Month => "substr(coalesce(m.sent_at, 'unknown'), 1, 7)",
            StatsGroup::Year => "substr(coalesce(m.sent_at, 'unknown'), 1, 4)",
            StatsGroup::Folder => "m.folder",
            StatsGroup::Label => "ml.label",
        };
        let chronological = matches!(group, StatsGroup::Month | StatsGroup::Year);
        let order = if chronological {
            "key DESC"
        } else {
            "count(*) DESC, key ASC"
        };
        let sql = if group == StatsGroup::Label {
            format!(
                "SELECT {key_expr} AS key, count(*)
                 FROM (SELECT m.id AS id {where_sql}) x
                 JOIN message_labels ml ON ml.message_id = x.id
                 GROUP BY key ORDER BY {order} LIMIT ?"
            )
        } else {
            format!(
                "SELECT {key_expr} AS key, count(*) {where_sql} GROUP BY key ORDER BY {order} LIMIT ?"
            )
        };
        let mut binds = binds;
        binds.push(Value::Integer(top.max(1) as i64));
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(binds), |r| {
            Ok(StatsBucket {
                key: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                count: r.get(1)?,
            })
        })?;
        let mut buckets = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        if chronological {
            buckets.reverse();
        }
        Ok((total, buckets))
    }

    /// Messages of a conversation in chronological order (enabled sources only).
    pub fn get_thread(
        &self,
        conversation_id: &str,
        limit: usize,
    ) -> Result<(usize, Vec<SearchHit>)> {
        let conversation_id = conversation_id.trim();
        if conversation_id.is_empty() {
            return Ok((0, Vec::new()));
        }
        let total: usize = self.conn.query_row(
            &format!(
                "SELECT count(*) FROM messages m
                 JOIN indexed_sources s ON s.source_key = m.source_key
                 WHERE m.conversation_id = ?1 AND {ENABLED_SOURCES}"
            ),
            [conversation_id],
            |r| r.get::<_, i64>(0),
        )? as usize;
        // Keep the first message and the most recent ones when the thread is long:
        // outcomes and decisions are usually at the end.
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {HIT_COLUMNS} FROM messages m
             JOIN indexed_sources s ON s.source_key = m.source_key
             WHERE m.conversation_id = ?1 AND {ENABLED_SOURCES}
             ORDER BY coalesce(m.sent_at, '') ASC, m.id ASC"
        ))?;
        let rows = stmt.query_map([conversation_id], |r| map_hit(r, 0.0))?;
        let mut all = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        if all.len() > limit && limit >= 2 {
            let tail = all.split_off(all.len() - (limit - 1));
            all.truncate(1);
            all.extend(tail);
        }
        self.attach_search_hit_labels(&mut all)?;
        Ok((total, all))
    }

    pub fn get_hit(&self, id: i64) -> Result<Option<SearchHit>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {HIT_COLUMNS} FROM messages m WHERE m.id = ?1"
        ))?;
        let mut rows = stmt.query_map([id], |r| map_hit(r, 0.0))?;
        let Some(hit) = rows.next().transpose()? else {
            return Ok(None);
        };
        let mut hits = vec![hit];
        self.attach_search_hit_labels(&mut hits)?;
        Ok(hits.pop())
    }
}
