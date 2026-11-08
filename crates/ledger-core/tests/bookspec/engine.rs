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
        Claims::new(
            bookspec_checker::covered_rules()
                .into_iter()
                .map(|r| r.id.clone()),
        )
    }
}
