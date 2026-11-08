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
        for &(area, n, option) in PICKS_S_C7B {
            claims = claims.with_choice(RuleId::new(area, n), option);
        }
        claims
    }
}

/// The option wanderbooks picks for each CHOICE record split out of the
/// partner, asset, 1099 and format rules (P14 unit S-C7b); each is recorded
/// in `docs/spec/claims/<area>.md` as `**Pick (BS-<area>-<n>):**`.
pub(crate) const PICKS_S_C7B: &[(Area, u32, &str)] = &[
    (Area::Partner, 5, "largest-remainder-first-listed"),
    (Area::Partner, 6, "cumulative-from-first-close"),
    (Area::Partner, 7, "running-split"),
    (Area::Partner, 8, "outside-the-books"),
    (Area::Partner, 9, "tin-facts-then-w9"),
    (Area::Partner, 10, "cash-cards-and-financing"),
    (Area::Partner, 11, "capital-movement-counted-once"),
    (Area::Partner, 12, "books-carrying-amount"),
    (Area::Partner, 13, "labelled-cash-default-or-all"),
    (Area::Partner, 14, "tax-basis-beside-book"),
    (Area::Partner, 15, "running-cumulative"),
    (Area::Asset, 5, "straight-line-by-days"),
    (Area::Asset, 6, "lesser-of-value-and-basis"),
    (Area::Asset, 7, "books-recorded-amounts"),
    (Area::Asset, 8, "register-carries-tax-figures"),
    (Area::Tax1099, 2, "refuse-an-unknown-year"),
    (Area::Tax1099, 3, "presumed-a-check-ach-or-transfer"),
    (Area::Tax1099, 4, "dated-by-the-payments-date"),
    (Area::Format, 3, "plain-ascii-transliterated-and-listed"),
    (Area::Format, 4, "cr-lf"),
    (Area::Format, 5, "docnum-is-the-entry-id"),
    (Area::Format, 6, "everything-through-as-of"),
    (Area::Format, 7, "journal-sample-columns"),
];
