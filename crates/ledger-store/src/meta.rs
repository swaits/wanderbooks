//! Journal payloads of the books' own record of how they are kept:
//! settings, conventions, decisions, open items and approvals (journal
//! format 2, plan/served.md 5.2). One encoder and one decoder per event,
//! so the writer and every reader (replay, the projection, the viewer)
//! read a line the same way.
//!
//! ```text
//! GOAL: every event round-trips: decode(encode(x)) == x.
//! INVARIANTS:
//!   1. A payload missing a field, or with a field of the wrong type, is
//!      JournalCorrupt; optional fields may be absent or null.
//!   2. Decoding checks shape only (rules.md B-28): what a command checks
//!      (limits, the chart) stays with the command.
//! ```

use crate::error::StoreError;
use crate::journal::JournalLine;
use chrono::NaiveDate;
use ledger_core::arap::{SendKind, Sent, SentVia};
use ledger_core::conventions::{Approval, ConventionKind, NewConvention, NewDecision, NewOpenItem};
use ledger_core::settings::{Change, Source};
use ledger_core::{
    Annotation, AnnotationFields, ClassId, ProjectFacts, ProjectKind, ProjectStatus,
};
use serde_json::{Map, Value, json};

/// The journal format these events need: an older reader stops at the
/// declaration (format gate) instead of misreading them.
pub const FORMAT: u32 = 2;

fn corrupt(event: &str, what: &str) -> StoreError {
    StoreError::JournalCorrupt(format!("{event}: missing or invalid '{what}'"))
}

fn str_of(p: &Value, event: &str, k: &str) -> Result<String, StoreError> {
    p.get(k)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| corrupt(event, k))
}

fn opt_str(p: &Value, event: &str, k: &str) -> Result<Option<String>, StoreError> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(corrupt(event, k)),
    }
}

fn u64_of(p: &Value, event: &str, k: &str) -> Result<u64, StoreError> {
    p.get(k)
        .and_then(Value::as_u64)
        .ok_or_else(|| corrupt(event, k))
}

fn opt_u64(p: &Value, event: &str, k: &str) -> Result<Option<u64>, StoreError> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| corrupt(event, k)),
    }
}

fn date_of(s: &str, event: &str, k: &str) -> Result<NaiveDate, StoreError> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| corrupt(event, k))
}

fn opt_date(p: &Value, event: &str, k: &str) -> Result<Option<NaiveDate>, StoreError> {
    opt_str(p, event, k)?
        .map(|s| date_of(&s, event, k))
        .transpose()
}

/// Insert `v` under `k` unless it is `None`.
fn put<T: Into<Value>>(o: &mut Map<String, Value>, k: &str, v: Option<T>) {
    if let Some(v) = v {
        o.insert(k.to_string(), v.into());
    }
}

/// A source as journaled: `{decided_by, decided_on, citation?, note?}`.
pub fn source_json(s: &Source) -> Value {
    let mut o = Map::new();
    o.insert("decided_by".into(), json!(s.decided_by));
    o.insert("decided_on".into(), json!(s.decided_on.to_string()));
    put(&mut o, "citation", s.citation.clone());
    put(&mut o, "note", s.note.clone());
    Value::Object(o)
}

/// A journaled source (`p[k]`).
pub fn source_of(p: &Value, event: &str, k: &str) -> Result<Source, StoreError> {
    let s = p
        .get(k)
        .filter(|v| v.is_object())
        .ok_or_else(|| corrupt(event, k))?;
    Ok(Source {
        decided_by: str_of(s, event, "decided_by")?,
        decided_on: date_of(&str_of(s, event, "decided_on")?, event, "source.decided_on")?,
        citation: opt_str(s, event, "citation")?,
        note: opt_str(s, event, "note")?,
    })
}

/// `settings.set {changes: [{key, value|null}], source}`.
pub fn settings_set(changes: &[Change], source: &Source) -> Value {
    json!({
        "changes": changes
            .iter()
            .map(|c| json!({ "key": c.key, "value": c.value.clone().unwrap_or(Value::Null) }))
            .collect::<Vec<_>>(),
        "source": source_json(source),
    })
}

/// Read a `settings.set`.
pub fn settings_set_of(p: &Value) -> Result<(Vec<Change>, Source), StoreError> {
    const E: &str = "settings.set";
    let changes = p
        .get("changes")
        .and_then(Value::as_array)
        .ok_or_else(|| corrupt(E, "changes"))?
        .iter()
        .map(|c| {
            Ok(Change {
                key: str_of(c, E, "key")?,
                value: match c.get("value") {
                    None => return Err(corrupt(E, "value")),
                    Some(Value::Null) => None,
                    Some(v) => Some(v.clone()),
                },
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    Ok((changes, source_of(p, E, "source")?))
}

/// `convention.set {id, kind, title, body_md, structured?, source}`.
pub fn convention_set(c: &NewConvention) -> Value {
    let mut o = Map::new();
    o.insert("id".into(), json!(c.id));
    o.insert("kind".into(), json!(c.kind.key()));
    o.insert("title".into(), json!(c.title));
    o.insert("body_md".into(), json!(c.body_md));
    put(&mut o, "structured", c.structured.clone());
    o.insert("source".into(), source_json(&c.source));
    Value::Object(o)
}

/// Read a `convention.set`.
pub fn convention_set_of(p: &Value) -> Result<NewConvention, StoreError> {
    const E: &str = "convention.set";
    Ok(NewConvention {
        id: str_of(p, E, "id")?,
        kind: ConventionKind::parse(&str_of(p, E, "kind")?).ok_or_else(|| corrupt(E, "kind"))?,
        title: str_of(p, E, "title")?,
        body_md: str_of(p, E, "body_md")?,
        structured: p.get("structured").filter(|v| !v.is_null()).cloned(),
        source: source_of(p, E, "source")?,
    })
}

/// `convention.retired {id, reason, source}`.
pub fn convention_retired(id: &str, reason: &str, source: &Source) -> Value {
    json!({ "id": id, "reason": reason, "source": source_json(source) })
}

/// Read a `convention.retired`: `(id, reason, source)`.
pub fn convention_retired_of(p: &Value) -> Result<(String, String, Source), StoreError> {
    const E: &str = "convention.retired";
    Ok((
        str_of(p, E, "id")?,
        str_of(p, E, "reason")?,
        source_of(p, E, "source")?,
    ))
}

/// `decision.recorded {id, subject, decision_md, decided_by, decided_on,
/// evidence_attachment?, supersedes?, citation?, projects?}`: `projects`
/// only when it names any (journal feature `projects`), so a decision
/// without them writes the bytes it always did.
pub fn decision_recorded(id: u64, d: &NewDecision) -> Value {
    let mut o = Map::new();
    o.insert("id".into(), json!(id));
    o.insert("subject".into(), json!(d.subject));
    o.insert("decision_md".into(), json!(d.decision_md));
    o.insert("decided_by".into(), json!(d.decided_by));
    o.insert("decided_on".into(), json!(d.decided_on.to_string()));
    put(&mut o, "evidence_attachment", d.evidence_attachment);
    put(&mut o, "supersedes", d.supersedes);
    put(&mut o, "citation", d.citation.clone());
    if !d.projects.is_empty() {
        o.insert("projects".into(), json!(d.projects));
    }
    Value::Object(o)
}

/// Read a `decision.recorded`: `(id, decision)`.
pub fn decision_recorded_of(p: &Value) -> Result<(u64, NewDecision), StoreError> {
    const E: &str = "decision.recorded";
    let decided_by = p
        .get("decided_by")
        .and_then(Value::as_array)
        .and_then(|a| {
            a.iter()
                .map(|v| v.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()
        })
        .ok_or_else(|| corrupt(E, "decided_by"))?;
    let projects = match p.get("projects") {
        None | Some(Value::Null) => vec![],
        Some(v) => v
            .as_array()
            .and_then(|a| {
                a.iter()
                    .map(|x| x.as_u64().and_then(|n| ClassId::try_from(n).ok()))
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or_else(|| corrupt(E, "projects"))?,
    };
    Ok((
        u64_of(p, E, "id")?,
        NewDecision {
            subject: str_of(p, E, "subject")?,
            decision_md: str_of(p, E, "decision_md")?,
            decided_by,
            decided_on: date_of(&str_of(p, E, "decided_on")?, E, "decided_on")?,
            evidence_attachment: opt_u64(p, E, "evidence_attachment")?,
            supersedes: opt_u64(p, E, "supersedes")?,
            citation: opt_str(p, E, "citation")?,
            projects,
        },
    ))
}

/// `project.set {id, name, status, kind?, description?, party_id?,
/// started_on?, ended_on?, links?, notes?}`: a class's whole project
/// record (rules.md B-37; journal feature `projects`). Every optional key
/// is left out when empty, so the payload is canonical.
pub fn project_set(id: ClassId, name: &str, f: &ProjectFacts) -> Value {
    let mut o = Map::new();
    o.insert("id".into(), json!(id));
    o.insert("name".into(), json!(name));
    o.insert("status".into(), json!(f.status.key()));
    put(&mut o, "kind", f.kind.map(ProjectKind::key));
    put(&mut o, "description", f.description.clone());
    put(&mut o, "party_id", f.party);
    put(&mut o, "started_on", f.started_on.map(|d| d.to_string()));
    put(&mut o, "ended_on", f.ended_on.map(|d| d.to_string()));
    if !f.links.is_empty() {
        o.insert("links".into(), json!(f.links));
    }
    put(&mut o, "notes", f.notes.clone());
    Value::Object(o)
}

/// Read a `project.set`: `(id, name, facts)`. A missing or mistyped key is
/// corrupt; a status or kind this version doesn't know is newer (a newer
/// version wrote it, rules.md B-27).
pub fn project_set_of(
    line: &crate::journal::JournalLine,
) -> Result<(ClassId, String, ProjectFacts), StoreError> {
    const E: &str = "project.set";
    let p = &line.payload;
    let id = ClassId::try_from(u64_of(p, E, "id")?).map_err(|_| corrupt(E, "id"))?;
    let name = str_of(p, E, "name")?;
    let key = str_of(p, E, "status")?;
    let status = ProjectStatus::parse(&key)
        .ok_or_else(|| crate::format::newer_content(line, format!("the project status {key:?}")))?;
    let kind = match opt_str(p, E, "kind")? {
        None => None,
        Some(k) => Some(ProjectKind::parse(&k).ok_or_else(|| {
            crate::format::newer_content(line, format!("the project kind {k:?}"))
        })?),
    };
    let links = match p.get("links") {
        None | Some(Value::Null) => vec![],
        Some(v) => v
            .as_array()
            .and_then(|a| {
                a.iter()
                    .map(|x| x.as_str().map(str::to_string))
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or_else(|| corrupt(E, "links"))?,
    };
    Ok((
        id,
        name,
        ProjectFacts {
            description: opt_str(p, E, "description")?,
            status,
            kind,
            party: opt_u64(p, E, "party_id")?,
            started_on: opt_date(p, E, "started_on")?,
            ended_on: opt_date(p, E, "ended_on")?,
            links,
            notes: opt_str(p, E, "notes")?,
        },
    ))
}

/// `open_item.raised {id, title, detail_md?, due?, source}`.
pub fn open_item_raised(id: u64, i: &NewOpenItem) -> Value {
    let mut o = Map::new();
    o.insert("id".into(), json!(id));
    o.insert("title".into(), json!(i.title));
    put(&mut o, "detail_md", i.detail_md.clone());
    put(&mut o, "due", i.due.map(|d| d.to_string()));
    o.insert("source".into(), source_json(&i.source));
    Value::Object(o)
}

/// Read an `open_item.raised`: `(id, item)`.
pub fn open_item_raised_of(p: &Value) -> Result<(u64, NewOpenItem), StoreError> {
    const E: &str = "open_item.raised";
    Ok((
        u64_of(p, E, "id")?,
        NewOpenItem {
            title: str_of(p, E, "title")?,
            detail_md: opt_str(p, E, "detail_md")?,
            due: opt_date(p, E, "due")?,
            source: source_of(p, E, "source")?,
        },
    ))
}

/// `open_item.resolved {id, resolution, source}`.
pub fn open_item_resolved(id: u64, resolution: &str, source: &Source) -> Value {
    json!({ "id": id, "resolution": resolution, "source": source_json(source) })
}

/// Read an `open_item.resolved`: `(id, resolution, source)`.
pub fn open_item_resolved_of(p: &Value) -> Result<(u64, String, Source), StoreError> {
    const E: &str = "open_item.resolved";
    Ok((
        u64_of(p, E, "id")?,
        str_of(p, E, "resolution")?,
        source_of(p, E, "source")?,
    ))
}

/// `entry.approved {entry_id, approver, fingerprint, note?, source}`.
pub fn entry_approved(entry: u64, a: &Approval) -> Value {
    let mut o = Map::new();
    o.insert("entry_id".into(), json!(entry));
    o.insert("approver".into(), json!(a.approver));
    o.insert("fingerprint".into(), json!(a.fingerprint));
    put(&mut o, "note", a.note.clone());
    o.insert("source".into(), source_json(&a.source));
    Value::Object(o)
}

/// Read an `entry.approved` at `seq`: `(entry, approval)`.
pub fn entry_approved_of(p: &Value, seq: u64) -> Result<(u64, Approval), StoreError> {
    const E: &str = "entry.approved";
    Ok((
        u64_of(p, E, "entry_id")?,
        Approval {
            approver: str_of(p, E, "approver")?,
            fingerprint: str_of(p, E, "fingerprint")?,
            note: opt_str(p, E, "note")?,
            source: source_of(p, E, "source")?,
            seq,
        },
    ))
}

/// A list of strings, each checked (`to` on `invoice.sent`).
fn str_list(p: &Value, event: &str, k: &str) -> Result<Vec<String>, StoreError> {
    p.get(k)
        .and_then(Value::as_array)
        .ok_or_else(|| corrupt(event, k))?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_string)
                .ok_or_else(|| corrupt(event, k))
        })
        .collect()
}

/// `invoice.sent {invoice_id, on, by, via, to, kind, reference?, note?}`
/// (plan P4b S5): a send is a fact of the document's life and moves no
/// figure.
pub fn invoice_sent(invoice: u64, s: &Sent) -> Value {
    let mut o = Map::new();
    o.insert("invoice_id".into(), json!(invoice));
    o.insert("on".into(), json!(s.on.to_string()));
    o.insert("by".into(), json!(s.by));
    o.insert("via".into(), json!(s.via.key()));
    o.insert("to".into(), json!(s.to));
    o.insert("kind".into(), json!(s.kind.key()));
    put(&mut o, "reference", s.reference.clone());
    put(&mut o, "note", s.note.clone());
    Value::Object(o)
}

/// Read an `invoice.sent`: `(invoice, send)`. A channel or kind this
/// version doesn't know is newer, never corrupt (rules.md B-27).
pub fn invoice_sent_of(line: &JournalLine) -> Result<(u64, Sent), StoreError> {
    const E: &str = "invoice.sent";
    let p = &line.payload;
    let via = str_of(p, E, "via")?;
    let kind = str_of(p, E, "kind")?;
    Ok((
        u64_of(p, E, "invoice_id")?,
        Sent {
            on: date_of(&str_of(p, E, "on")?, E, "on")?,
            by: str_of(p, E, "by")?,
            via: SentVia::parse(&via).ok_or_else(|| {
                crate::format::newer_content(line, format!("the send channel {via:?}"))
            })?,
            to: str_list(p, E, "to")?,
            kind: SendKind::parse(&kind).ok_or_else(|| {
                crate::format::newer_content(line, format!("the send kind {kind:?}"))
            })?,
            reference: opt_str(p, E, "reference")?,
            note: opt_str(p, E, "note")?,
        },
    ))
}

/// A bad-debt write-off as journaled (plan P4b S7; rules.md B-41):
/// `invoice.written_off {invoice_id, entry_id, date, amount, reason}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WrittenOff {
    /// The invoice written off.
    pub invoice_id: u64,
    /// The entry that posted it (already rebuilt from its own events).
    pub entry_id: u64,
    /// The write-off's day.
    pub date: NaiveDate,
    /// What was written off.
    pub amount: rust_decimal::Decimal,
    /// Why (it has no resolution: the evidence is the receipt).
    pub reason: String,
}

/// `invoice.written_off {invoice_id, entry_id, date, amount, reason}`.
pub fn invoice_written_off(
    invoice_id: u64,
    entry_id: u64,
    date: NaiveDate,
    amount: rust_decimal::Decimal,
    reason: &str,
) -> Value {
    json!({
        "invoice_id": invoice_id,
        "entry_id": entry_id,
        "date": date.to_string(),
        "amount": amount.to_string(),
        "reason": reason,
    })
}

/// Read an `invoice.written_off` payload.
pub fn invoice_written_off_of(p: &Value) -> Result<WrittenOff, StoreError> {
    const E: &str = "invoice.written_off";
    let amount = str_of(p, E, "amount")?
        .parse()
        .map_err(|_| corrupt(E, "amount"))?;
    Ok(WrittenOff {
        invoice_id: u64_of(p, E, "invoice_id")?,
        entry_id: u64_of(p, E, "entry_id")?,
        date: date_of(&str_of(p, E, "date")?, E, "date")?,
        amount,
        reason: str_of(p, E, "reason")?,
    })
}

/// `attachment.text_extracted {attachment_id, text_sha256, extractor,
/// version, pages, status}` (plan P6, feature `attachment_text`).
pub fn text_extracted(
    attachment_id: u64,
    text_sha256: Option<&str>,
    extractor: &str,
    version: &str,
    pages: u32,
    status: &str,
) -> Value {
    json!({
        "attachment_id": attachment_id,
        "text_sha256": text_sha256,
        "extractor": extractor,
        "version": version,
        "pages": pages,
        "status": status,
    })
}

/// Read an `attachment.text_extracted`: `(attachment_id, text_sha256,
/// extractor, version, pages, status)`. A status this version doesn't know
/// is newer (B-27: never "corrupt" for newer).
pub fn text_extracted_of(line: &crate::journal::JournalLine) -> Result<TextExtracted, StoreError> {
    const E: &str = "attachment.text_extracted";
    let p = &line.payload;
    let status = str_of(p, E, "status")?;
    if !matches!(status.as_str(), "ok" | "none" | "failed") {
        return Err(crate::format::newer_content(
            line,
            format!("the extraction status {status:?}"),
        ));
    }
    Ok((
        u64_of(p, E, "attachment_id")?,
        opt_str(p, E, "text_sha256")?,
        str_of(p, E, "extractor")?,
        str_of(p, E, "version")?,
        u32::try_from(u64_of(p, E, "pages")?).map_err(|_| corrupt(E, "pages"))?,
        status,
    ))
}

/// What an `attachment.text_extracted` says: attachment id, text blob,
/// extractor, version, pages, status.
pub type TextExtracted = (u64, Option<String>, String, String, u32, String);

/// The annotation fields this version knows (the closed list, plan P6
/// step 5).
const ANNOTATION_FIELDS: &[&str] = &[
    "receipt_number",
    "payee",
    "amount",
    "date",
    "instrument",
    "category",
    "source",
    "reimbursable_to",
];

/// `attachment.annotated {attachment_id, fields{...}, note?}`: only the
/// fields given are written, so a canonical payload.
pub fn annotated(a: &Annotation) -> Value {
    let mut fields = Map::new();
    put(&mut fields, "receipt_number", a.fields.receipt_number);
    put(&mut fields, "payee", a.fields.payee.clone());
    put(
        &mut fields,
        "amount",
        a.fields.amount.map(|v| v.to_string()),
    );
    put(&mut fields, "date", a.fields.date.map(|d| d.to_string()));
    put(&mut fields, "instrument", a.fields.instrument.clone());
    put(&mut fields, "category", a.fields.category.clone());
    put(&mut fields, "source", a.fields.source.clone());
    put(
        &mut fields,
        "reimbursable_to",
        a.fields.reimbursable_to.clone(),
    );
    let mut o = Map::new();
    o.insert("attachment_id".into(), json!(a.attachment_id));
    o.insert("fields".into(), Value::Object(fields));
    put(&mut o, "note", a.note.clone());
    Value::Object(o)
}

/// Read an `attachment.annotated`. A field this version doesn't know, or a
/// mistyped one, is judged by its kind: unknown is newer (a later version
/// wrote it, B-27), mistyped is corrupt.
pub fn annotated_of(line: &crate::journal::JournalLine) -> Result<Annotation, StoreError> {
    const E: &str = "attachment.annotated";
    let p = &line.payload;
    let fields = p
        .get("fields")
        .filter(|v| v.is_object())
        .ok_or_else(|| corrupt(E, "fields"))?;
    for key in fields.as_object().expect("object").keys() {
        if !ANNOTATION_FIELDS.contains(&key.as_str()) {
            return Err(crate::format::newer_content(
                line,
                format!("the annotation field {key:?}"),
            ));
        }
    }
    let text = |k: &str| opt_str(fields, E, k);
    let receipt_number = match fields.get("receipt_number") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| corrupt(E, "receipt_number"))?,
        ),
    };
    let amount = match text("amount")? {
        None => None,
        Some(s) => Some(
            s.parse()
                .map_err(|_| StoreError::JournalCorrupt(format!("{E}: amount")))?,
        ),
    };
    let date = opt_date(fields, E, "date")?;
    Ok(Annotation {
        attachment_id: u64_of(p, E, "attachment_id")?,
        fields: AnnotationFields {
            receipt_number,
            payee: text("payee")?,
            amount,
            date,
            instrument: text("instrument")?,
            category: text("category")?,
            source: text("source")?,
            reimbursable_to: text("reimbursable_to")?,
        },
        note: opt_str(p, E, "note")?,
    })
}

/// `entry.settles {entry_id, settles: [entry_id]}` (plan P6).
pub fn entry_settles(entry: u64, settles: &[u64]) -> Value {
    json!({ "entry_id": entry, "settles": settles })
}

/// Read an `entry.settles`: `(entry, settles)`.
pub fn entry_settles_of(p: &Value) -> Result<(u64, Vec<u64>), StoreError> {
    const E: &str = "entry.settles";
    let settles = p
        .get("settles")
        .and_then(Value::as_array)
        .ok_or_else(|| corrupt(E, "settles"))?
        .iter()
        .map(|v| v.as_u64().ok_or_else(|| corrupt(E, "settles")))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((u64_of(p, E, "entry_id")?, settles))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledger_core::conventions::ConventionKind;

    fn src() -> Source {
        Source {
            decided_by: "A".into(),
            decided_on: NaiveDate::from_ymd_opt(2026, 10, 17).unwrap(),
            citation: Some("c".into()),
            note: None,
        }
    }

    /// GOAL: every event round-trips.
    #[test]
    fn round_trips() {
        let ch = vec![
            Change {
                key: "company.name".into(),
                value: Some(json!("X")),
            },
            Change {
                key: "books.close_into".into(),
                value: None,
            },
        ];
        assert_eq!(
            settings_set_of(&settings_set(&ch, &src())).unwrap(),
            (ch, src())
        );
        let c = NewConvention {
            id: "t".into(),
            kind: ConventionKind::PostingTemplate,
            title: "T".into(),
            body_md: "B".into(),
            structured: Some(json!({"lines": []})),
            source: src(),
        };
        assert_eq!(convention_set_of(&convention_set(&c)).unwrap(), c);
        assert_eq!(
            convention_retired_of(&convention_retired("t", "r", &src())).unwrap(),
            ("t".into(), "r".into(), src())
        );
        let d = NewDecision {
            subject: "S".into(),
            decision_md: "D".into(),
            decided_by: vec!["A".into(), "B".into()],
            decided_on: src().decided_on,
            evidence_attachment: Some(7),
            supersedes: None,
            citation: None,
            projects: vec![],
        };
        assert_eq!(
            decision_recorded_of(&decision_recorded(3, &d)).unwrap(),
            (3, d)
        );
        let i = NewOpenItem {
            title: "T".into(),
            detail_md: Some("d".into()),
            due: Some(src().decided_on),
            source: src(),
        };
        assert_eq!(
            open_item_raised_of(&open_item_raised(4, &i)).unwrap(),
            (4, i)
        );
        assert_eq!(
            open_item_resolved_of(&open_item_resolved(4, "done", &src())).unwrap(),
            (4, "done".into(), src())
        );
        let a = Approval {
            approver: "A".into(),
            fingerprint: "00ff".into(),
            note: None,
            source: src(),
            seq: 9,
        };
        assert_eq!(
            entry_approved_of(&entry_approved(5, &a), 9).unwrap(),
            (5, a)
        );
        let line = crate::journal::JournalLine {
            seq: 11,
            ts: "2026-10-27T00:00:00Z".into(),
            line_type: "attachment.annotated".into(),
            payload: annotated(&Annotation {
                attachment_id: 3,
                fields: AnnotationFields {
                    receipt_number: Some(7),
                    payee: Some("Harbor Design".into()),
                    amount: Some("12.34".parse().unwrap()),
                    date: NaiveDate::from_ymd_opt(2026, 3, 1),
                    instrument: Some("Check 1234".into()),
                    category: None,
                    source: Some("a folder".into()),
                    reimbursable_to: Some("Avery Quill".into()),
                },
                note: Some("read on 2026-03-02".into()),
            }),
            prev: String::new(),
        };
        let read = annotated_of(&line).unwrap();
        assert_eq!(annotated(&read), line.payload);
        assert_eq!(
            entry_settles_of(&entry_settles(4, &[1, 2])).unwrap(),
            (4, vec![1, 2])
        );
        let x = crate::journal::JournalLine {
            seq: 12,
            ts: "2026-10-27T00:00:00Z".into(),
            line_type: "attachment.text_extracted".into(),
            payload: text_extracted(3, Some("00ff"), "wanderbooks", "1", 2, "ok"),
            prev: String::new(),
        };
        assert_eq!(
            text_extracted_of(&x).unwrap(),
            (
                3,
                Some("00ff".into()),
                "wanderbooks".into(),
                "1".into(),
                2,
                "ok".into()
            )
        );
    }

    /// INVARIANT 1.
    #[test]
    fn bad_shapes_are_corrupt() {
        assert!(
            settings_set_of(&json!({"changes": [{"key": "k"}], "source": source_json(&src())}))
                .is_err()
        );
        assert!(settings_set_of(&json!({"changes": []})).is_err());
        assert!(convention_set_of(&json!({"id": "t", "kind": "nope", "title": "T", "body_md": "B", "source": source_json(&src())})).is_err());
        assert!(decision_recorded_of(&json!({"id": 1, "subject": "S", "decision_md": "D", "decided_by": [1], "decided_on": "2026-10-17"})).is_err());
        assert!(
            open_item_raised_of(
                &json!({"id": 1, "title": "T", "due": 5, "source": source_json(&src())})
            )
            .is_err()
        );
    }
}
