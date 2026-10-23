//! Live updates: a change written to the books (by the books server, in
//! practice) reaches the GUI's view model on its own, without a restart,
//! and the changed entry is marked fresh; a show request reaches it, and
//! while it runs it holds the books' viewer presence.

use chrono::NaiveDate;
use ledger_core::{AccountType, Currency, Money, Side};
use ledger_gui::data::{Loader, Source, Update};
use ledger_store::Store;
use rust_decimal::Decimal;
use std::time::{Duration, Instant};

/// A test's dir, removed when the test ends, pass or panic.
struct TestDir(std::path::PathBuf);

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_change_in_another_process_shows_up_live() {
    let dir = std::env::temp_dir().join(format!("ledger-gui-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _cleanup = TestDir(dir.clone());
    let data = dir.join("books-data");
    let mut store = Store::create(&data, Currency::USD).unwrap();
    let cash = store
        .add_account("1000", "Checking", AccountType::Asset, None, false)
        .unwrap();
    let equity = store
        .add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    drop(store);
    // The server makes the books' ops/ folder when it opens them; no
    // viewer has them open yet.
    ledger_store::ops::prepare(&data).unwrap();
    assert!(!ledger_store::ops::viewer_open(&data));

    let ctx = egui::Context::default();
    let loader = Loader::spawn(
        Source {
            data_dir: data.clone(),
            today: NaiveDate::from_ymd_opt(2026, 10, 12),
        },
        ctx,
    );
    let first = match loader.wait(Duration::from_secs(10)) {
        Some(Update::Books(b)) => b,
        _ => panic!("no initial load"),
    };
    assert_eq!(first.kpis.cash, Decimal::ZERO);
    // The running viewer holds its presence there (ops INVARIANT 3).
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ledger_store::ops::viewer_open(&data) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        ledger_store::ops::viewer_open(&data),
        "the running viewer holds no presence"
    );

    // The writer (the MCP server's store) posts an entry.
    let started = Instant::now();
    let mut store = Store::open(&data).unwrap();
    let d = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
    let e = store.create_entry(d, "Owner contribution").unwrap();
    let m = Money::new(Decimal::from(2500), Currency::USD);
    store.add_line(e, cash, Side::Debit, m).unwrap();
    store.add_line(e, equity, Side::Credit, m).unwrap();
    store.post_entry(e).unwrap();
    drop(store);

    // The GUI sees it, without being asked.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut seen = None;
    while Instant::now() < deadline {
        if let Some(Update::Books(b)) = loader.wait(Duration::from_millis(200))
            && b.kpis.cash == Decimal::from(2500)
        {
            seen = Some(b);
            break;
        }
    }
    let b = seen.expect("the posted entry never reached the GUI");
    assert!(b.fresh.contains(&e), "the new entry is highlighted as new");
    assert!(b.seq() > first.seq());
    eprintln!("live update arrived in {:?}", started.elapsed());

    // The agent asks the viewer to show the entry (books.show).
    let asked = Instant::now();
    ledger_store::ops::request(
        &data,
        ledger_store::ops::ViewerTarget::Entry { entry_id: e },
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut shown = false;
    while Instant::now() < deadline {
        if let Some(Update::Show(ledger_store::ops::ViewerTarget::Entry { entry_id })) =
            loader.wait(Duration::from_millis(200))
        {
            assert_eq!(entry_id, e);
            shown = true;
            break;
        }
    }
    assert!(shown, "the show request never reached the viewer");
    eprintln!("show request arrived in {:?}", asked.elapsed());

    // The viewer closed: its presence goes with it, within a poll.
    drop(loader);
    let deadline = Instant::now() + Duration::from_secs(10);
    while ledger_store::ops::viewer_open(&data) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !ledger_store::ops::viewer_open(&data),
        "a closed viewer still holds its presence"
    );
}

/// A setting the server journals while the viewer runs is followed (the
/// books' own settings, the only ones the viewer reads: D-44); a settings
/// file beside the books is never read.
#[test]
fn a_setting_changed_while_it_runs_is_followed() {
    let dir = std::env::temp_dir().join(format!("ledger-gui-live-set-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _cleanup = TestDir(dir.clone());
    let data = dir.join("books-data");
    drop(Store::create(&data, Currency::USD).unwrap());
    std::fs::write(
        dir.join("wanderbooks.toml"),
        "[company]\nname = \"Beside Co\"\n",
    )
    .unwrap();
    let loader = Loader::spawn(
        Source {
            data_dir: data.clone(),
            today: NaiveDate::from_ymd_opt(2026, 10, 12),
        },
        egui::Context::default(),
    );
    match loader.wait(Duration::from_secs(10)) {
        Some(Update::Books(b)) => assert_ne!(b.settings.company.name, "Beside Co"),
        _ => panic!("no initial load"),
    }
    let mut store = Store::open(&data).unwrap();
    store
        .change_settings(
            &[ledger_core::settings::Change {
                key: "company.name".into(),
                value: Some(serde_json::json!("Made Later Co")),
            }],
            &ledger_core::settings::Source {
                decided_by: "Tobin Harlow".into(),
                decided_on: NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(),
                citation: None,
                note: None,
            },
        )
        .unwrap();
    drop(store);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(Instant::now() < deadline, "the new setting was never read");
        match loader.wait(Duration::from_millis(200)) {
            Some(Update::Books(b)) if b.settings.company.name == "Made Later Co" => break,
            Some(Update::Error(e)) => panic!("the books didn't load: {e}"),
            _ => {}
        }
    }
}
