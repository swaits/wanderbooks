use crate::*;

impl Books<'_> {
    pub(crate) fn answer_run(&self, q: &Query) -> Result<Value, Refusal> {
        let n = q.event.ok_or_else(|| self.missing("event"))?;
        let run = self.runs.get(&n).ok_or_else(|| {
            Refusal::new(
                RuleId::new(Area::Asset, 1),
                format!("event {n} is no run_depreciation (or not yet run)"),
            )
        })?;
        let posted = if run.already_posted {
            Decimal::ZERO
        } else {
            run.amount
        };
        let caught_up = (posted - run.scheduled).max(Decimal::ZERO);
        Ok(json_figures([
            ("amount".to_string(), figure(posted)),
            ("caught_up".to_string(), figure(caught_up)),
        ]))
    }

    pub(crate) fn answer_disposal(&self, q: &Query) -> Result<Value, Refusal> {
        let name = q.asset.as_deref().ok_or_else(|| self.missing("asset"))?;
        let p = self.disposals.get(name).ok_or_else(|| {
            Refusal::new(
                RuleId::new(Area::Asset, 1),
                format!("asset {name:?} wasn't disposed of"),
            )
        })?;
        Ok(json_figures([
            (
                "final_depreciation".to_string(),
                figure(p.final_depreciation),
            ),
            (
                "catch_up_depreciation".to_string(),
                figure(p.catch_up_depreciation),
            ),
            (
                "excess_depreciation".to_string(),
                figure(p.excess_depreciation),
            ),
            ("accumulated".to_string(), figure(p.accumulated)),
            ("book_value".to_string(), figure(p.book_value)),
            ("gain_loss".to_string(), figure(p.gain_loss)),
        ]))
    }

    pub(crate) fn answer_depreciation_schedule(&self, q: &Query) -> Result<Value, Refusal> {
        let as_of = self.q_day(q)?;
        let name = q.asset.as_deref().ok_or_else(|| self.missing("asset"))?;
        let asset = self
            .l()
            .get_asset(self.asset(name, name))
            .expect("a registered asset");
        let lines = ledger_core::depreciation_schedule(asset, as_of)
            .map_err(|e| engine_refusal(RuleId::new(Area::Asset, 1), e))?;
        let mut m = JsonMap::new();
        for l in &lines {
            m.insert(l.period.clone(), Value::String(figure(l.amount)));
        }
        // A month the schedule doesn't reach is 0.00: from acquisition
        // (never later than in-service) through the day asked as of.
        let start = self
            .l()
            .asset_acquired_date(asset.id)
            .unwrap_or(asset.in_service_date);
        let (mut year, mut month) = (start.year(), start.month());
        while (year, month) <= (as_of.year(), as_of.month()) {
            m.entry(format!("{year:04}-{month:02}"))
                .or_insert_with(|| Value::String("0.00".to_string()));
            if month == 12 {
                year += 1;
                month = 1;
            } else {
                month += 1;
            }
        }
        let accumulated = ledger_core::accumulated_scheduled_through(asset, as_of)
            .map_err(|e| engine_refusal(RuleId::new(Area::Asset, 1), e))?;
        let book_value = ledger_core::scheduled_book_value(asset, as_of)
            .map_err(|e| engine_refusal(RuleId::new(Area::Asset, 1), e))?;
        m.insert(
            "accumulated".to_string(),
            Value::String(figure(accumulated)),
        );
        m.insert("book_value".to_string(), Value::String(figure(book_value)));
        Ok(Value::Object(m))
    }

    pub(crate) fn answer_schedule_plan(&self, q: &Query) -> Result<Value, Refusal> {
        let name = q
            .schedule
            .as_deref()
            .ok_or_else(|| self.missing("schedule"))?;
        let id = *self.schedules.get(name).ok_or_else(|| {
            Refusal::new(
                RuleId::new(Area::Prepaid, 1),
                format!("no schedule {name:?}"),
            )
        })?;
        let plan = self
            .l()
            .get_schedule(id)
            .expect("a registered schedule")
            .plan()
            .map_err(|e| engine_refusal(RuleId::new(Area::Prepaid, 1), e))?;
        let mut m = JsonMap::new();
        let mut total = Decimal::ZERO;
        for (d, amount) in &plan {
            m.insert(d.to_string(), Value::String(figure(*amount)));
            total += *amount;
        }
        m.insert("total".to_string(), Value::String(figure(total)));
        Ok(Value::Object(m))
    }

    pub(crate) fn answer_register_row(&self, q: &Query) -> Result<Value, Refusal> {
        let as_of = self.q_day(q)?;
        let name = q.asset.as_deref().ok_or_else(|| self.missing("asset"))?;
        let id = self.asset(name, name);
        let r = ledger_core::fixed_asset_register(self.l(), as_of)
            .map_err(|e| engine_refusal(RuleId::new(Area::Asset, 3), e))?;
        let row = r
            .rows
            .iter()
            .find(|row| row.asset_id == id)
            .ok_or_else(|| {
                Refusal::new(
                    RuleId::new(Area::Asset, 3),
                    "not on the register (acquired later)",
                )
            })?;
        Ok(json_figures([
            ("acquired".to_string(), row.acquired_date.to_string()),
            ("in_service".to_string(), row.in_service_date.to_string()),
            ("cost".to_string(), figure(row.cost)),
            (
                "accumulated_scheduled".to_string(),
                figure(row.accumulated_scheduled),
            ),
            (
                "accumulated_posted".to_string(),
                figure(row.accumulated_posted),
            ),
            (
                "unposted".to_string(),
                figure(row.accumulated_scheduled - row.accumulated_posted),
            ),
            ("book_value".to_string(), figure(row.book_value)),
        ]))
    }

    pub(crate) fn answer_register_totals(&self, q: &Query) -> Result<Value, Refusal> {
        let as_of = self.q_day(q)?;
        let r = ledger_core::fixed_asset_register(self.l(), as_of)
            .map_err(|e| engine_refusal(RuleId::new(Area::Asset, 3), e))?;
        Ok(json_figures([
            ("total_cost".to_string(), figure(r.total_cost)),
            (
                "total_accumulated_scheduled".to_string(),
                figure(r.total_accumulated_scheduled),
            ),
            (
                "total_accumulated_posted".to_string(),
                figure(r.total_accumulated_posted),
            ),
            ("total_book_value".to_string(), figure(r.total_book_value)),
            (
                "total_depreciable_basis".to_string(),
                figure(r.total_depreciable_basis),
            ),
            (
                "total_tax_basis_723".to_string(),
                figure(r.total_tax_basis_723),
            ),
            ("missing_basis".to_string(), r.missing_basis.to_string()),
            (
                "unregistered_cost".to_string(),
                figure(r.unregistered_cost()),
            ),
            (
                "unposted_depreciation".to_string(),
                figure(r.unposted_depreciation()),
            ),
            (
                "unattributed_accumulated".to_string(),
                figure(r.unattributed_accumulated()),
            ),
        ]))
    }

    pub(crate) fn answer_asset_basis(&self, q: &Query) -> Result<Value, Refusal> {
        let name = q.asset.as_deref().ok_or_else(|| self.missing("asset"))?;
        let a = self
            .l()
            .get_asset(self.asset(name, name))
            .expect("a registered asset");
        let f = a
            .basis_figures()
            .map_err(|e| engine_refusal(RuleId::new(Area::Asset, 4), e))?
            .ok_or_else(|| Refusal::new(RuleId::new(Area::Asset, 4), "no basis recorded"))?;
        let opt = |d: Option<Decimal>| d.map_or("none".to_string(), figure);
        let rule = a.basis.as_ref().expect("a basis recorded").rule;
        Ok(json_figures([
            ("rule".to_string(), rule.key().to_string()),
            (
                "contributor_adjusted_basis".to_string(),
                opt(a.basis.as_ref().and_then(|b| b.contributor_adjusted_basis)),
            ),
            (
                "fmv_at_contribution".to_string(),
                opt(a.fmv_at_contribution()),
            ),
            ("tax_basis_723".to_string(), opt(f.tax_basis_723)),
            ("depreciable_basis".to_string(), figure(f.depreciable_basis)),
            ("built_in_gain_704c".to_string(), opt(f.built_in_gain_704c)),
            ("sec704c_1c".to_string(), a.sec704c_1c().to_string()),
        ]))
    }
}

/// Check one `run` expectation: what the `run_depreciation` event `event`
/// posted (`amount`: zero when nothing was due, or the month already had
/// its run) and, of it, what it caught up beyond its own month's
/// depreciation (`caught_up`), both listed (rules.md B-15).
pub(crate) fn check_run(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    undated(ex, x);
    let n = x
        .event
        .unwrap_or_else(|| panic!("{}: run names its event", ex.id));
    let run = b.runs.get(&n).unwrap_or_else(|| {
        panic!(
            "{}: event {n} is no run_depreciation (or not yet run)",
            ex.id
        )
    });
    let posted = if run.already_posted {
        Decimal::ZERO
    } else {
        run.amount
    };
    let caught_up = (posted - run.scheduled).max(Decimal::ZERO);
    exactly(
        ex,
        x,
        &["amount".to_string(), "caught_up".to_string()].into(),
    );
    for (key, got) in [("amount", posted), ("caught_up", caught_up)] {
        let want = exact(&x.figures(&ex.id)[key], &format!("{} {key}", ex.id));
        assert_eq!(got, want, "{} run of event {n}: {key}", ex.id);
    }
}

/// Check one `disposal` expectation: what the disposal of `asset` took, as
/// the engine computed it when posting it (rules.md B-15), every figure
/// listed: `final_depreciation` (posted in its own entry before the
/// disposal), `catch_up_depreciation` and `excess_depreciation` (charged,
/// and credited back, in the disposal entry), `accumulated` (cleared),
/// `book_value` and `gain_loss` (a gain positive).
pub(crate) fn check_disposal(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    undated(ex, x);
    let asset = x
        .asset
        .as_deref()
        .unwrap_or_else(|| panic!("{}: disposal names its asset", ex.id));
    let p = b
        .disposals
        .get(asset)
        .unwrap_or_else(|| panic!("{}: asset {asset:?} wasn't disposed of", ex.id));
    let figures = [
        ("final_depreciation", p.final_depreciation),
        ("catch_up_depreciation", p.catch_up_depreciation),
        ("excess_depreciation", p.excess_depreciation),
        ("accumulated", p.accumulated),
        ("book_value", p.book_value),
        ("gain_loss", p.gain_loss),
    ];
    exactly(ex, x, &figures.iter().map(|(k, _)| k.to_string()).collect());
    for (key, got) in figures {
        let want = exact(&x.figures(&ex.id)[key], &format!("{} {key}", ex.id));
        assert_eq!(got, want, "{} disposal of {asset}: {key}", ex.id);
    }
}

/// Check one `depreciation_schedule` expectation: the schedule of `asset`
/// through `as_of` (rules.md B-15), each month named (`"YYYY-MM"`) its
/// depreciation, 0.00 for a month the schedule doesn't reach;
/// `accumulated` and `book_value` through `as_of`; the example names the
/// figures it shows.
pub(crate) fn check_depreciation_schedule(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    let as_of = day(ex, x);
    let name = x
        .asset
        .as_deref()
        .unwrap_or_else(|| panic!("{}: depreciation_schedule names its asset", ex.id));
    let what = format!("{} depreciation_schedule of {name} through {as_of}", ex.id);
    let asset = b
        .l()
        .get_asset(b.asset(name, &what))
        .expect("a registered asset");
    let lines =
        ledger_core::depreciation_schedule(asset, as_of).unwrap_or_else(|e| panic!("{what}: {e}"));
    for (key, want) in x.figures(&ex.id) {
        let want = exact(want, &format!("{what}: {key}"));
        let got = match key.as_str() {
            "accumulated" => ledger_core::accumulated_scheduled_through(asset, as_of),
            "book_value" => ledger_core::scheduled_book_value(asset, as_of),
            month => {
                month_key(month, &what);
                Ok(lines
                    .iter()
                    .find(|l| l.period == month)
                    .map_or(Decimal::ZERO, |l| l.amount))
            }
        }
        .unwrap_or_else(|e| panic!("{what}: {key}: {e}"));
        assert_eq!(got, want, "{what}: {key}");
    }
}

/// Check one `schedule_plan` expectation: what the schedule's plan
/// recognizes at each month end named (`"YYYY-MM-DD"`, 0.00 at a month end
/// the plan doesn't post), and `total`, all it recognizes (rules.md B-16);
/// the example names the figures it shows.
pub(crate) fn check_schedule_plan(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    undated(ex, x);
    let name = x
        .schedule
        .as_deref()
        .unwrap_or_else(|| panic!("{}: schedule_plan names its schedule", ex.id));
    let what = format!("{} schedule_plan of {name}", ex.id);
    let id = *b
        .schedules
        .get(name)
        .unwrap_or_else(|| panic!("{what}: no schedule {name:?}"));
    let plan = b
        .l()
        .get_schedule(id)
        .expect("a registered schedule")
        .plan()
        .unwrap_or_else(|e| panic!("{what}: {e}"));
    for (key, want) in x.figures(&ex.id) {
        let want = exact(want, &format!("{what}: {key}"));
        let got: Decimal = if key == "total" {
            plan.iter().map(|(_, a)| *a).sum()
        } else {
            let d = date(key, &what);
            assert_eq!(
                d,
                ledger_core::schedules::month_end(d),
                "{what}: {key} is no month end"
            );
            plan.iter().filter(|(m, _)| *m == d).map(|(_, a)| *a).sum()
        };
        assert_eq!(got, want, "{what}: {key}");
    }
}

/// Check one `register_row` expectation: the asset's row of the fixed
/// asset register as of `as_of` (rules.md B-15 rules 7 and 8, B-24):
/// `acquired` and `in_service` (`YYYY-MM-DD`), `cost`,
/// `accumulated_scheduled` (the schedule's), `accumulated_posted` (what
/// the books carry), `unposted` (the one less the other), `book_value`
/// (cost less posted); each 0.00 once disposed; the example names the
/// figures it shows. An asset not on the register by `as_of` (acquired
/// later) is refused.
pub(crate) fn check_register_row(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    let as_of = day(ex, x);
    let name = x
        .asset
        .as_deref()
        .unwrap_or_else(|| panic!("{}: register_row names its asset", ex.id));
    let what = format!("{} register_row of {name} as of {as_of}", ex.id);
    let id = b.asset(name, &what);
    let r =
        ledger_core::fixed_asset_register(b.l(), as_of).unwrap_or_else(|e| panic!("{what}: {e}"));
    let row = r
        .rows
        .iter()
        .find(|row| row.asset_id == id)
        .unwrap_or_else(|| panic!("{what}: not on the register (acquired later)"));
    let figures = BTreeMap::from([
        ("acquired", row.acquired_date.to_string()),
        ("in_service", row.in_service_date.to_string()),
        ("cost", figure(row.cost)),
        ("accumulated_scheduled", figure(row.accumulated_scheduled)),
        ("accumulated_posted", figure(row.accumulated_posted)),
        (
            "unposted",
            figure(row.accumulated_scheduled - row.accumulated_posted),
        ),
        ("book_value", figure(row.book_value)),
    ]);
    named(ex, x, &what, &figures);
}

/// Check one `register_totals` expectation: the fixed asset register's
/// totals as of `as_of`, over the assets held (rules.md B-24, B-25):
/// `total_cost`, `total_accumulated_scheduled`, `total_accumulated_posted`,
/// `total_book_value`, `total_depreciable_basis`, `total_tax_basis_723`,
/// `missing_basis` (how many held have none), and how it reconciles to
/// the books: `unregistered_cost`, `unposted_depreciation`,
/// `unattributed_accumulated`; the example names the figures it shows.
pub(crate) fn check_register_totals(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    let as_of = day(ex, x);
    let what = format!("{} register_totals as of {as_of}", ex.id);
    let r =
        ledger_core::fixed_asset_register(b.l(), as_of).unwrap_or_else(|e| panic!("{what}: {e}"));
    let figures = BTreeMap::from([
        ("total_cost", figure(r.total_cost)),
        (
            "total_accumulated_scheduled",
            figure(r.total_accumulated_scheduled),
        ),
        (
            "total_accumulated_posted",
            figure(r.total_accumulated_posted),
        ),
        ("total_book_value", figure(r.total_book_value)),
        ("total_depreciable_basis", figure(r.total_depreciable_basis)),
        ("total_tax_basis_723", figure(r.total_tax_basis_723)),
        ("missing_basis", r.missing_basis.to_string()),
        ("unregistered_cost", figure(r.unregistered_cost())),
        ("unposted_depreciation", figure(r.unposted_depreciation())),
        (
            "unattributed_accumulated",
            figure(r.unattributed_accumulated()),
        ),
    ]);
    named(ex, x, &what, &figures);
}

/// Check one `asset_basis` expectation: the asset's basis facts and the
/// tax figures they give (rules.md B-25): `rule`,
/// `contributor_adjusted_basis`, `fmv_at_contribution`, `tax_basis_723`,
/// `depreciable_basis`, `built_in_gain_704c` (a loss negative), each
/// `"none"` where the asset has none (a purchase's §723 basis), and
/// `sec704c_1c` (`"true"`: a built-in loss, which counts only for the
/// contributing member, IRC §704(c)(1)(C)); the example names the figures
/// it shows.
pub(crate) fn check_asset_basis(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    undated(ex, x);
    let name = x
        .asset
        .as_deref()
        .unwrap_or_else(|| panic!("{}: asset_basis names its asset", ex.id));
    let what = format!("{} asset_basis of {name}", ex.id);
    let a = b
        .l()
        .get_asset(b.asset(name, &what))
        .expect("a registered asset");
    let f = a
        .basis_figures()
        .unwrap_or_else(|e| panic!("{what}: {e}"))
        .unwrap_or_else(|| panic!("{what}: no basis recorded"));
    let opt = |d: Option<Decimal>| d.map_or("none".to_string(), figure);
    let rule = a.basis.as_ref().expect("a basis recorded").rule;
    let figures = BTreeMap::from([
        ("rule", rule.key().to_string()),
        (
            "contributor_adjusted_basis",
            opt(a.basis.as_ref().and_then(|b| b.contributor_adjusted_basis)),
        ),
        ("fmv_at_contribution", opt(a.fmv_at_contribution())),
        ("tax_basis_723", opt(f.tax_basis_723)),
        ("depreciable_basis", figure(f.depreciable_basis)),
        ("built_in_gain_704c", opt(f.built_in_gain_704c)),
        ("sec704c_1c", a.sec704c_1c().to_string()),
    ]);
    named(ex, x, &what, &figures);
}
