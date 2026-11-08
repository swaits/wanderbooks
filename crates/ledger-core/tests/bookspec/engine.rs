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
        // The picks for the CHOICE records split out of embedded choices
        // (P14 unit S-C7a: entry, ledger, period, statement, prepaid).
        Claims::new(
            bookspec_checker::covered_rules()
                .into_iter()
                .map(|r| r.id.clone()),
        )
        .with_choice(RuleId::new(Area::Entry, 6), "check-when-line-added")
        .with_choice(RuleId::new(Area::Entry, 7), "refuse-rather-than-crash")
        .with_choice(
            RuleId::new(Area::Entry, 8),
            "bounds-bind-commands-never-replay",
        )
        .with_choice(RuleId::new(Area::Ledger, 14), "book-in-the-smallest-unit")
        .with_choice(RuleId::new(Area::Ledger, 15), "child-has-its-parents-type")
        .with_choice(RuleId::new(Area::Ledger, 16), "on-or-after-the-original")
        .with_choice(
            RuleId::new(Area::Ledger, 17),
            "reversal-evidence-is-originals",
        )
        .with_choice(
            RuleId::new(Area::Ledger, 18),
            "assignment-is-a-fact-in-the-books",
        )
        .with_choice(RuleId::new(Area::Ledger, 19), "refuse-rather-than-renumber")
        .with_choice(RuleId::new(Area::Ledger, 20), "warn-by-default")
        .with_choice(RuleId::new(Area::Ledger, 21), "round-once-on-the-total")
        .with_choice(RuleId::new(Area::Period, 4), "nest-never-partially-overlap")
        .with_choice(
            RuleId::new(Area::Period, 5),
            "fiscal-year-to-date-by-default",
        )
        .with_choice(RuleId::new(Area::Statement, 6), "direct")
        .with_choice(RuleId::new(Area::Statement, 7), "constructive")
        .with_choice(RuleId::new(Area::Statement, 8), "year-through-as-of")
        .with_choice(RuleId::new(Area::Statement, 9), "one-project-per-line")
        .with_choice(RuleId::new(Area::Prepaid, 2), "daily")
        .with_choice(RuleId::new(Area::Prepaid, 3), "start-on-the-first")
        .with_choice(
            RuleId::new(Area::Prepaid, 4),
            "minor-unit-for-new-schedules",
        )
    }
}
