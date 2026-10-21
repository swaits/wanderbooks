//! Projects in the journal (rules.md B-37): `class.added`, then the whole
//! record as `project.set` (journal feature `projects`); links to a
//! project and decisions naming one wait for the feature too.
//!
//! ```text
//! GOAL: a project's record is journaled once, whole, replays exactly,
//!       and books waiting for their format upgrade can still add
//!       projects by name but write nothing an older reader would misread.
//! SOURCE: docs/spec/rules.md B-37, B-27; D-39 (the owner's upgrade).
//! ```

use chrono::NaiveDate;
use ledger_core::conventions::NewDecision;
use ledger_core::{Currency, ProjectFacts, ProjectKind, ProjectStatus};
use ledger_store::{
    JOURNAL_FILE, JournalFormat, JournalWriter, Snapshot, Store, StoreError, Subject,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

struct TestDir(PathBuf);

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn test_dir(name: &str) -> TestDir {
    let dir = std::env::temp_dir().join(format!(
        "wanderbooks-projects-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn facts() -> ProjectFacts {
    ProjectFacts {
        description: Some("An app for the harbor".into()),
        kind: Some(ProjectKind::OnSpec),
        started_on: Some(d(2026, 8, 1)),
        links: vec!["https://harbor.example/app".into()],
        ..Default::default()
    }
}

/// Every journal line, as (type, payload).
fn lines(dir: &Path) -> Vec<(String, Value)> {
    std::fs::read_to_string(dir.join(JOURNAL_FILE))
        .unwrap()
        .lines()
        .map(|l| {
            let v: Value = serde_json::from_str(l).unwrap();
            (
                v["type"].as_str().unwrap().to_string(),
                v["payload"].clone(),
            )
        })
        .collect()
}

fn decision(projects: Vec<u32>) -> NewDecision {
    NewDecision {
        subject: "Harbor".into(),
        decision_md: "Build it on spec.".into(),
        decided_by: vec!["Tobin Harlow".into()],
        decided_on: d(2026, 8, 1),
        evidence_attachment: None,
        supersedes: None,
        citation: None,
        projects,
    }
}

#[test]
fn add_project_journals_class_added_then_project_set() {
    let dir = test_dir("add");
    let mut s = Store::create(&dir.0, Currency::USD).unwrap();
    let id = s.add_project("  Harbor app ", &facts()).unwrap();
    let l = lines(&dir.0);
    let n = l.len();
    assert_eq!(
        l[n - 2],
        (
            "class.added".into(),
            json!({"id": id, "name": "Harbor app"})
        )
    );
    assert_eq!(
        l[n - 1],
        (
            "project.set".into(),
            json!({"id": id, "name": "Harbor app", "status": "active", "kind": "on_spec",
                   "description": "An app for the harbor", "started_on": "2026-08-01",
                   "links": ["https://harbor.example/app"]})
        )
    );
    assert_eq!(s.project_facts(id), Some(facts()));
    // A refused add journals nothing: a taken name.
    let head = s.head();
    assert!(s.add_project("HARBOR APP", &facts()).is_err());
    assert!(
        s.add_project(
            "Dock",
            &ProjectFacts {
                ended_on: Some(d(2026, 1, 1)),
                ..Default::default()
            }
        )
        .is_err()
    );
    assert_eq!(s.head(), head);
}

#[test]
fn a_name_only_project_journals_class_added_alone() {
    let dir = test_dir("name-only");
    let mut s = Store::create(&dir.0, Currency::USD).unwrap();
    let before = lines(&dir.0).len();
    let id = s.add_project("Harbor", &ProjectFacts::default()).unwrap();
    let l = lines(&dir.0);
    assert_eq!(l.len(), before + 1);
    assert_eq!(l[before].0, "class.added");
    assert_eq!(s.project_facts(id), Some(ProjectFacts::default()));
}

#[test]
fn project_writes_wait_for_the_upgrade_and_journal_nothing() {
    let dir = test_dir("upgrade");
    let mut s = Store::create_in_format(&dir.0, Currency::USD, &JournalFormat::default()).unwrap();
    // A name alone is a plain class: accepted.
    let id = s.add_project("Harbor", &ProjectFacts::default()).unwrap();
    let (att, _, _) = s
        .file_document("other", "pitch.pdf", "application/pdf", b"%PDF pitch", None)
        .unwrap();
    let head = s.head();
    let waits = |r: Result<(), StoreError>| match r {
        Err(StoreError::FormatUpgradePending { need }) => need,
        other => panic!("expected FormatUpgradePending, got {other:?}"),
    };
    let projects = "the journal feature projects";
    assert_eq!(waits(s.add_project("Dock", &facts()).map(|_| ())), projects);
    assert_eq!(
        waits(s.set_project(id, "Harbor", &facts()).map(|_| ())),
        projects
    );
    assert_eq!(
        waits(s.link_attachment(att, Subject::Project(u64::from(id)))),
        projects
    );
    // A decision waits for format 2 itself on these books; with
    // projects it waits for that feature too.
    waits(s.record_decision(&decision(vec![id])).map(|_| ()));
    assert_eq!(s.head(), head, "nothing journaled");
    assert!(s.upgrade_format().unwrap().is_some());
    s.add_project("Dock", &facts()).unwrap();
    assert!(s.set_project(id, "Harbor", &facts()).unwrap());
    s.link_attachment(att, Subject::Project(u64::from(id)))
        .unwrap();
    s.record_decision(&decision(vec![id])).unwrap();
    s.verify().unwrap();
}

/// The live books after this version is installed: format 2 with every
/// earlier feature but not `projects`. The format check passes; only the
/// feature check refuses, and nothing is written.
#[test]
fn format_2_books_without_the_feature_wait_for_it_alone() {
    let dir = test_dir("no-feature");
    let mut format = ledger_store::format::current();
    format
        .features
        .retain(|f| f != ledger_store::format::PROJECTS);
    assert_eq!(format.format, 2);
    let mut s = Store::create_in_format(&dir.0, Currency::USD, &format).unwrap();
    let id = s.add_project("Harbor", &ProjectFacts::default()).unwrap();
    let (att, _, _) = s
        .file_document("other", "pitch.pdf", "application/pdf", b"%PDF pitch", None)
        .unwrap();
    // A decision naming no project is format 2's: accepted.
    s.record_decision(&decision(vec![])).unwrap();
    let head = s.head();
    let waits = |r: Result<(), StoreError>| match r {
        Err(StoreError::FormatUpgradePending { need }) => need,
        other => panic!("expected FormatUpgradePending, got {other:?}"),
    };
    let projects = "the journal feature projects";
    assert_eq!(waits(s.add_project("Dock", &facts()).map(|_| ())), projects);
    assert_eq!(
        waits(s.set_project(id, "Harbor", &facts()).map(|_| ())),
        projects
    );
    assert_eq!(
        waits(s.link_attachment(att, Subject::Project(u64::from(id)))),
        projects
    );
    assert_eq!(
        waits(s.record_decision(&decision(vec![id])).map(|_| ())),
        projects
    );
    assert_eq!(s.head(), head, "nothing journaled");
    // A name alone is still a plain class.
    s.add_project("Dock", &ProjectFacts::default()).unwrap();
    s.verify().unwrap();
}

/// `class.added` lines appended as an earlier version wrote them: no
/// uniqueness and no trimming.
fn old_classes(dir: &Path, names: &[&str]) {
    let s = Store::create(dir, Currency::USD).unwrap();
    drop(s);
    let mut w = JournalWriter::open(&dir.join(JOURNAL_FILE)).unwrap();
    for (i, n) in names.iter().enumerate() {
        w.append("class.added", json!({"id": i + 1, "name": n}))
            .unwrap();
    }
}

#[test]
fn old_books_with_a_duplicate_name_change_either_project() {
    let dir = test_dir("dup");
    old_classes(&dir.0, &["Alpha", "alpha", "Harbor"]);
    let mut s = Store::open(&dir.0).unwrap();
    let paused = ProjectFacts {
        status: ProjectStatus::Paused,
        ..Default::default()
    };
    // A fact-only change to each keeps its name.
    assert!(s.set_project(1, "Alpha", &paused).unwrap());
    assert!(s.set_project(2, "alpha", &paused).unwrap());
    // Its own name up to capitals and space is still its own.
    assert!(s.set_project(2, " ALPHA ", &paused).unwrap());
    assert_eq!(s.list_classes()[1].name, "ALPHA");
    // A rename onto another project's name is still refused.
    let head = s.head();
    assert!(matches!(
        s.set_project(1, "harbor", &paused),
        Err(StoreError::Ledger(
            ledger_core::LedgerError::DuplicateClassName(_)
        ))
    ));
    assert_eq!(s.head(), head);
    // A rename away from the duplicate is accepted.
    assert!(s.set_project(2, "Alpha two", &paused).unwrap());
    s.verify().unwrap();
}

#[test]
fn a_fact_only_change_never_renames_an_untrimmed_old_name() {
    let dir = test_dir("untrimmed");
    old_classes(&dir.0, &["Retail "]);
    let mut s = Store::open(&dir.0).unwrap();
    let head = s.head();
    // Its own name, exactly: unchanged, nothing journaled.
    assert!(
        !s.set_project(1, "Retail ", &ProjectFacts::default())
            .unwrap()
    );
    assert_eq!(s.head(), head);
    let paused = ProjectFacts {
        status: ProjectStatus::Paused,
        ..Default::default()
    };
    assert!(s.set_project(1, "Retail ", &paused).unwrap());
    let l = lines(&dir.0);
    assert_eq!(
        l.last().unwrap().1["name"],
        "Retail ",
        "no rename journaled"
    );
    assert_eq!(s.list_classes()[0].name, "Retail ");
    s.verify().unwrap();
}

#[test]
fn update_with_nothing_new_journals_nothing() {
    let dir = test_dir("same");
    let mut s = Store::create(&dir.0, Currency::USD).unwrap();
    let id = s.add_project("Harbor", &facts()).unwrap();
    let head = s.head();
    assert!(!s.set_project(id, " Harbor ", &facts()).unwrap());
    assert_eq!(s.head(), head);
    let mut f = facts();
    f.status = ProjectStatus::Paused;
    assert!(s.set_project(id, "Harbor", &f).unwrap());
    assert_ne!(s.head(), head);
}

#[test]
fn project_set_replays_verifies_and_reloads() {
    let dir = test_dir("replay");
    let mut s = Store::create(&dir.0, Currency::USD).unwrap();
    let party = s.add_party("Harbor Co", &[]).unwrap();
    let id = s.add_project("Harbor", &facts()).unwrap();
    let mut f = facts();
    f.party = Some(party);
    f.status = ProjectStatus::Finished;
    f.ended_on = Some(d(2026, 9, 30));
    s.set_project(id, "Harbor app", &f).unwrap();
    s.verify().unwrap();
    drop(s);
    let s = Store::open(&dir.0).unwrap();
    s.verify().unwrap();
    assert_eq!(s.project_facts(id), Some(f.clone()));
    assert_eq!(s.list_classes()[0].name, "Harbor app");
    drop(s);
    let snap = Snapshot::load(&dir.0).unwrap();
    assert_eq!(snap.ledger().project_facts(id), Some(f));
    assert_eq!(snap.ledger().projects_of_party(party), vec![id]);
}

#[test]
fn an_unknown_status_or_kind_is_newer_a_malformed_record_corrupt() {
    for (case, payload, newer) in [
        (
            "status",
            json!({"id": 1, "name": "Harbor", "status": "on_hold"}),
            true,
        ),
        (
            "kind",
            json!({"id": 1, "name": "Harbor", "status": "active", "kind": "grant"}),
            true,
        ),
        ("no-status", json!({"id": 1, "name": "Harbor"}), false),
        (
            "links",
            json!({"id": 1, "name": "Harbor", "status": "active", "links": "x"}),
            false,
        ),
        (
            "date",
            json!({"id": 1, "name": "Harbor", "status": "active", "started_on": "soon"}),
            false,
        ),
    ] {
        let dir = test_dir(&format!("bad-{case}"));
        let mut s = Store::create(&dir.0, Currency::USD).unwrap();
        s.add_class("Harbor").unwrap();
        drop(s);
        let mut w = JournalWriter::open(&dir.0.join(JOURNAL_FILE)).unwrap();
        w.append("project.set", payload).unwrap();
        drop(w);
        match Store::open(&dir.0) {
            Err(StoreError::NewerJournal { .. }) if newer => {}
            Err(StoreError::JournalCorrupt(_)) if !newer => {}
            Err(e) => panic!("{case}: {e:?}"),
            Ok(_) => panic!("{case}: opened"),
        }
    }
}

#[test]
fn a_decision_without_projects_writes_the_bytes_it_always_did() {
    let d0 = decision(vec![]);
    let v = ledger_store::meta::decision_recorded(1, &d0);
    assert!(v.get("projects").is_none());
    assert_eq!(
        v,
        json!({"id": 1, "subject": "Harbor", "decision_md": "Build it on spec.",
               "decided_by": ["Tobin Harlow"], "decided_on": "2026-08-01"})
    );
    let (_, back) = ledger_store::meta::decision_recorded_of(&v).unwrap();
    assert_eq!(back, d0);
    let with = ledger_store::meta::decision_recorded(1, &decision(vec![3, 1]));
    assert_eq!(with["projects"], json!([3, 1]));
    assert_eq!(
        ledger_store::meta::decision_recorded_of(&with)
            .unwrap()
            .1
            .projects,
        vec![3, 1]
    );
    let mut bad = with.clone();
    bad["projects"] = json!(["x"]);
    assert!(ledger_store::meta::decision_recorded_of(&bad).is_err());
}

#[test]
fn attachments_link_to_a_project_and_unlink() {
    let dir = test_dir("links");
    let mut s = Store::create(&dir.0, Currency::USD).unwrap();
    let id = s.add_project("Harbor", &ProjectFacts::default()).unwrap();
    let (att, _, _) = s
        .file_document(
            "contract",
            "spec.pdf",
            "application/pdf",
            b"%PDF spec",
            None,
        )
        .unwrap();
    let sub = Subject::Project(u64::from(id));
    assert_eq!(Store::subject_feature(sub), Some("projects"));
    assert_eq!(Store::subject_feature(Subject::Party(1)), None);
    assert!(s.subject_exists(sub).unwrap());
    assert!(!s.subject_exists(Subject::Project(99)).unwrap());
    assert!(!s.subject_exists(Subject::Project(u64::MAX)).unwrap());
    assert!(s.link_attachment(att, Subject::Project(99)).is_err());
    s.link_attachment(att, sub).unwrap();
    assert_eq!(s.attachments_for(sub).unwrap()[0].id, att);
    assert!(s.attachment_links(att).unwrap().contains(&sub));
    drop(s);
    let mut s = Store::open(&dir.0).unwrap();
    s.verify().unwrap();
    s.unlink_attachment(att, sub, "wrong project").unwrap();
    assert!(s.attachments_for(sub).unwrap().is_empty());
    s.verify().unwrap();
}

#[test]
fn old_books_open_to_their_figures_with_projects_known() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/old-books");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    assert!(!dirs.is_empty());
    for fixture in dirs {
        let name = fixture.file_name().unwrap().to_string_lossy().to_string();
        let dir = test_dir(&format!("old-{name}"));
        copy(&fixture, &dir.0);
        let journal = std::fs::read(dir.0.join(JOURNAL_FILE)).unwrap();
        let expected: Value =
            serde_json::from_slice(&std::fs::read(fixture.join("expected.json")).unwrap()).unwrap();
        let s = Store::open(&dir.0).unwrap_or_else(|e| panic!("{name}: refused: {e}"));
        // The figures that version computed, unchanged by knowing projects.
        assert_eq!(
            json!(ledger_core::reports::snapshot(s.ledger()).map_err(|e| e.to_string())),
            expected["snapshot"],
            "{name}: figures"
        );
        assert_eq!(
            json!(s.verify().map(|_| ()).map_err(|e| e.to_string())),
            expected["verify"],
            "{name}: verify"
        );
        for c in s.list_classes() {
            assert_eq!(
                s.project_facts(c.id),
                Some(ProjectFacts::default()),
                "{name}"
            );
        }
        assert!(
            !lines(&dir.0).iter().any(|(t, _)| t == "project.set"),
            "{name}"
        );
        drop(s);
        assert_eq!(
            std::fs::read(dir.0.join(JOURNAL_FILE)).unwrap(),
            journal,
            "{name}: opening journals nothing"
        );
    }
}

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        if e.file_name() == "expected.json" || e.file_name() == "record.json" {
            continue;
        }
        if e.file_type().unwrap().is_dir() {
            copy(&e.path(), &to.join(e.file_name()));
        } else {
            std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
        }
    }
}

proptest::proptest! {
    /// Any record a project can hold reads back as written.
    #[test]
    fn prop_project_set_round_trips(
        status in 0usize..4,
        kind in proptest::option::of(0usize..4),
        party in proptest::option::of(1u64..1000),
        desc in proptest::option::of("[a-z ]{1,20}"),
        start in proptest::option::of(0u32..3000),
        links in proptest::collection::vec("[a-z]{1,10}", 0..8),
    ) {
        let f = ProjectFacts {
            description: desc,
            status: ProjectStatus::ALL[status],
            kind: kind.map(|k| ProjectKind::ALL[k]),
            party,
            started_on: start.map(|n| d(2020, 1, 1) + chrono::Days::new(u64::from(n))),
            ended_on: None,
            links,
            notes: None,
        };
        let payload = ledger_store::meta::project_set(7, "Harbor", &f);
        let line = ledger_store::journal::JournalLine {
            seq: 1,
            ts: String::new(),
            line_type: "project.set".into(),
            payload,
            prev: String::new(),
        };
        let back = ledger_store::meta::project_set_of(&line).unwrap();
        proptest::prop_assert_eq!(back, (7, "Harbor".to_string(), f));
    }
}
