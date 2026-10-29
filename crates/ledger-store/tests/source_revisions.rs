//! Source revisions (plan P5, journal feature `source_revisions`): who
//! decided a convention, a decision or an open item can be corrected, with
//! why, without touching its words. The correction is journaled as its own
//! event, replays identically, and waits for the owner's format upgrade.
//!
//! Expected behavior comes from `docs/receipt-store.md` (the three events)
//! and rules.md B-27 (a feature is declared before its first line).

use chrono::NaiveDate;
use ledger_core::Currency;
use ledger_core::conventions::{
    ConventionKind, DecisionSource, NewConvention, NewDecision, NewOpenItem,
};
use ledger_core::settings::Source;
use ledger_store::format::{KNOWN_FEATURES, SOURCE_REVISIONS};
use ledger_store::{JournalFormat, SUPPORTED_JOURNAL_FORMAT, Store, StoreError};
use std::path::PathBuf;

struct TestDir(PathBuf);

impl std::ops::Deref for TestDir {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn test_dir(name: &str) -> TestDir {
    let dir = std::env::temp_dir().join(format!("wanderbooks-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

fn lines_of(dir: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(dir.join(ledger_store::JOURNAL_FILE))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
}

fn source(by: &str, d: u32, citation: Option<&str>) -> Source {
    Source {
        decided_by: by.into(),
        decided_on: day(d),
        citation: citation.map(str::to_string),
        note: None,
    }
}

fn convention(src: Source) -> NewConvention {
    NewConvention {
        id: "close-checklist".into(),
        kind: ConventionKind::CloseStep,
        title: "Close checklist".into(),
        body_md: "Reconcile, then close.".into(),
        structured: None,
        source: src,
    }
}

fn decision() -> NewDecision {
    NewDecision {
        subject: "Materiality".into(),
        decision_md: "Five hundred dollars.".into(),
        decided_by: vec!["Owner".into()],
        decided_on: day(20),
        evidence_attachment: None,
        supersedes: None,
        citation: None,
        projects: vec![],
    }
}

fn item(src: Source) -> NewOpenItem {
    NewOpenItem {
        title: "Ask the CPA".into(),
        detail_md: None,
        due: None,
        source: src,
    }
}

/// Done-when (plan P5 step 5): a call that changes only the source is
/// journaled as the `*.source_revised` event, the words and the revision
/// stay, the live books and a replay agree, and the reopened books verify.
#[test]
fn a_source_only_revision_is_journaled() {
    let dir = test_dir("source-revised");
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    s.set_convention(&convention(source("Owner", 20, None)))
        .unwrap();
    let decision_id = s.record_decision(&decision()).unwrap();
    let item_id = s.raise_open_item(&item(source("Owner", 20, None))).unwrap();
    let before = lines_of(&dir).len();

    // A convention: the same words, a corrected source.
    let cpa = source("Pat Smith", 21, Some("IRS Pub. 538"));
    s.revise_convention_source(&convention(cpa.clone()), "the CPA decided it")
        .unwrap();
    s.revise_decision_source(
        decision_id,
        &DecisionSource {
            decided_by: vec!["Owner".into(), "Pat Smith".into()],
            decided_on: day(21),
            citation: Some("IRS Pub. 538".into()),
        },
        "both decided it",
    )
    .unwrap();
    s.revise_open_item_source(item_id, &cpa, "raised by the CPA")
        .unwrap();

    let lines = lines_of(&dir);
    let types: Vec<&str> = lines[before..]
        .iter()
        .map(|l| l["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        types,
        [
            "convention.source_revised",
            "decision.source_revised",
            "open_item.source_revised"
        ]
    );
    assert_eq!(lines[before]["payload"]["reason"], "the CPA decided it");
    assert_eq!(
        lines[before]["payload"]["source"]["decided_by"],
        "Pat Smith"
    );

    // The words and the revision are as they were; the source is new.
    let c = s
        .ledger()
        .conventions()
        .convention("close-checklist")
        .unwrap();
    assert_eq!(
        (c.body_md.as_str(), c.revision),
        ("Reconcile, then close.", 1)
    );
    assert_eq!(c.source.decided_by, "Pat Smith");
    let d = s.ledger().conventions().decision(decision_id).unwrap();
    assert_eq!(d.new.decision_md, "Five hundred dollars.");
    assert_eq!(d.new.decided_by, ["Owner", "Pat Smith"]);
    assert_eq!(d.new.citation.as_deref(), Some("IRS Pub. 538"));
    assert_eq!(
        s.ledger()
            .conventions()
            .open_item(item_id)
            .unwrap()
            .new
            .source,
        cpa
    );

    // Replay equals the live books, and the database file agrees.
    let live = s.ledger().clone();
    drop(s);
    let reopened = Store::open(&dir).unwrap();
    reopened.verify().unwrap();
    assert!(live.differs_from(reopened.ledger()).is_empty());
    drop(reopened);
    let _ = std::fs::remove_file(dir.join(ledger_store::db_file()));
    let rebuilt = Store::open(&dir).unwrap();
    rebuilt.verify().unwrap();
    assert!(live.differs_from(rebuilt.ledger()).is_empty());
}

/// A correction must be one: other words, the same source, a retired
/// convention or a missing reason is refused and journals nothing.
#[test]
fn a_source_revision_must_change_only_the_source() {
    let dir = test_dir("source-refused");
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    s.set_convention(&convention(source("Owner", 20, None)))
        .unwrap();
    let id = s.record_decision(&decision()).unwrap();
    let item_id = s.raise_open_item(&item(source("Owner", 20, None))).unwrap();
    let len = lines_of(&dir).len();
    let mut other_words = convention(source("Pat Smith", 21, None));
    other_words.body_md = "Close first.".into();
    for refused in [
        // other words
        s.revise_convention_source(&other_words, "why"),
        // the same source
        s.revise_convention_source(&convention(source("Owner", 20, None)), "why"),
        // no reason
        s.revise_convention_source(&convention(source("Pat Smith", 21, None)), "  "),
        // no such convention, decision, item
        s.revise_convention_source(
            &NewConvention {
                id: "nonesuch".into(),
                ..convention(source("Pat Smith", 21, None))
            },
            "why",
        ),
        s.revise_decision_source(99, &decision().source(), "why"),
        s.revise_decision_source(id, &decision().source(), "same source"),
        s.revise_decision_source(
            id,
            &DecisionSource {
                decided_by: vec![],
                decided_on: day(21),
                citation: None,
            },
            "why",
        ),
        s.revise_open_item_source(99, &source("Pat Smith", 21, None), "why"),
        s.revise_open_item_source(item_id, &source("Owner", 20, None), "same source"),
    ] {
        assert!(matches!(refused, Err(StoreError::Ledger(_))), "{refused:?}");
    }
    assert_eq!(lines_of(&dir).len(), len, "a refusal journals nothing");
}

/// R-A step 3 (plan P5): books that have not declared `source_revisions`
/// refuse the three corrections, journaling nothing, with the format
/// upgrade pending; after the owner's go-ahead they journal.
#[test]
fn source_revisions_wait_for_the_upgrade() {
    let dir = test_dir("source-wait");
    // The version before P5: format 2, every feature but this one.
    let before = JournalFormat {
        format: SUPPORTED_JOURNAL_FORMAT,
        features: KNOWN_FEATURES
            .iter()
            .copied()
            .filter(|f| *f != SOURCE_REVISIONS)
            .map(str::to_string)
            .collect(),
    };
    let mut s = Store::create_in_format(&dir, Currency::USD, &before).unwrap();
    s.set_convention(&convention(source("Owner", 20, None)))
        .unwrap();
    let id = s.record_decision(&decision()).unwrap();
    let item_id = s.raise_open_item(&item(source("Owner", 20, None))).unwrap();
    let head = s.head();
    let len = lines_of(&dir).len();
    let cpa = source("Pat Smith", 21, None);
    for refused in [
        s.revise_convention_source(&convention(cpa.clone()), "why"),
        s.revise_decision_source(
            id,
            &DecisionSource {
                decided_by: vec!["Pat Smith".into()],
                decided_on: day(21),
                citation: None,
            },
            "why",
        ),
        s.revise_open_item_source(item_id, &cpa, "why"),
    ] {
        match refused {
            Err(StoreError::FormatUpgradePending { need }) => {
                assert_eq!(need, "the journal feature source_revisions")
            }
            other => panic!("expected FormatUpgradePending, got {other:?}"),
        }
    }
    assert_eq!(s.head(), head, "a refusal journals nothing");
    assert_eq!(lines_of(&dir).len(), len);
    assert!(s.upgrade_format().unwrap().is_some(), "the go-ahead");
    s.revise_convention_source(&convention(cpa.clone()), "why")
        .unwrap();
    s.revise_open_item_source(item_id, &cpa, "why").unwrap();
    let live = s.ledger().clone();
    drop(s);
    let reopened = Store::open(&dir).unwrap();
    reopened.verify().unwrap();
    assert!(live.differs_from(reopened.ledger()).is_empty());
}
