//! "Needs attention", mapped for the viewer: the items come from
//! `ledger_package::attention` (the same function `books.start` reads), and
//! this module only turns each `Target` into the viewer's [`Route`] and each
//! severity into the viewer's own.
//!
//! ```text
//! GOAL: the viewer never computes an attention item itself; it draws the
//!       ones the shared engine found, so the list can't drift from the
//!       server's next steps.
//! SOURCE: plan/served.md U0b (D-53: Needs attention lives in
//!         ledger-package over PackageBooks); ledger_package::attention
//!         (the conditions, the prompts, the INVARIANTS).
//! INVARIANTS:
//!   1. Pure: reads `b`, writes nothing.
//!   2. One item out per item the shared engine found, in the same order,
//!      with the same severity, title, detail, and prompt.
//!   3. `target` is the item's `Target` mapped to this viewer's `Route`.
//! ```

use crate::books::{Attention, Books, Severity};
use crate::nav::{ReportKind, Route};
use ledger_store::{
    Attachment, DiskCheck, JournalLine, Reconciliation, Statement, StoreError, Subject,
};

/// Everything the books need, as items to show.
pub fn find(b: &Books) -> Vec<Attention> {
    let settings = ledger_package::Settings {
        company: &b.settings.company,
        cash_accounts: &b.settings.cash_accounts,
        financing_accounts: &b.settings.financing_accounts,
        investing_accounts: &b.settings.investing_accounts,
    };
    let integrity = ledger_package::Integrity {
        disk: b.integrity.disk.clone(),
        file_failures: b.integrity.file_failures.clone(),
    };
    ledger_package::find_with(&View(b), b.today, &settings, &integrity)
        .into_iter()
        .map(|a| Attention {
            severity: severity(a.severity),
            title: a.title,
            detail: a.detail,
            target: route(a.target),
            prompt: a.prompt,
        })
        .collect()
}

fn severity(s: ledger_package::Severity) -> Severity {
    match s {
        ledger_package::Severity::Problem => Severity::Problem,
        ledger_package::Severity::Todo => Severity::Todo,
        ledger_package::Severity::Info => Severity::Info,
    }
}

fn route(t: ledger_package::Target) -> Option<Route> {
    Some(match t {
        ledger_package::Target::None => return None,
        ledger_package::Target::Entry(e) => Route::Entry(e),
        ledger_package::Target::Transactions => Route::Transactions,
        ledger_package::Target::Company => Route::Company,
        ledger_package::Target::Accounts => Route::Accounts,
        ledger_package::Target::Account(a) => Route::Account(a),
        ledger_package::Target::Party(p) => Route::Party(p),
        ledger_package::Target::Assets => Route::Assets,
        ledger_package::Target::Asset(a) => Route::Asset(a),
        ledger_package::Target::Documents => Route::Documents,
        ledger_package::Target::Invoice(i) => Route::Invoice(i),
        ledger_package::Target::Journal => Route::Journal,
        ledger_package::Target::Report(r) => Route::Reports(match r {
            ledger_package::Report::Vendor1099 => ReportKind::Vendor1099,
            ledger_package::Report::ApAging => ReportKind::ApAging,
            ledger_package::Report::ArAging => ReportKind::ArAging,
            ledger_package::Report::FixedAssetRegister => ReportKind::FixedAssetRegister,
        }),
    })
}

/// The view model seen through [`PackageBooks`]: the snapshot's own state,
/// already read, handed to the shared engine without reading the books
/// again.
struct View<'a>(&'a Books);

impl ledger_package::PackageBooks for View<'_> {
    fn ledger(&self) -> &ledger_core::Ledger {
        &self.0.ledger
    }

    fn journal_seq(&self) -> u64 {
        self.0.journal.len() as u64
    }

    fn journal(&self) -> Result<Vec<JournalLine>, StoreError> {
        Ok(self.0.journal.clone())
    }

    fn all_attachments(&self) -> Result<Vec<Attachment>, StoreError> {
        Ok(self.0.attachments.clone())
    }

    fn links_of(&self, attachment_id: u64) -> Result<Vec<Subject>, StoreError> {
        Ok(self.0.links_of(attachment_id).to_vec())
    }

    fn attachment_bytes(&self, attachment_id: u64) -> Result<Vec<u8>, StoreError> {
        let a = self
            .0
            .attachment(attachment_id)
            .ok_or_else(|| StoreError::Io(std::io::Error::other("no such attachment")))?;
        self.0
            .attachment_bytes(a)
            .map_err(|e| StoreError::Io(std::io::Error::other(e)))
    }

    fn statements(&self) -> Result<Vec<Statement>, StoreError> {
        Ok(self.0.statements.clone())
    }

    fn reconciliations(&self) -> Result<Vec<Reconciliation>, StoreError> {
        Ok(self.0.reconciliations.clone())
    }

    fn disk_check(&self) -> DiskCheck {
        self.0.integrity.disk.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Empty books opened as of `today`.
    fn books(tag: &str, today: chrono::NaiveDate) -> Books {
        let dir =
            std::env::temp_dir().join(format!("ledger-gui-attention-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // Loaded books need no files: removed once loaded, pass or panic.
        let _cleanup = crate::test_support::TempDir(dir.clone());
        let data_dir = dir.join("books-data");
        ledger_store::Store::create(&data_dir, ledger_core::Currency::USD).unwrap();
        crate::data::load_once(&crate::data::Source {
            data_dir,
            today: Some(today),
        })
        .unwrap()
    }

    /// INVARIANT 2's cash flow items (rules.md B-14 rule 12, D-35): a
    /// sale of equipment for an IOU is a Todo naming the entry, with the
    /// statement's fix; equipment bought on an operating card is Info; a
    /// sale for cash asks nothing.
    #[test]
    fn cash_flow_warnings_are_items() {
        use crate::test_support::{Build, d};
        use ledger_core::{AccountType, Currency, Money, NewAsset, Side};
        let mut b = Build::new("attention-cf");
        let mut add =
            |code: &str, name: &str, ty| b.s.add_account(code, name, ty, None, false).unwrap();
        let equipment = add("1500", "Equipment", AccountType::Asset);
        let accum = add("1510", "Accumulated Depreciation", AccountType::Asset);
        let iou = add("1210", "Due from buyer", AccountType::Asset);
        let card = add("2050", "Card", AccountType::Liability);
        let depr = add("6500", "Depreciation", AccountType::Expense);
        let import = ledger_config::codec::import(
            "[books]\ncash_accounts = [\"1000\"]\n",
            None,
            b.s.ledger().settings(),
            "USD",
        )
        .unwrap();
        b.s.adopt_settings(
            &import.changes,
            &ledger_core::settings::Source {
                decided_by: "the test".into(),
                decided_on: d(2026, 1, 1),
                citation: None,
                note: None,
            },
        )
        .unwrap();
        let (cash, capital) = (b.cash, b.capital);
        b.post(d(2026, 2, 1), cash, capital, 500_000);
        b.s.register_asset(NewAsset {
            description: "Laptop".into(),
            in_service_date: d(2026, 2, 2),
            cost: Money::new(rust_decimal::Decimal::new(100_000, 2), Currency::USD),
            salvage_value: Money::new(rust_decimal::Decimal::ZERO, Currency::USD),
            useful_life_months: 36,
            cost_account: equipment,
            accum_depr_account: accum,
            expense_account: depr,
        })
        .unwrap();
        let bought = b.post(d(2026, 2, 2), equipment, card, 100_000);
        let usd = |c: i64| Money::new(rust_decimal::Decimal::new(c, 2), Currency::USD);
        let e =
            b.s.create_entry(d(2026, 3, 2), "Sell the laptop on an IOU")
                .unwrap();
        b.s.add_line(e, iou, Side::Debit, usd(100_000)).unwrap();
        b.s.add_line(e, equipment, Side::Credit, usd(100_000))
            .unwrap();
        b.s.attach_receipt(Some(e), "r.pdf", "application/pdf", b"%PDF r", None)
            .unwrap();
        b.s.post_entry(e).unwrap();
        let (_dir, books) = b.books();
        let items = find(&books);
        let sold = items
            .iter()
            .find(|a| a.title == format!("Equipment sold for an IOU: entry #{e}"))
            .unwrap_or_else(|| panic!("{items:?}"));
        assert_eq!(sold.severity, Severity::Todo);
        assert_eq!(sold.target, Some(Route::Entry(e)));
        assert!(
            sold.detail.contains("books.investing_accounts"),
            "{}",
            sold.detail
        );
        assert!(sold.prompt.starts_with("In wanderbooks,"));
        let on_card = items
            .iter()
            .find(|a| {
                a.title == format!("Equipment bought on an operating payable: entry #{bought}")
            })
            .unwrap_or_else(|| panic!("{items:?}"));
        assert_eq!(on_card.severity, Severity::Info);
        assert_eq!(
            items
                .iter()
                .filter(|a| a.title.starts_with("Equipment"))
                .count(),
            2
        );
    }

    /// INVARIANTS 2 and 4: a dated rule past its review is one Todo item
    /// whose prompt asks the agent to find the figure from the official
    /// authority (named, never the repo's copy), confirm it with the human
    /// and pass it explicitly, and to tell the human so it can be reported
    /// upstream; before the review there is no such item. No prompt points
    /// the agent at files, git, GitHub or pull requests.
    #[test]
    fn a_stale_rule_asks_for_the_authority_and_the_humans_confirmation() {
        let rule = ledger_core::rules::THRESHOLD_1099;
        let review = rule.review_after();
        assert!(
            find(&books("fresh", review.pred_opt().unwrap()))
                .iter()
                .all(|a| !a.title.contains(rule.name)),
            "no item before the review date"
        );
        let items = find(&books("stale", review));
        let stale: Vec<_> = items
            .iter()
            .filter(|a| a.title == format!("Check the {}", rule.name))
            .collect();
        assert_eq!(stale.len(), 1, "{items:?}");
        let a = stale[0];
        assert_eq!(a.severity, Severity::Todo);
        assert!(
            a.detail.contains(&rule.known_through.to_string()),
            "{}",
            a.detail
        );
        for want in [
            "In wanderbooks,",
            rule.name,
            &rule.known_through.to_string(),
            rule.authority,
            "confirm it with me",
            "pass it explicitly",
            "report it upstream",
        ] {
            assert!(a.prompt.contains(want), "{want:?} missing: {}", a.prompt);
        }
        for item in &items {
            let lower = item.prompt.to_lowercase();
            for never in ["pull request", ".md", "docs/", rule.source] {
                assert!(!lower.contains(never), "{never:?} in {}", item.prompt);
            }
            for word in lower.split(|c: char| !c.is_alphanumeric()) {
                assert!(
                    !["git", "github", "repo", "repository", "issue"].contains(&word),
                    "{word:?} in {}",
                    item.prompt
                );
            }
        }
    }
}
