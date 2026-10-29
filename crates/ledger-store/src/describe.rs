//! The journal in words: one sentence for each line, and the subjects a
//! line is about. The one describer the server's `books.journal_history`
//! reads.
//!
//! ```text
//! GOAL: an agent can read what the books did, in order, without the
//!       journal file: each line says in a sentence what happened, to
//!       what, and which entry it belongs to.
//! SOURCE: plan/served.md P5 step 4 (the only way to "read the journal");
//!         docs/receipt-store.md (the lines and their payloads);
//!         docs/spec/rules.md B-26 (no output holds a full TIN).
//! INVARIANTS:
//!   1. Every line type the store writes has a sentence (test: every
//!      `format::EVENT_TYPES` entry).
//!   2. A sentence quotes only identifiers, dates, amounts and the
//!      short names and descriptions a person typed; never a setting's
//!      value, a payload's secret, or a full TIN (they are masked).
//!   3. Lines are returned in journal order, from the line after
//!      `after_seq`, at most `limit`; a filter never reorders them.
//!   4. A subject is `kind:id` (`entry:12`, `invoice:3`, `bill:3`,
//!      `account:10`, `party:5`, `asset:2`); a line is about the subject
//!      its type names and every one its payload refers to.
//! ```

use crate::journal::JournalLine;
use serde_json::Value;

/// The most characters of any quoted text.
const QUOTE_MAX: usize = 80;

/// One line of the journal, described.
#[derive(Clone, Debug, PartialEq)]
pub struct Described {
    /// The line's seq.
    pub seq: u64,
    /// When it was written (UTC, RFC 3339).
    pub ts: String,
    /// Its type.
    pub line_type: String,
    /// A sentence.
    pub summary: String,
    /// The subjects it is about, as `kind:id`.
    pub subjects: Vec<String>,
    /// The entry it belongs to, if it is about one.
    pub entry_id: Option<u64>,
}

/// Which lines to list (INVARIANT 3).
#[derive(Clone, Debug, Default)]
pub struct HistoryFilter {
    /// Only lines after this seq.
    pub after_seq: u64,
    /// Only these types: an exact type (`entry.posted`) or a group ending
    /// in a dot (`entry.`). Empty: every type.
    pub types: Vec<String>,
    /// Only lines about this subject (`entry:12`).
    pub subject: Option<String>,
    /// At most this many.
    pub limit: usize,
}

/// A subject as `kind:id`, parsed.
pub fn parse_subject(text: &str) -> Result<(String, u64), String> {
    let bad = || {
        format!(
            "subject {text:?}: give kind:id, like entry:12, invoice:3, bill:3, account:10, party:5 or asset:2"
        )
    };
    let (kind, id) = text.split_once(':').ok_or_else(bad)?;
    let kinds = [
        "entry",
        "invoice",
        "bill",
        "account",
        "party",
        "asset",
        "period",
        "schedule",
        "reconciliation",
        "payroll_run",
        "project",
    ];
    if !kinds.contains(&kind.trim()) {
        return Err(bad());
    }
    let id = id.trim().parse::<u64>().map_err(|_| bad())?;
    Ok((kind.trim().to_string(), id))
}

fn num(p: &Value, k: &str) -> Option<u64> {
    p.get(k).and_then(Value::as_u64)
}

fn quote(text: &str) -> String {
    let t: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > QUOTE_MAX {
        let mut s: String = t.chars().take(QUOTE_MAX - 1).collect();
        s.push('…');
        s
    } else {
        t
    }
}

/// The subjects a line is about (INVARIANT 4).
pub fn subjects(line: &JournalLine) -> Vec<String> {
    let p = &line.payload;
    let mut out: Vec<String> = Vec::new();
    let mut add = |kind: &str, id: Option<u64>| {
        if let Some(id) = id {
            let s = format!("{kind}:{id}");
            if !out.contains(&s) {
                out.push(s);
            }
        }
    };
    let prefix = line.line_type.split('.').next().unwrap_or("");
    let own = match prefix {
        "entry" => Some("entry"),
        "invoice" => Some("invoice"),
        "bill" => Some("bill"),
        "account" => Some("account"),
        "party" => Some("party"),
        "asset" => Some("asset"),
        "period" => Some("period"),
        "schedule" => Some("schedule"),
        "reconciliation" => Some("reconciliation"),
        "payroll" => Some("payroll_run"),
        "project" => Some("project"),
        _ => None,
    };
    if let Some(kind) = own {
        // `entry.posted` carries `entry_id`; `entry.created` carries `id`.
        let id = match prefix {
            "entry" => num(p, "entry_id").or_else(|| num(p, "id")),
            "invoice" => num(p, "invoice_id").or_else(|| num(p, "id")),
            "bill" => num(p, "bill_id").or_else(|| num(p, "id")),
            "asset" => num(p, "asset_id").or_else(|| num(p, "id")),
            "party" => num(p, "party_id").or_else(|| num(p, "id")),
            _ => num(p, "id"),
        };
        add(kind, id);
    }
    add("entry", num(p, "entry_id"));
    add("invoice", num(p, "invoice_id"));
    add("bill", num(p, "bill_id"));
    add("account", num(p, "account_id"));
    add("asset", num(p, "asset_id"));
    add("party", num(p, "party_id"));
    if line.line_type == "receipt.attached" || line.line_type == "receipt.linked" {
        add("entry", num(p, "entry_id"));
    }
    out
}

/// A line's amount, when it has one: a string, or `{amount: ..}`.
fn amount_of(p: &Value) -> Option<String> {
    let a = p.get("amount")?;
    a.as_str()
        .map(str::to_string)
        .or_else(|| a.get("amount").and_then(Value::as_str).map(str::to_string))
}

/// The sentence for a line (INVARIANT 2).
pub fn describe(line: &JournalLine) -> String {
    let p = &line.payload;
    let ty = line.line_type.as_str();
    let mut head = ty.replace(['.', '_'], " ");
    if let Some(first) = head.get(0..1) {
        head = first.to_uppercase() + &head[1..];
    }
    let mut parts: Vec<String> = Vec::new();
    // What it is about, by id.
    for (k, label) in [
        ("entry_id", "entry"),
        ("invoice_id", "invoice"),
        ("bill_id", "bill"),
        ("account_id", "account"),
        ("asset_id", "asset"),
        ("party_id", "party"),
        ("attachment_id", "attachment"),
    ] {
        if let Some(n) = num(p, k) {
            parts.push(format!("{label} {n}"));
        }
    }
    if parts.is_empty()
        && let Some(n) = num(p, "id")
    {
        parts.push(format!("#{n}"));
    }
    // What a person typed about it.
    for k in ["description", "title", "name", "subject", "code"] {
        if let Some(t) = p.get(k).and_then(Value::as_str)
            && !t.is_empty()
        {
            parts.push(quote(t));
            break;
        }
    }
    if let Some(a) = amount_of(p) {
        parts.push(a);
    }
    if let Some(d) = p.get("date").and_then(Value::as_str) {
        parts.push(d.to_string());
    }
    // Settings: which keys, never their values.
    if ty == "settings.set"
        && let Some(changes) = p.get("changes").and_then(Value::as_array)
    {
        let keys: Vec<&str> = changes
            .iter()
            .filter_map(|c| c.get("key").and_then(Value::as_str))
            .take(8)
            .collect();
        parts.push(keys.join(", "));
    }
    let text = if parts.is_empty() {
        head
    } else {
        format!("{head}: {}", parts.join(", "))
    };
    ledger_core::parties::redact_full_tins(&text).into_owned()
}

/// Describe one line.
pub fn describe_line(line: &JournalLine) -> Described {
    let subjects = subjects(line);
    let entry_id = subjects
        .iter()
        .find_map(|s| s.strip_prefix("entry:").and_then(|n| n.parse().ok()));
    Described {
        seq: line.seq,
        ts: line.ts.clone(),
        line_type: line.line_type.clone(),
        summary: describe(line),
        subjects,
        entry_id,
    }
}

/// The lines `filter` asks for, described, in journal order (INVARIANT 3),
/// and whether more match after the last one returned.
pub fn history(lines: &[JournalLine], filter: &HistoryFilter) -> (Vec<Described>, bool) {
    let wanted = |l: &JournalLine| {
        l.seq > filter.after_seq
            && (filter.types.is_empty()
                || filter.types.iter().any(|t| {
                    if t.ends_with('.') {
                        l.line_type.starts_with(t.as_str())
                    } else {
                        l.line_type == *t
                    }
                }))
            && filter
                .subject
                .as_ref()
                .is_none_or(|s| subjects(l).contains(s))
    };
    let mut out = Vec::new();
    let mut more = false;
    for l in lines.iter().filter(|l| wanted(l)) {
        if out.len() == filter.limit {
            more = true;
            break;
        }
        out.push(describe_line(l));
    }
    (out, more)
}

/// The journal in words, read from the books' own journal file.
#[derive(Clone, Debug, PartialEq)]
pub struct History {
    /// The lines asked for, in order.
    pub lines: Vec<Described>,
    /// Whether more match after the last.
    pub more: bool,
    /// The journal's last seq.
    pub head_seq: u64,
    /// Torn tails set aside on open: (when, bytes).
    pub tail_repairs: Vec<(String, u64)>,
}

impl crate::Store {
    /// `filter`'s lines of the journal, described (the only way an agent
    /// reads the journal: `books.journal_history`). The file is read and
    /// its chain verified, as `Store::verify` does.
    pub fn journal_history(&self, filter: &HistoryFilter) -> Result<History, crate::StoreError> {
        let (lines, _format) = crate::journal::read_journal_and_format(
            &self.data_dir().join(crate::store::JOURNAL_FILE),
        )?;
        let (described, more) = history(&lines, filter);
        let tail_repairs = self
            .tail_repairs()?
            .into_iter()
            .map(|(when, bytes, _saved_as)| (when, bytes))
            .collect();
        Ok(History {
            lines: described,
            more,
            head_seq: lines.last().map_or(0, |l| l.seq),
            tail_repairs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn line(seq: u64, ty: &str, payload: Value) -> JournalLine {
        JournalLine {
            seq,
            ts: "2026-10-29T00:00:00Z".into(),
            line_type: ty.into(),
            payload,
            prev: String::new(),
        }
    }

    #[test]
    fn every_event_type_has_a_sentence() {
        for ty in crate::format::EVENT_TYPES {
            let s = describe(&line(1, ty, json!({})));
            assert!(!s.is_empty(), "{ty}");
        }
    }

    #[test]
    fn a_sentence_names_the_entry_and_what_a_person_typed() {
        let l = line(
            5,
            "entry.posted",
            json!({"entry_id": 12, "description": "Office   supplies"}),
        );
        assert_eq!(describe(&l), "Entry posted: entry 12, Office supplies");
        assert_eq!(subjects(&l), vec!["entry:12"]);
        assert_eq!(describe_line(&l).entry_id, Some(12));
    }

    #[test]
    fn settings_show_their_keys_never_their_values() {
        let l = line(
            2,
            "settings.set",
            json!({"changes": [{"key": "company.ein", "value": "12-3456789"}]}),
        );
        let s = describe(&l);
        assert!(
            s.contains("company.ein") && !s.contains("12-3456789"),
            "{s}"
        );
    }

    #[test]
    fn a_full_tin_in_a_description_is_masked() {
        let l = line(
            3,
            "entry.created",
            json!({"id": 1, "description": "TIN 123-45-6789"}),
        );
        assert!(!describe(&l).contains("6789"), "{}", describe(&l));
    }

    #[test]
    fn history_filters_in_order_and_says_when_more_remain() {
        let lines: Vec<JournalLine> = (1..=6)
            .map(|i| {
                line(
                    i,
                    if i % 2 == 0 {
                        "entry.posted"
                    } else {
                        "entry.created"
                    },
                    json!({"entry_id": i % 3, "id": i % 3}),
                )
            })
            .collect();
        let f = HistoryFilter {
            after_seq: 1,
            types: vec!["entry.posted".into()],
            subject: None,
            limit: 2,
        };
        let (got, more) = history(&lines, &f);
        assert_eq!(got.iter().map(|d| d.seq).collect::<Vec<_>>(), vec![2, 4]);
        assert!(more);
        let f = HistoryFilter {
            subject: Some("entry:0".into()),
            limit: 10,
            ..Default::default()
        };
        let (got, more) = history(&lines, &f);
        assert_eq!(got.iter().map(|d| d.seq).collect::<Vec<_>>(), vec![3, 6]);
        assert!(!more);
        assert!(parse_subject("entry:12").is_ok());
        assert!(parse_subject("nonsense").is_err());
    }
}
