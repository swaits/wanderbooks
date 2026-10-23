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
use ledger_gui::pages::projects::{CONTRIBUTION_NOTE, Snap, Standing, chart_span, listed};
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
    let devices = add("1500", "Equipment", AccountType::Asset);
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
    // Direct costs, and the costs card's total.
    assert_eq!(h.get_all_by_label("$1,800.00").count(), 2);
    h.get_by_label("$3,200.00");
    // By account to date: the same statement lines.
    let fig = ledger_core::project_figures(
        &f.books.ledger,
        f.zeta,
        ledger_core::calendar::FIRST_DAY,
        today(),
    )
    .unwrap();
    assert_eq!(fig.to_date.revenue_by_account, fig.period.revenue);
    assert_eq!(fig.to_date.expenses_by_account, fig.period.expenses);
    for l in &fig.period.revenue {
        h.get_by_role_and_label(Role::Button, &format!("{} {}", l.code, l.name));
    }
    // An expense account is a row and a bar on Costs by category.
    for l in &fig.period.expenses {
        let label = format!("{} {}", l.code, l.name);
        assert_eq!(
            h.get_all_by_role_and_label(Role::Button, &label).count(),
            2,
            "{label}"
        );
    }
    h.get_by_label("Contribution margin");
    h.get_by_label("64.0%");
}

#[test]
fn unrecovered_cost_shows_only_on_spec() {
    let f = fixture("unrecovered");
    let h = harness(&f, Route::Project(f.alpha));
    h.get_by_label("Unrecovered cost");
    h.get_by_label("if repaid: not a receivable");
    // The devices (an asset) count nothing: direct costs, the costs
    // card's total and the unrecovered cost are all the prototype's
    // $300.00.
    assert_eq!(h.get_all_by_label("$300.00").count(), 3);
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
        let net = h.get_by_label("Contribution").rect();
        assert!(net.right() <= w, "{w}: Contribution at {net:?}");
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
        for tile in ["Revenue", "Contribution", "Contribution margin"] {
            for r in h.get_all_by_label(tile).map(|n| n.rect()) {
                assert!(r.right() <= 1440.0, "{tag}: {tile} at {r:?}");
            }
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

// ------------------------------------------------- the snapshot (B-37 item 7)

/// Books holding one project of `kind`, started `started_on`, with
/// `posts`: (date, debit code, credit code, cents), the line that isn't
/// cash tagged to it. Codes: 1000 Cash, 4000 Design Services, 6000
/// Contractors, 6100 Software. Rent (6200) is no project's.
struct Mini {
    dir: TempDir,
    id: ClassId,
    tag: String,
}

fn mini(
    tag: &str,
    kind: ProjectKind,
    started_on: Option<NaiveDate>,
    posts: &[(NaiveDate, &str, &str, i64)],
) -> Mini {
    let dir = TempDir(std::env::temp_dir().join(format!(
        "ledger-gui-projects-snap-{tag}-{}",
        std::process::id()
    )));
    let _ = std::fs::remove_dir_all(&dir.0);
    let mut s = Store::create(&dir.0.join("books-data"), Currency::USD).unwrap();
    for (code, name, ty) in [
        ("1000", "Cash", AccountType::Asset),
        ("4000", "Design Services", AccountType::Revenue),
        ("6000", "Contractors", AccountType::Expense),
        ("6100", "Software", AccountType::Expense),
        ("6200", "Rent", AccountType::Expense),
    ] {
        s.add_account(code, name, ty, None, false).unwrap();
    }
    let id = s
        .add_project(
            "Kestrel",
            &ProjectFacts {
                kind: Some(kind),
                started_on,
                ..Default::default()
            },
        )
        .unwrap();
    let code = |s: &Store, c: &str| s.ledger().chart().find_by_code(c).unwrap().id;
    for &(date, dr, cr, c) in posts {
        let e = s.create_entry(date, "Line").unwrap();
        let (dr_id, cr_id) = (code(&s, dr), code(&s, cr));
        s.add_line(e, dr_id, Side::Debit, usd(c)).unwrap();
        s.add_line(e, cr_id, Side::Credit, usd(c)).unwrap();
        s.set_line_class(e, if dr == "1000" { 1 } else { 0 }, Some(id))
            .unwrap();
        s.post_entry(e).unwrap();
    }
    // Overhead, no project's: never in the snapshot.
    let e = s.create_entry(d(2026, 4, 30), "Rent").unwrap();
    let (rent, cash) = (code(&s, "6200"), code(&s, "1000"));
    s.add_line(e, rent, Side::Debit, usd(150_000)).unwrap();
    s.add_line(e, cash, Side::Credit, usd(150_000)).unwrap();
    s.post_entry(e).unwrap();
    drop(s);
    Mini {
        dir,
        id,
        tag: format!("snap-{tag}"),
    }
}

fn mini_harness(m: &Mini) -> Harness<'static, App> {
    let books = load(&m.dir);
    let id = m.id;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 2000.0))
        .build_eframe(move |cc| App::with_books(&cc.egui_ctx, books, false, Route::Project(id)));
    h.run_steps(3);
    shot(&mut h, &m.tag);
    h
}

/// For eyes: SHOT_DIR keeps a picture of the page.
fn shot(h: &mut Harness<'static, App>, tag: &str) {
    if let Ok(out) = std::env::var("SHOT_DIR") {
        let _ = std::fs::create_dir_all(&out);
        if let Ok(img) = h.render() {
            img.save(std::path::Path::new(&out).join(format!("{tag}.png")))
                .unwrap();
        }
    }
}

/// Every text painted this frame.
fn painted(h: &Harness<'static, App>) -> Vec<String> {
    painted_colored(h).into_iter().map(|(t, _)| t).collect()
}

/// Every text painted this frame, with its color.
fn painted_colored(h: &Harness<'static, App>) -> Vec<(String, egui::Color32)> {
    fn walk(s: &egui::Shape, out: &mut Vec<(String, egui::Color32)>) {
        match s {
            egui::Shape::Text(t) => {
                let color = t
                    .galley
                    .job
                    .sections
                    .first()
                    .map_or(t.fallback_color, |s| s.format.color);
                let color = t.override_text_color.unwrap_or(color);
                out.push((t.galley.text().to_string(), color));
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for c in &h.output().shapes {
        walk(&c.shape, &mut out);
    }
    out
}

/// The value a snapshot tile labelled `label` shows: the label directly
/// under that tile's own (the same left edge, just below), so a figure
/// elsewhere on the page can't stand in for it. Exactly one such tile.
fn tile(h: &Harness<'static, App>, label: &str) -> String {
    let nodes: Vec<(egui::Rect, String)> = h
        .query_all(egui_kittest::kittest::By::new().role(Role::Label))
        .filter_map(|n| Some((n.rect(), n.value()?)))
        .collect();
    let found: Vec<String> = nodes
        .iter()
        .filter(|(_, v)| v == label)
        .filter_map(|(r, _)| {
            nodes
                .iter()
                .filter(|(x, _)| {
                    (x.left() - r.left()).abs() < 1.0
                        && x.top() >= r.bottom() - 1.0
                        && x.top() <= r.bottom() + 12.0
                })
                .min_by(|a, b| a.0.top().total_cmp(&b.0.top()))
                .map(|(_, v)| v.clone())
        })
        .filter(|v| v.contains('$') || v.ends_with('%'))
        .collect();
    assert_eq!(found.len(), 1, "one tile {label}: {found:?}");
    found[0].clone()
}

/// The tiles' figures are ledger-core's, the same books.project returns
/// (to_date's revenue, expenses, net and margin_pct; its tests check the
/// server's side, and ledger-cli's projects_parity both on one book).
fn assert_tiles(h: &Harness<'static, App>, s: &Snap) {
    use ledger_gui::fmt::dollars;
    assert_eq!(tile(h, "Revenue"), dollars(s.revenue), "revenue");
    assert_eq!(tile(h, "Direct costs"), dollars(s.expenses), "direct costs");
    assert_eq!(
        tile(h, "Contribution"),
        dollars(s.contribution),
        "contribution"
    );
    match s.margin_pct {
        Some(m) => assert_eq!(tile(h, "Contribution margin"), format!("{m}%")),
        None => assert!(h.query_by_label("Contribution margin").is_none()),
    }
}

/// [`assert_tiles`], and the pill says where it stands (to date, whichever
/// view: `standing`).
fn assert_snapshot(h: &Harness<'static, App>, s: &Snap, standing: Standing) {
    assert_tiles(h, s);
    h.get_by_label(standing.words());
}

/// The tooltip shown after hovering `at` a while: every label containing
/// `text` once it shows.
fn hover_shows(h: &mut Harness<'static, App>, at: egui::Pos2, text: &str) -> bool {
    h.hover_at(at);
    for _ in 0..60 {
        h.run_steps(1);
        if h.query_all_by_label_contains(text).next().is_some()
            || painted(h).iter().any(|t| t.contains(text))
        {
            return true;
        }
    }
    false
}

#[test]
fn a_profitable_projects_snapshot_is_ledger_cores() {
    let f = fixture("snap-profit");
    let t = ledger_core::project_to_date(&f.books.ledger, f.zeta, today()).unwrap();
    let s = Snap::to_date(&t);
    assert_eq!(
        s,
        Snap {
            revenue: cents(500_000),
            expenses: cents(180_000),
            contribution: cents(320_000),
            margin_pct: Some(Decimal::new(640, 1)),
            standing: Standing::Black,
        }
    );
    assert_eq!(s.contribution, t.net());
    let mut h = harness(&f, Route::Project(f.zeta));
    shot(&mut h, "snap-profit");
    assert_snapshot(&h, &s, s.standing);
    h.get_by_label("before overhead");
    assert!(h.query_all_by_label("Snapshot").next().is_some());
    // The Contribution tile says what it leaves out, on hover.
    let at = h
        .query_all_by_label("Contribution")
        .map(|n| n.rect())
        .min_by(|a, b| a.top().total_cmp(&b.top()))
        .unwrap()
        .center();
    assert!(
        hover_shows(&mut h, at, CONTRIBUTION_NOTE),
        "the Contribution tile's hover note"
    );
    // Costs by category: its one expense account, a bar that opens it.
    h.get_by_label("Costs by category, to date");
    assert_eq!(
        h.get_all_by_role_and_label(Role::Button, "6000 Contractors")
            .count(),
        2,
        "a bar and By account's row"
    );
}

#[test]
fn a_loss_making_project_is_in_the_red() {
    let m = mini(
        "loss",
        ProjectKind::Client,
        None,
        &[
            (d(2026, 3, 2), "1000", "4000", 20_000),
            (d(2026, 3, 9), "6000", "1000", 10_000),
            (d(2026, 4, 9), "6100", "1000", 50_000),
        ],
    );
    let b = load(&m.dir);
    let t = ledger_core::project_to_date(&b.ledger, m.id, today()).unwrap();
    let s = Snap::to_date(&t);
    assert_eq!(s.contribution, cents(-40_000));
    assert_eq!(s.standing, Standing::Red);
    assert_eq!(s.margin_pct, Some(Decimal::new(-2000, 1)));
    let h = mini_harness(&m);
    assert_snapshot(&h, &s, Standing::Red);
    // Rent, overhead, is no part of it.
    assert!(h.query_by_label("$1,500.00").is_none());
    // Both cost accounts, largest first: the costs card's bars (the
    // first buttons so named; By account has the others, by code), so
    // Software's $500 is above Contractors' $100 though its code is after.
    let first = |label: &str| {
        h.get_all_by_role_and_label(Role::Button, label)
            .next()
            .unwrap()
            .rect()
    };
    assert!(first("6100 Software").top() < first("6000 Contractors").top());
}

#[test]
fn an_on_spec_projects_snapshot_has_its_unrecovered_cost() {
    let f = fixture("snap-spec");
    let t = ledger_core::project_to_date(&f.books.ledger, f.alpha, today()).unwrap();
    let s = Snap::to_date(&t);
    assert_eq!(s.standing, Standing::Red);
    assert_eq!(t.unrecovered_cost, Some(cents(30_000)));
    let mut h = harness(&f, Route::Project(f.alpha));
    shot(&mut h, "snap-spec");
    assert_snapshot(&h, &s, Standing::Red);
    h.get_by_label("Unrecovered cost");
    h.get_by_label("if repaid: not a receivable");
}

#[test]
fn no_revenue_means_no_margin() {
    let m = mini(
        "no-revenue",
        ProjectKind::Internal,
        None,
        &[(d(2026, 5, 1), "6100", "1000", 9_900)],
    );
    let h = mini_harness(&m);
    assert!(h.query_by_label("Contribution margin").is_none());
    h.get_by_label("In the red");
    h.get_by_label("−$99.00");
}

#[test]
fn a_project_with_no_entries_has_nothing_tagged_yet() {
    let m = mini("empty", ProjectKind::Client, None, &[]);
    let b = load(&m.dir);
    let t = ledger_core::project_to_date(&b.ledger, m.id, today()).unwrap();
    assert_eq!(Snap::to_date(&t).standing, Standing::Nothing);
    assert_eq!(chart_span(today(), None, None), None);
    let h = mini_harness(&m);
    h.get_by_label("Nothing tagged yet");
    h.get_by_label("nothing tagged yet");
    h.get_by_label("No tagged lines yet");
    h.get_by_label("No costs tagged");
    assert!(h.query_by_label("Contribution margin").is_none());
}

#[test]
fn a_reimbursement_above_its_cost_leaves_costs_below_zero() {
    let m = mini(
        "reimbursed",
        ProjectKind::Client,
        None,
        &[
            (d(2026, 2, 9), "6000", "1000", 20_000),
            // The client repays the contractor and more, credited to it.
            (d(2026, 3, 1), "1000", "6000", 30_000),
        ],
    );
    let b = load(&m.dir);
    let t = ledger_core::project_to_date(&b.ledger, m.id, today()).unwrap();
    assert_eq!(t.expenses, cents(-10_000));
    assert_eq!(t.expenses_by_account[0].amount, cents(-10_000));
    let s = Snap::to_date(&t);
    assert_eq!(
        (s.contribution, s.standing),
        (cents(10_000), Standing::Black)
    );
    assert_eq!(s.margin_pct, None, "no revenue");
    let h = mini_harness(&m);
    // Direct costs, and the costs card's total.
    assert_eq!(h.get_all_by_label("−$100.00").count(), 2);
    h.get_by_label("Below zero: an account credited more than it cost (a reimbursement).");
    assert_eq!(
        h.get_all_by_role_and_label(Role::Button, "6000 Contractors")
            .count(),
        2
    );
}

#[test]
fn a_project_started_this_month_charts_one_month() {
    let m = mini(
        "this-month",
        ProjectKind::Client,
        Some(d(2026, 10, 1)),
        &[(d(2026, 10, 5), "1000", "4000", 50_000)],
    );
    let b = load(&m.dir);
    assert_eq!(
        chart_span(today(), Some(d(2026, 10, 1)), Some(d(2026, 10, 5))),
        Some((d(2026, 10, 1), today()))
    );
    let f = ledger_core::project_figures(&b.ledger, m.id, d(2026, 10, 1), today()).unwrap();
    assert_eq!(f.months.len(), 1);
    assert_eq!(f.months_net_to_date, vec![cents(50_000)]);
    let h = mini_harness(&m);
    h.get_by_label("since Oct 2026");
    h.get_by_label("In the black");
    h.get_by_label("100.0%");
}

#[test]
fn the_chart_runs_from_the_start_or_first_entry_and_caps() {
    // The earlier of the two, from its month's first day.
    assert_eq!(
        chart_span(today(), Some(d(2026, 6, 20)), Some(d(2026, 3, 9))),
        Some((d(2026, 3, 1), today()))
    );
    assert_eq!(
        chart_span(today(), Some(d(2026, 2, 2)), Some(d(2026, 3, 9))),
        Some((d(2026, 2, 1), today()))
    );
    assert_eq!(
        chart_span(today(), Some(d(2026, 2, 2)), None),
        Some((d(2026, 2, 1), today()))
    );
    // A start after today charts only this month.
    assert_eq!(
        chart_span(today(), Some(d(2027, 1, 1)), None),
        Some((d(2026, 10, 1), today()))
    );
    // Ten years back: the last 24 months.
    assert_eq!(
        chart_span(today(), Some(d(2016, 1, 1)), None),
        Some((d(2024, 11, 1), today()))
    );
}

/// The fiscal year switch shows books.project's default period (B-30's
/// omitted dates: the fiscal year to date), and switching writes nothing.
#[test]
fn the_fiscal_year_switch_shows_the_default_period() {
    let m = mini(
        "fiscal",
        ProjectKind::Client,
        None,
        &[
            (d(2025, 11, 3), "1000", "4000", 100_000),
            (d(2025, 12, 1), "6100", "1000", 25_000),
            (d(2026, 2, 9), "6000", "1000", 40_000),
        ],
    );
    let b = load(&m.dir);
    let (start, end) = b.periods().statement(None, None).unwrap();
    assert!(
        start > d(2025, 12, 1),
        "the sale and the software are before this fiscal year"
    );
    let fy = Snap::period(&ledger_core::project_figures(&b.ledger, m.id, start, end).unwrap());
    let all = Snap::to_date(&ledger_core::project_to_date(&b.ledger, m.id, today()).unwrap());
    assert_eq!(
        (all.expenses, all.contribution, all.standing),
        (cents(65_000), cents(35_000), Standing::Black)
    );
    assert_eq!(
        (fy.revenue, fy.expenses, fy.contribution, fy.standing),
        (Decimal::ZERO, cents(40_000), cents(-40_000), Standing::Red)
    );
    let journal = std::fs::read(m.dir.0.join("books-data").join("journal.jsonl")).unwrap();
    let mut h = mini_harness(&m);
    assert_snapshot(&h, &all, Standing::Black);
    h.get_by_label("This fiscal year").click();
    h.run_steps(3);
    assert!(h.state_mut().state_mut().projects.fiscal);
    shot(&mut h, "snap-fiscal");
    // The tiles are the fiscal year's; where it stands is still to date,
    // as the to-date cards beside it and the list say.
    assert_snapshot(&h, &fy, Standing::Black);
    assert!(h.query_by_label("In the red").is_none());
    assert_eq!(
        h.query_all_by_label(&format!("since {}", ledger_gui::fmt::date(start)))
            .count(),
        2,
        "Revenue's and Direct costs' span"
    );
    h.get_by_label("To date").click();
    h.run_steps(3);
    assert!(!h.state_mut().state_mut().projects.fiscal);
    assert_snapshot(&h, &all, Standing::Black);
    assert_eq!(
        std::fs::read(m.dir.0.join("books-data").join("journal.jsonl")).unwrap(),
        journal,
        "the viewer wrote nothing"
    );
}

/// The list colors each net by its sign and says where it stands.
#[test]
fn the_list_cues_in_the_black_and_in_the_red() {
    let f = fixture("list-cue");
    let rows = listed(&f.books);
    let standing = |id| {
        rows.iter()
            .find(|r| r.id == id)
            .and_then(|r| r.to_date.as_ref())
            .map(|t| Snap::to_date(t).standing)
    };
    assert_eq!(standing(f.zeta), Some(Standing::Black));
    assert_eq!(standing(f.alpha), Some(Standing::Red));
    assert_eq!(standing(f.beta), Some(Standing::Nothing));
    let mut h = harness(&f, Route::Projects);
    shot(&mut h, "snap-list");
    let texts = painted_colored(&h);
    let color = |t: &str| {
        texts
            .iter()
            .find(|(x, _)| x.ends_with(t))
            .map(|(_, c)| *c)
            .unwrap_or_else(|| panic!("{t} painted: {texts:?}"))
    };
    // Each net in its standing's color, as its cue: never the names'.
    let (black, red, name) = (color("3,200.00"), color("(300.00)"), color("Zeta site"));
    assert_eq!(black, color("in the black"));
    assert_eq!(red, color("in the red"));
    assert!(
        black != name && red != name && black != red,
        "{black:?} {red:?} {name:?}"
    );
    // Alphabetical still.
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["alpha app", "Beta", "Zeta site"]);
}

/// A cost tagged and then repaid exactly (or reversed) leaves no account
/// with a balance, but it was tagged: break-even, never "nothing tagged".
#[test]
fn a_cost_repaid_exactly_breaks_even() {
    let m = mini(
        "even",
        ProjectKind::Client,
        None,
        &[
            (d(2026, 3, 9), "6000", "1000", 40_000),
            (d(2026, 4, 1), "1000", "6000", 40_000),
        ],
    );
    let b = load(&m.dir);
    let t = ledger_core::project_to_date(&b.ledger, m.id, today()).unwrap();
    assert!(t.expenses_by_account.is_empty(), "the account nets to zero");
    assert_eq!((t.revenue_lines, t.expenses_lines), (0, 2));
    let s = Snap::to_date(&t);
    assert_eq!(
        (s.contribution, s.standing),
        (Decimal::ZERO, Standing::Even)
    );
    let mut h = mini_harness(&m);
    assert_snapshot(&h, &s, Standing::Even);
    h.get_by_label("before overhead");
    h.get_by_label("Tagged costs net to zero");
    h.get_by_label("Tagged lines net to zero");
    assert!(h.query_by_label("Nothing tagged yet").is_none());
    assert!(h.query_by_label("No costs tagged").is_none());
    // The list: 0.00 and its cue, not a dash.
    h.state_mut().goto(Route::Projects);
    h.run_steps(3);
    let texts = painted(&h);
    assert!(texts.iter().any(|t| t.ends_with("break-even")), "{texts:?}");
    assert!(texts.iter().any(|t| t == "0.00"), "{texts:?}");
}

/// Lines only in earlier years: this fiscal year has nothing, said so,
/// while the pill keeps the standing to date.
#[test]
fn a_fiscal_year_with_nothing_says_so() {
    let m = mini(
        "fiscal-empty",
        ProjectKind::Client,
        None,
        &[(d(2025, 6, 3), "1000", "4000", 70_000)],
    );
    let mut h = mini_harness(&m);
    h.get_by_label("This fiscal year").click();
    h.run_steps(3);
    shot(&mut h, "snap-fiscal-empty");
    h.get_by_label("nothing this fiscal year");
    h.get_by_label("In the black");
    assert!(h.query_by_label("Nothing tagged yet").is_none());
}

/// The by-month chart's running net is ledger-core's months_net_to_date:
/// hovering each month shows its "Net to date", in the chart's own strip.
#[test]
fn the_running_net_is_each_months_net_to_date() {
    let m = mini(
        "running",
        ProjectKind::Client,
        None,
        &[
            (d(2026, 3, 2), "1000", "4000", 120_000),
            (d(2026, 3, 9), "6000", "1000", 10_000),
            (d(2026, 5, 9), "6100", "1000", 30_000),
            (d(2026, 8, 1), "1000", "4000", 45_000),
        ],
    );
    let b = load(&m.dir);
    let (s, e) = chart_span(today(), None, Some(d(2026, 3, 2))).unwrap();
    let f = ledger_core::project_figures(&b.ledger, m.id, s, e).unwrap();
    assert_eq!(f.months.len(), 8);
    // In month order, a month repeating the one before it (nothing new
    // that month) told once: what sweeping the pointer across shows.
    let mut want: Vec<String> = f
        .months_net_to_date
        .iter()
        .map(|v| format!("Net to date: {}", ledger_gui::fmt::dollars(*v)))
        .collect();
    want.dedup();
    assert_eq!(want.len(), 3, "{want:?}");
    let mut h = mini_harness(&m);
    let title = h.get_by_label("By month").rect();
    let right = h.get_by_label("Costs by category, to date").rect().left();
    // Over the bars, and over the strip under them.
    for dy in [100.0, 230.0] {
        let mut seen = Vec::new();
        let mut x = title.left();
        while x < right {
            h.hover_at(egui::pos2(x, title.bottom() + dy));
            h.run_steps(2);
            for t in painted(&h) {
                seen.extend(
                    t.lines()
                        .filter(|l| {
                            l.starts_with("Net to date: $") || l.starts_with("Net to date: −$")
                        })
                        .map(str::to_string),
                );
            }
            x += 12.0;
        }
        seen.dedup();
        assert_eq!(seen, want, "at {dy}");
    }
    shot(&mut h, "snap-running");
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(64))]

    /// The snapshot's contribution is project_to_date's net, whatever the
    /// lines (refunds, reimbursements), and its standing is the net's sign.
    #[test]
    fn prop_snapshot_contribution_is_the_net_to_date(
        lines in proptest::collection::vec((0usize..3, 1i64..500_000, proptest::bool::ANY, proptest::bool::ANY, proptest::bool::ANY), 0..12),
    ) {
        let mut l = ledger_core::Ledger::new(Currency::USD);
        let cash = l.add_account("1000", "Cash", AccountType::Asset, None, false).unwrap();
        let accts = [
            l.add_account("4000", "Sales", AccountType::Revenue, None, false).unwrap(),
            l.add_account("6000", "Costs", AccountType::Expense, None, false).unwrap(),
            l.add_account("6100", "Tools", AccountType::Expense, None, false).unwrap(),
        ];
        let k = l.add_class("K").unwrap();
        let mut tagged = 0usize;
        for (a, c, debit, tag, reverse) in lines {
            let e = l.create_entry(d(2026, 3, 1), "e");
            let (dr, cr) = if debit { (accts[a], cash) } else { (cash, accts[a]) };
            l.add_line(e, dr, Side::Debit, usd(c)).unwrap();
            l.add_line(e, cr, Side::Credit, usd(c)).unwrap();
            if tag {
                l.set_line_class(e, if debit { 0 } else { 1 }, Some(k)).unwrap();
                tagged += 1;
            }
            l.post_entry(e).unwrap();
            // A reversal: its tagged line too, netting the account to
            // zero (it was tagged all the same).
            if reverse {
                l.reverse_entry(e, d(2026, 3, 2), "undo").unwrap();
                if tag {
                    tagged += 1;
                }
            }
        }
        let t = ledger_core::project_to_date(&l, k, today()).unwrap();
        proptest::prop_assert_eq!(t.revenue_lines + t.expenses_lines, tagged);
        let s = Snap::to_date(&t);
        proptest::prop_assert_eq!(s.contribution, t.net());
        proptest::prop_assert_eq!(s.revenue - s.expenses, s.contribution);
        proptest::prop_assert_eq!(s.margin_pct, t.margin_pct());
        let want = if t.net() > Decimal::ZERO {
            Standing::Black
        } else if t.net() < Decimal::ZERO {
            Standing::Red
        } else if tagged == 0 {
            Standing::Nothing
        } else {
            Standing::Even
        };
        proptest::prop_assert_eq!(s.standing, want);
    }
}

/// rules.md B-37 item 8: an invoice's or bill's page (its entry's) lists
/// the document's lines, each project a link that opens it; an untagged
/// line has none. The entry's own lines show the tagged revenue or cost
/// line too.
#[test]
fn an_invoice_and_a_bill_show_each_lines_project() {
    use ledger_core::arap::DocumentLine;
    let dir = TempDir(std::env::temp_dir().join(format!(
        "ledger-gui-projects-documents-{}",
        std::process::id()
    )));
    let _ = std::fs::remove_dir_all(&dir.0);
    let mut s = Store::create(&dir.0.join("books-data"), Currency::USD).unwrap();
    s.add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let zeta = s
        .add_project(
            "Zeta site",
            &ProjectFacts {
                kind: Some(ProjectKind::Client),
                ..Default::default()
            },
        )
        .unwrap();
    let harbor = s.add_class("Harbor app").unwrap();
    let c = s.add_customer("Cedar Client").unwrap();
    let v = s.add_vendor("Dana Illustration").unwrap();
    let line = |what: &str, c: i64, class| DocumentLine {
        description: what.into(),
        amount: cents(c),
        class_id: class,
    };
    let inv = s
        .create_invoice(
            c,
            d(2026, 3, 2),
            d(2026, 4, 1),
            vec![
                line("Design", 300_000, Some(zeta)),
                line("Rush fee", 10_000, None),
            ],
        )
        .unwrap();
    let bill = s
        .create_bill(
            v,
            d(2026, 3, 9),
            d(2026, 4, 8),
            vec![
                line("Illustration", 120_000, Some(harbor)),
                line("Postage", 5_000, None),
            ],
        )
        .unwrap();
    let inv_entry = s.ledger().get_invoice(inv).unwrap().entry_id;
    let bill_entry = s.ledger().get_bill(bill).unwrap().entry_id;
    drop(s);
    let tag = |name: &str| format!("{} {name}", egui_phosphor::regular::TAG);
    for (entry, title, kept, untagged, project, id, shot_name) in [
        (
            inv_entry,
            "INVOICE 1, DUE APR 1, 2026",
            "Design",
            "Rush fee",
            "Zeta site",
            zeta,
            "invoice-lines",
        ),
        (
            bill_entry,
            "BILL 1, DUE APR 8, 2026",
            "Illustration",
            "Postage",
            "Harbor app",
            harbor,
            "bill-lines",
        ),
    ] {
        let books = load(&dir);
        let mut h = Harness::builder()
            .with_size(egui::vec2(1440.0, 1400.0))
            .build_eframe(move |cc| {
                App::with_books(&cc.egui_ctx, books, false, Route::Entry(entry))
            });
        h.run_steps(3);
        shot(&mut h, shot_name);
        h.get_by_label(title);
        h.get_by_label(kept);
        h.get_by_label(untagged);
        // The entry's tagged line and the document's line: two links, one
        // project; the untagged lines, none.
        assert_eq!(h.query_all_by_label(&tag(project)).count(), 2, "{title}");
        // The two links and the history's "tagged project" event: nothing
        // else wears a tag, so the untagged lines have none.
        assert_eq!(
            h.query_all_by_label_contains(egui_phosphor::regular::TAG)
                .count(),
            3,
            "{title}"
        );
        assert_eq!(
            h.query_all_by_label_contains(&format!("tagged project {project}"))
                .count(),
            1,
            "{title}"
        );
        // The document's line, the lower of the two, opens the project.
        h.query_all_by_label(&tag(project))
            .max_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
            .unwrap()
            .click();
        h.run_steps(3);
        assert_eq!(h.state_mut().route(), &Route::Project(id), "{title}");
    }
}

/// The demo's "Client work" project carries every client invoice (rules.md
/// B-37 item 8): its revenue to date is the invoices' total, its direct
/// costs the contractors tagged to it.
#[test]
fn the_demos_client_work_counts_every_client_invoice() {
    let dir = TempDir(
        std::env::temp_dir().join(format!("ledger-gui-projects-demo-{}", std::process::id())),
    );
    let _ = std::fs::remove_dir_all(&dir.0);
    let today = d(2026, 10, 12);
    ledger_gui::demo::build(&dir.0, today).unwrap();
    let b = data::load_once(&Source {
        data_dir: dir.0.join("books-data"),
        today: Some(today),
    })
    .unwrap();
    let l = &b.ledger;
    let client = l.find_class_by_name("Client work").unwrap().id;
    let invoices = l.invoices();
    assert!(!invoices.is_empty());
    assert!(
        invoices
            .iter()
            .all(|i| i.lines.iter().all(|x| x.class_id == Some(client))),
        "every client invoice is tagged"
    );
    let invoiced: Decimal = invoices.iter().map(|i| i.total().unwrap()).sum();
    let t = ledger_core::project_to_date(l, client, today).unwrap();
    assert_eq!(t.revenue, invoiced);
    let tagged_costs: Decimal = l
        .posted_entries()
        .flat_map(|e| e.lines().iter())
        .filter(|x| x.class_id == Some(client))
        .filter(|x| {
            l.chart()
                .get(x.account)
                .is_some_and(|a| a.account_type == AccountType::Expense)
        })
        .map(|x| match x.side {
            Side::Debit => x.amount.amount,
            Side::Credit => -x.amount.amount,
        })
        .sum();
    assert_eq!(t.expenses, tagged_costs);
    assert!(t.expenses > Decimal::ZERO && t.net() > Decimal::ZERO);
}
