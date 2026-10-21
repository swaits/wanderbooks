//! Projects in the viewer (rules.md B-37): the list and a project's page
//! show ledger-core's figures; rows, links and bars open what they name;
//! the journal, books.show and the party page know projects.

use chrono::NaiveDate;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ledger_core::conventions::NewDecision;
use ledger_core::{AccountType, ClassId, Currency, Money, ProjectFacts, ProjectKind, Side};
use ledger_gui::books::Books;
use ledger_gui::nav::{Route, Section};
use ledger_gui::pages::journal::describe;
use ledger_gui::pages::projects::listed;
use ledger_gui::{App, Source, data};
use ledger_store::ops::ViewerTarget;
use ledger_store::{Store, Subject};
use rust_decimal::Decimal;
use std::path::PathBuf;

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn cents(c: i64) -> Decimal {
    Decimal::new(c, 2)
}

fn usd(c: i64) -> Money {
    Money::new(cents(c), Currency::USD)
}

fn today() -> NaiveDate {
    d(2026, 10, 15)
}

/// Books with three projects: "Zeta site" (client work for Riley Chen:
/// $5,000.00 earned, $1,800.00 of contractors), "alpha app" (on spec:
/// $300.00 of costs, and test devices bought for it, an asset), and
/// "Beta" (a bare class); rent is no project's. A pitch is linked to
/// alpha, and a decision names it.
struct Fixture {
    _dir: TempDir,
    books: Books,
    zeta: ClassId,
    alpha: ClassId,
    beta: ClassId,
    riley: u64,
    alpha_cost: u64,
    pitch: u64,
}

fn fixture(tag: &str) -> Fixture {
    let dir = TempDir(
        std::env::temp_dir().join(format!("ledger-gui-projects-{tag}-{}", std::process::id())),
    );
    let _ = std::fs::remove_dir_all(&dir.0);
    let data_dir = dir.0.join("books-data");
    let mut s = Store::create(&data_dir, Currency::USD).unwrap();
    let mut add = |code: &str, name: &str, ty| s.add_account(code, name, ty, None, false).unwrap();
    let cash = add("1000", "Cash", AccountType::Asset);
    let devices = add("1200", "Equipment", AccountType::Asset);
    let revenue = add("4000", "Design Services", AccountType::Revenue);
    let contractors = add("6000", "Contractors", AccountType::Expense);
    let rent = add("6200", "Rent", AccountType::Expense);
    let riley = s.add_party("Riley Chen", &[]).unwrap();
    let zeta = s
        .add_project(
            "Zeta site",
            &ProjectFacts {
                kind: Some(ProjectKind::Client),
                party: Some(riley),
                description: Some("A site for Riley".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let alpha = s
        .add_project(
            "alpha app",
            &ProjectFacts {
                kind: Some(ProjectKind::OnSpec),
                links: vec!["https://alpha.example".into()],
                ..Default::default()
            },
        )
        .unwrap();
    let beta = s.add_class("Beta").unwrap();
    let post = |s: &mut Store, date, what: &str, dr, cr, c, tag: Option<(ClassId, bool)>| {
        let e = s.create_entry(date, what).unwrap();
        s.add_line(e, dr, Side::Debit, usd(c)).unwrap();
        s.add_line(e, cr, Side::Credit, usd(c)).unwrap();
        if let Some((class, debit)) = tag {
            s.set_line_class(e, if debit { 0 } else { 1 }, Some(class))
                .unwrap();
        }
        s.post_entry(e).unwrap();
        e
    };
    post(
        &mut s,
        d(2026, 3, 2),
        "Zeta milestone",
        cash,
        revenue,
        500_000,
        Some((zeta, false)),
    );
    post(
        &mut s,
        d(2026, 3, 9),
        "Zeta contractor",
        contractors,
        cash,
        180_000,
        Some((zeta, true)),
    );
    let alpha_cost = post(
        &mut s,
        d(2026, 4, 1),
        "Alpha prototype",
        contractors,
        cash,
        30_000,
        Some((alpha, true)),
    );
    post(
        &mut s,
        d(2026, 4, 2),
        "Alpha test devices",
        devices,
        cash,
        90_000,
        Some((alpha, true)),
    );
    post(&mut s, d(2026, 4, 30), "Rent", rent, cash, 150_000, None);
    let (pitch, _, _) = s
        .file_document(
            "other",
            "alpha pitch.pdf",
            "application/pdf",
            b"%PDF-1.4 pitch",
            None,
        )
        .unwrap();
    s.link_attachment(pitch, Subject::Project(u64::from(alpha)))
        .unwrap();
    s.record_decision(&NewDecision {
        subject: "Alpha on spec".into(),
        decision_md: "Build it on spec.".into(),
        decided_by: vec!["Tobin Harlow".into()],
        decided_on: d(2026, 4, 1),
        evidence_attachment: None,
        supersedes: None,
        citation: None,
        projects: vec![alpha],
    })
    .unwrap();
    drop(s);
    let books = load(&dir);
    Fixture {
        _dir: dir,
        books,
        zeta,
        alpha,
        beta,
        riley,
        alpha_cost,
        pitch,
    }
}

fn load(dir: &TempDir) -> Books {
    data::load_once(&Source {
        data_dir: dir.0.join("books-data"),
        today: Some(today()),
    })
    .unwrap()
}

fn harness(f: &Fixture, route: Route) -> Harness<'static, App> {
    let books = load(&f._dir);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 2000.0))
        .build_eframe(move |cc| App::with_books(&cc.egui_ctx, books, false, route.clone()));
    h.run_steps(3);
    h
}

#[test]
fn projects_page_lists_every_project_alphabetically() {
    let f = fixture("list");
    let rows = listed(&f.books);
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["alpha app", "Beta", "Zeta site"]);
    assert_eq!(rows[1].facts, ProjectFacts::default());
    assert_eq!(Section::Projects.home(), Route::Projects);
    assert_eq!(Route::Project(f.beta).section(), Section::Projects);
    let h = harness(&f, Route::Projects);
    h.get_by_label("3 projects, all active. Net to date across them: $2,900.00.");
    for n in names {
        h.get_by_role_and_label(Role::Button, n);
    }
    assert_eq!(rows[1].facts.kind, None, "Beta: kind not recorded");
}

#[test]
fn project_figures_are_ledger_cores() {
    let f = fixture("figures");
    let t = ledger_core::project_to_date(&f.books.ledger, f.zeta, today()).unwrap();
    assert_eq!((t.revenue, t.expenses), (cents(500_000), cents(180_000)));
    let row = listed(&f.books)
        .into_iter()
        .find(|r| r.id == f.zeta)
        .unwrap();
    assert_eq!(row.to_date, Some(t.clone()));
    let h = harness(&f, Route::Project(f.zeta));
    h.get_by_label("$5,000.00");
    h.get_by_label("$1,800.00");
    h.get_by_label("$3,200.00");
    // By account to date: the same statement lines.
    let fig = ledger_core::project_figures(
        &f.books.ledger,
        f.zeta,
        ledger_core::calendar::FIRST_DAY,
        today(),
    )
    .unwrap();
    for l in fig.period.revenue.iter().chain(&fig.period.expenses) {
        h.get_by_role_and_label(Role::Button, &format!("{} {}", l.code, l.name));
    }
    h.get_by_label("Margin to date");
    h.get_by_label("64.0%");
}

#[test]
fn unrecovered_cost_shows_only_on_spec() {
    let f = fixture("unrecovered");
    let h = harness(&f, Route::Project(f.alpha));
    h.get_by_label("Unrecovered cost");
    h.get_by_label("if repaid: not a receivable");
    // The devices (an asset) count nothing: spending and the unrecovered
    // cost are both the prototype's $300.00.
    assert_eq!(h.get_all_by_label("$300.00").count(), 2);
    let h = harness(&f, Route::Project(f.zeta));
    assert!(h.query_by_label("Unrecovered cost").is_none());
}

#[test]
fn a_project_row_opens_the_project() {
    let f = fixture("row");
    let mut h = harness(&f, Route::Projects);
    h.get_by_role_and_label(Role::Button, "alpha app").click();
    h.run_steps(3);
    assert_eq!(h.state_mut().route(), &Route::Project(f.alpha));
}

#[test]
fn an_entry_row_opens_the_entry() {
    let f = fixture("entry-row");
    let mut h = harness(&f, Route::Project(f.alpha));
    h.get_by_role_and_label(Role::Button, "Alpha prototype")
        .click();
    h.run_steps(3);
    assert_eq!(h.state_mut().route(), &Route::Entry(f.alpha_cost));
}

#[test]
fn a_document_opens_the_document() {
    let f = fixture("doc");
    let mut h = harness(&f, Route::Project(f.alpha));
    h.get_by_label("alpha pitch.pdf").click();
    h.run_steps(3);
    assert_eq!(h.state_mut().route(), &Route::Document(f.pitch));
}

#[test]
fn the_party_link_opens_the_party() {
    let f = fixture("party-link");
    let mut h = harness(&f, Route::Project(f.zeta));
    h.get_by_label("Riley Chen").click();
    h.run_steps(3);
    assert_eq!(h.state_mut().route(), &Route::Party(f.riley));
}

#[test]
fn profit_by_project_bar_opens_the_project() {
    let f = fixture("profit");
    let mut h = harness(&f, Route::Profit);
    h.get_by_label("Net income by project, this year");
    h.get_by_role_and_label(Role::Button, "Zeta site").click();
    h.run_steps(3);
    assert_eq!(h.state_mut().route(), &Route::Project(f.zeta));
}

#[test]
fn party_page_lists_its_projects() {
    let f = fixture("party");
    assert_eq!(f.books.ledger.projects_of_party(f.riley), vec![f.zeta]);
    let mut h = harness(&f, Route::Party(f.riley));
    h.get_by_role_and_label(Role::Button, "Zeta site").click();
    h.run_steps(3);
    assert_eq!(h.state_mut().route(), &Route::Project(f.zeta));
}

#[test]
fn entry_line_project_opens_the_project() {
    let f = fixture("entry-line");
    let mut h = harness(&f, Route::Entry(f.alpha_cost));
    h.get_by_label(&format!("{} alpha app", egui_phosphor::regular::TAG))
        .click();
    h.run_steps(3);
    assert_eq!(h.state_mut().route(), &Route::Project(f.alpha));
}

#[test]
fn journal_describes_project_set() {
    let f = fixture("journal");
    let b = &f.books;
    let texts: Vec<(String, Option<Route>)> = b
        .journal
        .iter()
        .filter_map(|l| describe(b, l).map(|e| (e.text, e.route)))
        .collect();
    let has = |t: &str, r: Route| texts.iter().any(|(x, y)| x == t && *y == Some(r.clone()));
    assert!(
        has("Added project Zeta site", Route::Project(f.zeta)),
        "{texts:?}"
    );
    assert!(
        has(
            "Project Zeta site: active, client work",
            Route::Project(f.zeta)
        ),
        "{texts:?}"
    );
    assert!(
        has(
            "Project alpha app: active, on spec",
            Route::Project(f.alpha)
        ),
        "{texts:?}"
    );
    assert!(
        has("Added project Beta", Route::Project(f.beta)),
        "{texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|(t, _)| t == "Linked alpha pitch.pdf to Project alpha app"),
        "{texts:?}"
    );
}

#[test]
fn show_project_target_opens_its_page() {
    let f = fixture("show");
    let mut h = harness(&f, Route::Overview);
    h.state_mut()
        .show(ViewerTarget::Project { class_id: f.alpha });
    h.run_steps(2);
    assert_eq!(h.state_mut().route(), &Route::Project(f.alpha));
    h.state_mut().show(ViewerTarget::Project { class_id: 999 });
    h.run_steps(1);
    assert_eq!(h.state_mut().route(), &Route::Project(f.alpha));
    h.state_mut().show(ViewerTarget::Page {
        page: "projects".into(),
    });
    h.run_steps(2);
    assert_eq!(h.state_mut().route(), &Route::Projects);
    // Every project's page, and one that doesn't exist, render.
    for c in [f.zeta, f.alpha, f.beta, 999] {
        h.state_mut().goto(Route::Project(c));
        h.run_steps(2);
    }
}

/// A project named `name` with `link`, a $100.00 sale and a decision, in
/// books of its own; a harness `w` by `h` on its page.
fn edge(tag: &str, name: &str, link: &str, w: f32, h: f32) -> (TempDir, Harness<'static, App>) {
    let dir = TempDir(std::env::temp_dir().join(format!(
        "ledger-gui-projects-edge-{tag}-{}",
        std::process::id()
    )));
    let _ = std::fs::remove_dir_all(&dir.0);
    let mut s = Store::create(&dir.0.join("books-data"), Currency::USD).unwrap();
    let cash = s
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let revenue = s
        .add_account("4000", "Design Services", AccountType::Revenue, None, false)
        .unwrap();
    let id = s
        .add_project(
            name,
            &ProjectFacts {
                kind: Some(ProjectKind::Client),
                links: vec![link.to_string()],
                ..Default::default()
            },
        )
        .unwrap();
    let e = s.create_entry(d(2026, 3, 2), "Sale").unwrap();
    s.add_line(e, cash, Side::Debit, usd(10_000)).unwrap();
    s.add_line(e, revenue, Side::Credit, usd(10_000)).unwrap();
    s.set_line_class(e, 1, Some(id)).unwrap();
    s.post_entry(e).unwrap();
    s.record_decision(&NewDecision {
        subject: "Take the job".into(),
        decision_md: "Do it.".into(),
        decided_by: vec!["Tobin Harlow".into()],
        decided_on: d(2026, 3, 1),
        evidence_attachment: None,
        supersedes: None,
        citation: None,
        projects: vec![id],
    })
    .unwrap();
    drop(s);
    let books = load(&dir);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(w, h))
        .build_eframe(move |cc| App::with_books(&cc.egui_ctx, books, false, Route::Project(id)));
    harness.run_steps(4);
    // For eyes: SHOT_DIR keeps a picture of each edge page.
    if let Ok(out) = std::env::var("SHOT_DIR") {
        let _ = std::fs::create_dir_all(&out);
        if let Ok(img) = harness.render() {
            img.save(std::path::Path::new(&out).join(format!("{tag}.png")))
                .unwrap();
        }
    }
    (dir, harness)
}

/// A 200-character link with no space to wrap at stays inside the Facts
/// card: left of the Decisions card beside it, and inside the window.
#[test]
fn a_long_link_stays_in_its_card() {
    let link = format!("https://git.example/{}", "a".repeat(180));
    for (w, tag) in [(1440.0, "link-1440"), (900.0, "link-900")] {
        let (_dir, h) = edge(tag, "Harbor", &link, w, 2600.0);
        let shown = h.get_by_label_contains("https://git.example/").rect();
        let decisions = h.get_by_label("Decisions").rect();
        assert!(
            shown.right() <= w,
            "{w}: the link runs to {}",
            shown.right()
        );
        if decisions.left() > shown.left() {
            assert!(
                shown.right() <= decisions.left(),
                "{w}: the link runs to {}, under Decisions at {}",
                shown.right(),
                decisions.left()
            );
        }
        let net = h.get_by_label("Net to date").rect();
        assert!(net.right() <= w, "{w}: Net to date at {net:?}");
    }
}

/// A 200-character name, with spaces or without, keeps the page inside
/// the window: the title and every tile show.
#[test]
fn a_long_name_keeps_the_page_in_the_window() {
    for (name, tag) in [
        ("Harbor ".repeat(28).trim().to_string(), "name-spaces"),
        ("H".repeat(200), "name-nospace"),
    ] {
        let (_dir, h) = edge(tag, &name, "https://harbor.example", 1440.0, 2000.0);
        for tile in ["Income to date", "Net to date", "Margin to date"] {
            let r = h.get_by_label(tile).rect();
            assert!(r.right() <= 1440.0, "{tag}: {tile} at {r:?}");
        }
        let decisions = h.get_by_label("Decisions").rect();
        assert!(
            decisions.right() <= 1440.0,
            "{tag}: Decisions at {decisions:?}"
        );
        let named: Vec<_> = h
            .query_all_by_label_contains("HHHHHHHHHH")
            .chain(h.query_all_by_label_contains("Harbor Harbor"))
            .collect();
        assert!(!named.is_empty(), "{tag}: the name shows");
        for title in named {
            assert!(
                title.rect().right() <= 1440.0,
                "{tag}: {title:?} at {:?}",
                title.rect()
            );
        }
    }
}

/// The by-month bars are the project's own: a click opens no report (the
/// reports are the whole company's), and the margin keeps one place.
#[test]
fn a_month_bar_opens_nothing_and_margin_keeps_one_place() {
    let (_dir, mut h) = edge("month", "Harbor", "https://harbor.example", 1440.0, 2000.0);
    let route = h.state_mut().route().clone();
    // All income, no spending: 100.0%, as 64.0% elsewhere.
    h.get_by_label("100.0%");
    let title = h.get_by_label("By month").rect();
    let at = egui::pos2(title.left() + 200.0, title.bottom() + 120.0);
    h.hover_at(at);
    h.run_steps(2);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(1);
    }
    h.run_steps(2);
    assert_eq!(h.state_mut().route(), &route);
}

/// An entry with no receipt says nothing about receipts.
#[test]
fn an_entry_without_receipts_says_nothing_of_them() {
    fn walk(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let f = fixture("no-receipts");
    let h = harness(&f, Route::Project(f.zeta));
    let mut texts = Vec::new();
    for c in &h.output().shapes {
        walk(&c.shape, &mut texts);
    }
    let subs: Vec<&String> = texts.iter().filter(|t| t.contains("Contractors")).collect();
    assert!(
        !subs.is_empty(),
        "the entries' sub-lines are painted: {texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t.contains("0 receipts")),
        "no \"0 receipts\": {subs:?}"
    );
}

/// The party page has the same long-name edge: a 200-character name keeps
/// to its Facts card and the window.
#[test]
fn a_long_party_name_keeps_to_its_card() {
    let dir = TempDir(std::env::temp_dir().join(format!(
        "ledger-gui-projects-party-long-{}",
        std::process::id()
    )));
    let _ = std::fs::remove_dir_all(&dir.0);
    let mut s = Store::create(&dir.0.join("books-data"), Currency::USD).unwrap();
    let name = "P".repeat(200);
    let party = s.add_party(&name, &[]).unwrap();
    drop(s);
    let books = load(&dir);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 1400.0))
        .build_eframe(move |cc| App::with_books(&cc.egui_ctx, books, false, Route::Party(party)));
    h.run_steps(4);
    let linked = h.get_by_label("Linked records").rect();
    let named: Vec<_> = h.query_all_by_label_contains("PPPPPPPPPP").collect();
    assert!(!named.is_empty());
    for n in named {
        let r = n.rect();
        assert!(r.right() <= 1440.0, "{n:?} at {r:?}");
        // The Facts card's Name row, beside Linked records.
        let key = h.get_by_label("Name").rect();
        if (r.center().y - key.center().y).abs() < 12.0 && r.left() < linked.left() {
            assert!(
                r.right() <= linked.left(),
                "{n:?} at {r:?} under Linked records"
            );
        }
    }
}
