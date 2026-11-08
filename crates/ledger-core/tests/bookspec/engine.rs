use crate::*;

/// The `Engine` adapter: the wanderbooks implementation over `Store`, as
/// the bookspec runner drives any engine (`plan/served.md` P13 step 2).
impl Engine for Books<'_> {
    fn load(&mut self, setup: &Setup) -> Result<(), Refusal> {
        if setup != &self.ex.setup {
            return Err(Refusal::new(
                RuleId::new(Area::Ledger, 1),
                "the setup handed over is not the example's",
            ));
        }
        self.setup();
        Ok(())
    }

    fn apply(&mut self, event: &Event) -> Result<(), Refusal> {
        let n = self.posted.len() + 1;
        self.apply_event(n, event)
            .map_err(|e| store_refusal_obj(&e))
    }

    fn change(&mut self, change: &ChangeSpec) -> Result<(), Refusal> {
        self.try_change(change)
    }

    fn query(&self, q: &Query) -> Result<Value, Refusal> {
        self.answer(q)
    }

    fn claims(&self) -> Claims {
        let mut claims = Claims::new(
            bookspec_checker::covered_rules()
                .into_iter()
                .map(|r| r.id.clone()),
        );
        for &(area, n, option) in PICKS {
            claims = claims.with_choice(RuleId::new(area, n), option);
        }
        claims
    }
}

/// The option wanderbooks picks for each CHOICE record split out of an
/// embedded choice (P14 unit S-C7); each is recorded in
/// `docs/spec/claims/<area>.md` as `**Pick (BS-<area>-<n>):**`.
pub(crate) const PICKS: &[(Area, u32, &str)] = &[
    (Area::Entry, 6, "check-when-line-added"),
    (Area::Entry, 7, "refuse-rather-than-crash"),
    (Area::Entry, 8, "bounds-bind-commands-never-replay"),
    (Area::Ledger, 14, "book-in-the-smallest-unit"),
    (Area::Ledger, 15, "child-has-its-parents-type"),
    (Area::Ledger, 16, "on-or-after-the-original"),
    (Area::Ledger, 17, "reversal-evidence-is-originals"),
    (Area::Ledger, 18, "assignment-is-a-fact-in-the-books"),
    (Area::Ledger, 19, "refuse-rather-than-renumber"),
    (Area::Ledger, 20, "warn-by-default"),
    (Area::Ledger, 21, "round-once-on-the-total"),
    (Area::Period, 4, "nest-never-partially-overlap"),
    (Area::Period, 5, "fiscal-year-to-date-by-default"),
    (Area::Statement, 6, "direct"),
    (Area::Statement, 7, "constructive"),
    (Area::Statement, 8, "year-through-as-of"),
    (Area::Statement, 9, "one-project-per-line"),
    (Area::Prepaid, 2, "daily"),
    (Area::Prepaid, 3, "start-on-the-first"),
    (Area::Prepaid, 4, "minor-unit-for-new-schedules"),
    (Area::Partner, 5, "largest-remainder-first-listed"),
    (Area::Partner, 6, "cumulative-from-first-close"),
    (Area::Partner, 7, "running-split"),
    (Area::Partner, 8, "outside-the-books"),
    (Area::Partner, 10, "cash-cards-and-financing"),
    (Area::Partner, 11, "capital-movement-counted-once"),
    (Area::Partner, 12, "books-carrying-amount"),
    (Area::Partner, 13, "labelled-cash-default-or-all"),
    (Area::Partner, 14, "tax-basis-beside-book"),
    (Area::Partner, 15, "running-cumulative"),
    (Area::Partner, 16, "contract-is-property-vendor-paid-is-cash"),
    (Area::Partner, 17, "close-at-the-change"),
    (Area::Partner, 18, "book-amount-with-note"),
    (Area::Asset, 5, "straight-line-by-days"),
    (Area::Asset, 6, "lesser-of-value-and-basis"),
    (Area::Asset, 7, "books-recorded-amounts"),
    (Area::Asset, 8, "register-carries-tax-figures"),
    (Area::Asset, 9, "traditional"),
    (Area::Tax1099, 2, "refuse-an-unknown-year"),
    (Area::Tax1099, 3, "presumed-a-check-ach-or-transfer"),
    (Area::Tax1099, 4, "dated-by-the-payments-date"),
    (Area::Tax1099, 5, "payees-share-only"),
    (Area::Arap, 7, "due-today-is-current"),
    (Area::Arap, 8, "as-of-the-day"),
    (Area::Arap, 9, "refuse-a-payment-before-its-document"),
    (Area::Arap, 10, "ignore-case-and-spaces"),
    (Area::Arap, 11, "reverse-then-plain-entry"),
    (Area::Arap, 12, "gross-when-taken"),
    (Area::Arap, 13, "gross"),
    (Area::Arap, 14, "refuse-with-the-remedy"),
    (Area::Format, 3, "plain-ascii-transliterated-and-listed"),
    (Area::Format, 4, "cr-lf"),
    (Area::Format, 5, "docnum-is-the-entry-id"),
    (Area::Format, 6, "everything-through-as-of"),
    (Area::Format, 7, "journal-sample-columns"),
    (Area::Format, 8, "closes-left-out"),
    (Area::Format, 9, "no-customer-or-vendor-name"),
    (Area::Format, 10, "refuse-the-entry"),
];
