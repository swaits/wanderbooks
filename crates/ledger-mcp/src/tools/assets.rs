//! Fixed assets through the server: `books.register_asset`,
//! `books.asset_schedule`, `books.list_assets`, `books.run_depreciation`,
//! `books.dispose_asset`, and the basis, label and correction tools.
//! Split whole out of `server.rs` (plan/served.md 8.2 U0a).

use super::*;

/// A parameter and its deprecated alias (one release): the value given,
/// noting when it came by the alias; both given and meaning different
/// values (by `same`: an amount, a day or a rule however written) is
/// refused.
fn aliased<'a>(
    name: &str,
    value: Option<&'a str>,
    alias: &str,
    old: Option<&'a str>,
    notes: &mut Vec<String>,
    same: fn(&str, &str) -> bool,
) -> Result<Option<&'a str>, String> {
    match (value, old) {
        (Some(v), Some(o)) if !same(v, o) => Err(format!(
            "{name} {v:?} and its old name {alias} {o:?} disagree: give {name} only"
        )),
        (Some(v), _) => Ok(Some(v)),
        (None, Some(o)) => {
            notes.push(format!("{alias} is now {name}"));
            Ok(Some(o))
        }
        (None, None) => Ok(None),
    }
}

/// Two amounts given for one figure mean the same (`"100"` and
/// `"100.00"`); text that isn't an amount compares as text.
fn same_amount(a: &str, b: &str) -> bool {
    match (a.trim().parse::<Decimal>(), b.trim().parse::<Decimal>()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Two days given for one date mean the same.
fn same_day(a: &str, b: &str) -> bool {
    match (parse_date(a), parse_date(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Two basis rules mean the same, by wire or pre-P3 name
/// (`"carryover"` and `"carryover_723"`).
fn same_rule(a: &str, b: &str) -> bool {
    let rule = |r: &str| {
        ledger_core::BasisRule::parse(r).or_else(|| ledger_core::BasisRule::parse_legacy(r))
    };
    match (rule(a), rule(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

/// A basis rule from a tool: its wire name, or its pre-P3 name (an alias
/// for one release, noted).
fn parse_basis_rule(rule: &str, notes: &mut Vec<String>) -> Result<ledger_core::BasisRule, String> {
    if let Some(r) = ledger_core::BasisRule::parse(rule) {
        return Ok(r);
    }
    match ledger_core::BasisRule::parse_legacy(rule) {
        Some(r) => {
            notes.push(format!("the rule {rule:?} is now {:?}", r.key()));
            Ok(r)
        }
        None => Err(format!(
            "basis rule {rule:?}: expected purchase, carryover_723, or personal_use_conversion"
        )),
    }
}

/// A basis record from a tool's facts, checked (AssetBasis::check). A
/// contribution's value at contribution is required: it is a fact the
/// human gives, never assumed from the cost.
fn parse_asset_basis(
    rule: &str,
    adjusted: Option<&str>,
    fmv: Option<&str>,
    method: Option<&str>,
    source: Option<&BasisSourceIn>,
    notes: &mut Vec<String>,
) -> Result<ledger_core::AssetBasis, String> {
    let rule = parse_basis_rule(rule, notes)?;
    if rule.is_contribution() && fmv.is_none() {
        return Err(format!(
            "a contribution ({}) needs fmv_at_contribution: its fair market value when contributed (ask the human; the books' amount in the contribution entry, if that is what it was worth)",
            rule.key()
        ));
    }
    let basis = ledger_core::AssetBasis {
        rule,
        contributor_adjusted_basis: adjusted.map(parse_amount).transpose()?,
        fmv_at_contribution: fmv.map(parse_amount).transpose()?,
        sec704c_method: method
            .map(|m| {
                ledger_core::Sec704cMethod::parse(m).ok_or_else(|| {
                    format!(
                        "sec704c_method {m:?}: expected traditional, traditional_with_curative, or remedial"
                    )
                })
            })
            .transpose()?,
        source: source
            .map(|s| -> Result<_, String> {
                Ok(ledger_core::settings::Source {
                    decided_by: s.decided_by.clone(),
                    decided_on: match &s.decided_on {
                        Some(d) => parse_date(d)?,
                        None => today(),
                    },
                    citation: s.citation.clone(),
                    note: s.note.clone(),
                })
            })
            .transpose()?,
    };
    basis
        .check()
        .map_err(|why| format!("bad tax basis: {why}"))?;
    Ok(basis)
}

/// An asset's basis facts and figures for a tool's output.
fn basis_out(a: &ledger_core::Asset) -> Result<Option<AssetBasisOut>, String> {
    let Some(b) = &a.basis else {
        return Ok(None);
    };
    let Some(f) = a.basis_figures().map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    use ledger_core::BasisRule;
    let loss = f
        .built_in_gain_704c
        .filter(|g| g.is_sign_negative() && !g.is_zero());
    Ok(Some(AssetBasisOut {
        rule: b.rule.key().to_string(),
        contributor_adjusted_basis: b.contributor_adjusted_basis.map(money),
        fmv_at_contribution: a.fmv_at_contribution().map(money),
        fmv_recorded: a.fmv_is_recorded(),
        tax_basis_723: f.tax_basis_723.map(money),
        depreciable_basis: money(f.depreciable_basis),
        built_in_gain_704c: f.built_in_gain_704c.map(money),
        sec704c_1c_note: loss.map(|l| {
            format!(
                "a built-in loss of {}: it counts only for the contributing member(s); for the others the basis is treated as the value at contribution (IRC §704(c)(1)(C))",
                money(-l)
            )
        }),
        sec704c_method: b.sec704c_method.map(|m| m.key().to_string()),
        source: b.source.as_ref().map(|s| BasisSourceOut {
            decided_by: s.decided_by.clone(),
            decided_on: s.decided_on.to_string(),
            citation: s.citation.clone(),
            note: s.note.clone(),
        }),
        sources: BasisSourcesOut {
            tax_basis_723: b.rule.is_contribution().then(|| "IRC §723".to_string()),
            depreciable_basis: match b.rule {
                BasisRule::Purchase => "IRC §1012",
                BasisRule::Carryover723 => "IRC §723",
                BasisRule::PersonalUseConversion => "Treas. Reg. §1.168(i)-4(b)",
            }
            .to_string(),
            built_in_gain_704c: b
                .rule
                .is_contribution()
                .then(|| "Treas. Reg. §1.704-3(a)(3)(ii)".to_string()),
        },
    }))
}

/// An asset named by id or by tag (exactly one).
fn resolve_asset(store: &Store, id: Option<u64>, tag: Option<&str>) -> Result<u64, String> {
    match (id, tag) {
        (Some(id), None) => store
            .ledger()
            .get_asset(id)
            .map(|a| a.id)
            .ok_or_else(|| format!("unknown asset {id}")),
        (None, Some(tag)) => store
            .ledger()
            .asset_by_tag(tag)
            .map(|a| a.id)
            .ok_or_else(|| format!("no asset is tagged {tag:?}")),
        _ => Err("name the asset by asset_id or by tag (one of them)".into()),
    }
}

#[tool_router(router = assets_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.label_asset",
        title = "Label a fixed asset",
        description = "Change a fixed asset's description, link it to the entry that acquired it (acquisition_entry), or set or change its tag (the label on the thing: letters, digits, - _ . /, at most 32; unique ignoring case and never reused by another asset, even after a retag or disposal) and its manufacturer serial number. The asset is its id or current tag. Journaled. Print labels with books.asset_labels.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn label_asset(
        &self,
        params: Parameters<LabelAssetParams>,
    ) -> Result<Json<LabelAssetOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let id = resolve_asset_ref(&store, &p.asset)?;
        let clear = p.clear_serial.unwrap_or(false);
        if clear && p.serial.is_some() {
            return Err("pass serial or clear_serial, not both".into());
        }
        if p.tag.is_none()
            && p.serial.is_none()
            && !clear
            && p.acquisition_entry.is_none()
            && p.description.is_none()
        {
            return Err(
                "nothing to change: pass tag, serial, clear_serial, description, or acquisition_entry"
                    .into(),
            );
        }
        if let Some(d) = &p.description {
            store
                .ledger()
                .check_asset_description(id, d)
                .map_err(|e| e.to_string())?;
        }
        if let Some(entry) = p.acquisition_entry {
            store
                .ledger()
                .check_asset_acquisition(id, entry)
                .map_err(|e| e.to_string())?;
        }
        // Both checked first, so a bad serial doesn't leave a new tag behind.
        if let Some(tag) = &p.tag {
            store
                .ledger()
                .check_asset_tag_for(Some(id), tag)
                .map_err(|e| e.to_string())?;
        }
        let serial = if clear { None } else { p.serial.as_deref() };
        if p.serial.is_some() || clear {
            store
                .ledger()
                .check_asset_serial(id, serial)
                .map_err(|e| e.to_string())?;
        }
        if let Some(tag) = &p.tag {
            store.set_asset_tag(id, tag).map_err(|e| e.to_string())?;
        }
        if p.serial.is_some() || clear {
            store
                .set_asset_serial(id, serial)
                .map_err(|e| e.to_string())?;
        }
        if let Some(entry) = p.acquisition_entry {
            store
                .set_asset_acquisition(id, entry)
                .map_err(|e| e.to_string())?;
        }
        if let Some(d) = &p.description {
            store
                .set_asset_description(id, d)
                .map_err(|e| e.to_string())?;
        }
        let a = store.ledger().get_asset(id).expect("resolved");
        Ok(Json(LabelAssetOut {
            asset_id: id,
            tag: a.tag.clone(),
            serial: a.serial.clone(),
            acquisition_entry: a.acquired_by,
            description: a.description.clone(),
        }))
    }

    #[tool(
        name = "books.correct_asset",
        title = "Correct a fixed asset",
        description = "Correct a registered fixed asset's cost, salvage value, useful life, or in-service date (never before its acquisition entry's date) when it was registered wrong, with the reason; omitted fields keep their values. Refused while the books carry depreciation for it (a run posted and not reversed: correct with entries then). Its tag, serial, links and basis record stay (a recorded value at contribution never moves; a pre-P3 basis reads it as the book cost, so its built-in gain moves, noted); correct the entry that put it on the books too (books.export fixed_asset_register shows any difference). Journaled with the old figures.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn correct_asset(
        &self,
        params: Parameters<CorrectAssetParams>,
    ) -> Result<Json<AssetSummaryOne>, String> {
        let p = params.0;
        let mut notes = Vec::new();
        let in_service = aliased(
            "in_service_date",
            p.in_service_date.as_deref(),
            "in_service",
            p.in_service.as_deref(),
            &mut notes,
            same_day,
        )?;
        let mut store = self.lock()?;
        let id = resolve_asset_ref(&store, &p.asset)?;
        let a = store.ledger().get_asset(id).expect("resolved").clone();
        let base = store.ledger().base_currency();
        let fix = ledger_core::AssetCorrection {
            cost: match &p.cost {
                Some(c) => Money::new(parse_amount(c)?, base),
                None => a.cost,
            },
            salvage_value: match &p.salvage_value {
                Some(c) => Money::new(parse_amount(c)?, base),
                None => a.salvage_value,
            },
            useful_life_months: p.useful_life_months.unwrap_or(a.useful_life_months),
            in_service_date: match in_service {
                Some(d) => parse_date(d)?,
                None => a.in_service_date,
            },
        };
        let before = a.built_in_gain_704c();
        store
            .correct_asset(id, fix, &p.reason)
            .map_err(|e| e.to_string())?;
        let ledger = store.ledger();
        let a = ledger.get_asset(id).expect("resolved");
        if a.built_in_gain_704c() != before {
            notes.push(format!(
                "its basis was recorded before P3 with no value at contribution, which reads as the book cost, so its §704(c) built-in gain moved with the cost ({} to {}): record the value at contribution with books.set_asset_tax_basis",
                before.map_or("none".into(), money),
                a.built_in_gain_704c().map_or("none".into(), money)
            ));
        }
        Ok(Json(AssetSummaryOne {
            asset_id: id,
            tag: a.tag.clone(),
            cost: money(a.cost.amount),
            salvage_value: money(a.salvage_value.amount),
            useful_life_months: a.useful_life_months,
            in_service_date: a.in_service_date.to_string(),
            acquired_date: ledger
                .asset_acquired_date(id)
                .expect("known asset")
                .to_string(),
            basis: basis_out(a)?,
            notes,
        }))
    }

    #[tool(
        name = "books.set_asset_tax_basis",
        title = "Set an asset's tax basis",
        description = "Record how the company got a fixed asset and its tax figures beside book value. rule: purchase (basis is cost, IRC §1012); carryover_723 (a member's business property: their adjusted basis carries over, IRC §723); personal_use_conversion (a member's personal property: §723 basis likewise, depreciable basis the lesser of value and that basis, Treas. Reg. §1.168(i)-4(b)). A contribution needs contributor_adjusted_basis and fmv_at_contribution (ask the human; never assume the cost); built_in_gain_704c = value − §723 basis (Treas. Reg. §1.704-3(a)(3)(ii)), a loss counting only for the contributor. sec704c_method once chosen; basis_source: who says so. Journaled; book figures don't change.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn set_asset_tax_basis(
        &self,
        params: Parameters<SetAssetTaxBasisParams>,
    ) -> Result<Json<SetAssetTaxBasisOut>, String> {
        let p = params.0;
        let mut notes = Vec::new();
        let adjusted = aliased(
            "contributor_adjusted_basis",
            p.contributor_adjusted_basis.as_deref(),
            "contributor_basis",
            p.contributor_basis.as_deref(),
            &mut notes,
            same_amount,
        )?;
        let basis = parse_asset_basis(
            &p.rule,
            adjusted,
            p.fmv_at_contribution.as_deref(),
            p.sec704c_method.as_deref(),
            p.basis_source.as_ref(),
            &mut notes,
        )?;
        let mut store = self.lock()?;
        let id = resolve_asset_ref(&store, &p.asset)?;
        store
            .set_asset_basis(id, basis)
            .map_err(|e| e.to_string())?;
        let a = store.ledger().get_asset(id).expect("resolved");
        Ok(Json(SetAssetTaxBasisOut {
            asset_id: id,
            basis: basis_out(a)?.ok_or("the basis just set has no figures")?,
            notes,
        }))
    }

    #[tool(
        name = "books.asset_labels",
        title = "Asset label sheet",
        description = "A printable sheet of asset labels (a PDF, handed over: run its download to save it): each label shows the company, the tag, a QR code of the tag, the description, and the serial number. Avery 5160/8160 (US letter, 30 labels of 1\" x 2-5/8\"); skip reuses a partly used sheet. Omit assets for every active asset with a tag. Save the file and tell the human to print it at 100% scale.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn asset_labels(
        &self,
        params: Parameters<AssetLabelsParams>,
    ) -> Result<Json<AssetLabelsOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let ledger = store.ledger();
        let assets: Vec<&ledger_core::Asset> = match &p.assets {
            Some(refs) => refs
                .iter()
                .map(|r| {
                    let id = resolve_asset_ref(&store, r)?;
                    Ok(ledger.get_asset(id).expect("resolved"))
                })
                .collect::<Result<_, String>>()?,
            None => ledger
                .registered_assets()
                .filter(|a| a.status == ledger_core::AssetStatus::Active && a.tag.is_some())
                .collect(),
        };
        let labels = assets
            .iter()
            .map(|a| {
                Ok(ledger_import::AssetLabel {
                    tag: a.tag.clone().ok_or_else(|| {
                        format!(
                            "asset {} has no tag: give it one with books.label_asset first",
                            a.id
                        )
                    })?,
                    description: a.description.clone(),
                    serial: a.serial.clone(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let pdf = ledger_import::asset_labels_pdf(
            &labels,
            &self.settings().company.name,
            ledger_import::LabelSheet::Avery5160,
            p.skip.unwrap_or(0) as usize,
        )
        .map_err(|e| e.to_string())?;
        let file = self.hand_over(
            &book_name(&store),
            &format!("asset-labels-{}.pdf", today()),
            "application/pdf",
            pdf,
        )?;
        Ok(Json(AssetLabelsOut {
            file,
            tags: labels.into_iter().map(|l| l.tag).collect(),
        }))
    }

    #[tool(
        name = "books.register_asset",
        title = "Register a fixed asset",
        description = "Register a fixed asset (equipment, vehicles, buildings). Links three accounts: cost (asset), accumulated depreciation (asset, contra), depreciation expense. in_service_date (ready and available for its use) starts depreciation; the acquisition_entry (its purchase or contribution, dated on or before that) gives the acquired date, which puts it on the register. Its tax basis (basis_rule and the contribution facts) as books.set_asset_tax_basis takes it. Depreciation is straight-line, book basis only — tax depreciation (MACRS, Section 179) stays with the CPA.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn register_asset(
        &self,
        params: Parameters<RegisterAssetParams>,
    ) -> Result<Json<RegisterAssetOut>, String> {
        let p = params.0;
        let mut notes = Vec::new();
        let in_service_date = parse_date(
            aliased(
                "in_service_date",
                p.in_service_date.as_deref(),
                "purchase_date",
                p.purchase_date.as_deref(),
                &mut notes,
                same_day,
            )?
            .ok_or("in_service_date is required: the day it was placed in service")?,
        )?;
        let mut store = self.lock()?;
        let base = store.ledger().base_currency();
        let cost_account = resolve_account(&store, &p.cost_account)?;
        let accum_depr_account = resolve_account(&store, &p.accum_depr_account)?;
        let expense_account = resolve_account(&store, &p.expense_account)?;
        let tag = match p.tag {
            Some(t) => t,
            None => store.ledger().next_asset_tag(self.settings().tag_prefix()),
        };
        // Checked before registering, so a bad tag or serial registers
        // nothing.
        store
            .ledger()
            .check_asset_tag_for(None, &tag)
            .map_err(|e| e.to_string())?;
        if let Some(serial) = &p.serial {
            ledger_core::assets::check_serial(serial)
                .map_err(|why| format!("bad serial number: {why}"))?;
        }
        if let Some(entry) = p.acquisition_entry {
            let e = store
                .ledger()
                .get_entry(entry)
                .ok_or_else(|| format!("unknown entry {entry}"))?;
            if !e
                .lines()
                .iter()
                .any(|l| l.account == cost_account && l.side == Side::Debit)
                || e.status() != ledger_core::EntryStatus::Posted
            {
                return Err(format!(
                    "entry {entry} can't be the acquisition: it must be posted and debit the cost account"
                ));
            }
            let reversed = (1..store.ledger().next_entry_id())
                .filter_map(|id| store.ledger().get_entry(id))
                .any(|x| x.reversal_of() == Some(entry));
            if e.reversal_of().is_some() || reversed {
                return Err(format!(
                    "entry {entry} can't be the acquisition: it is a reversal or has been reversed (link the entry that stands)"
                ));
            }
            if e.date() > in_service_date {
                return Err(format!(
                    "entry {entry} can't be the acquisition: it is dated {}, after the in-service date {in_service_date} (an asset is acquired before it serves: correct the in-service date, or the entry)",
                    e.date()
                ));
            }
        }
        // Checked before registering, so a bad tax basis registers nothing
        // (found by the spec-blind MCP suite, 2026-10-15).
        let rule = aliased(
            "basis_rule",
            p.basis_rule.as_deref(),
            "tax_basis_rule",
            p.tax_basis_rule.as_deref(),
            &mut notes,
            same_rule,
        )?;
        let adjusted = aliased(
            "contributor_adjusted_basis",
            p.contributor_adjusted_basis.as_deref(),
            "contributor_basis",
            p.contributor_basis.as_deref(),
            &mut notes,
            same_amount,
        )?;
        let basis = match rule {
            Some(rule) => Some(parse_asset_basis(
                rule,
                adjusted,
                p.fmv_at_contribution.as_deref(),
                p.sec704c_method.as_deref(),
                p.basis_source.as_ref(),
                &mut notes,
            )?),
            None if adjusted.is_some()
                || p.fmv_at_contribution.is_some()
                || p.sec704c_method.is_some()
                || p.basis_source.is_some() =>
            {
                return Err("the basis facts need basis_rule".into());
            }
            None => None,
        };
        let new = NewAsset {
            description: p.description,
            in_service_date,
            cost: Money::new(parse_amount(&p.cost)?, base),
            salvage_value: Money::new(
                parse_amount(p.salvage_value.as_deref().unwrap_or("0"))?,
                base,
            ),
            useful_life_months: p.useful_life_months,
            cost_account,
            accum_depr_account,
            expense_account,
        };
        // One registration is several commands: rehearse them all on a
        // copy first, and ask for the journal feature the basis needs
        // (B-27), so a refusal anywhere (the tag, the serial, the link, the
        // basis, books waiting for their format upgrade) journals nothing.
        {
            let mut copy = store.ledger().clone();
            let id = copy
                .register_asset(new.clone())
                .map_err(|e| e.to_string())?;
            copy.set_asset_tag(id, &tag).map_err(|e| e.to_string())?;
            if let Some(serial) = &p.serial {
                copy.set_asset_serial(id, Some(serial))
                    .map_err(|e| e.to_string())?;
            }
            if let Some(entry) = p.acquisition_entry {
                copy.set_asset_acquisition(id, entry)
                    .map_err(|e| e.to_string())?;
            }
            if let Some(basis) = &basis {
                copy.set_asset_basis(id, basis.clone())
                    .map_err(|e| e.to_string())?;
                store
                    .ensure_feature(ledger_store::format::ASSET_BASIS)
                    .map_err(|e| e.to_string())?;
            }
        }
        let asset_id = store.register_asset(new).map_err(|e| e.to_string())?;
        store
            .set_asset_tag(asset_id, &tag)
            .map_err(|e| e.to_string())?;
        if let Some(serial) = &p.serial {
            store
                .set_asset_serial(asset_id, Some(serial))
                .map_err(|e| e.to_string())?;
        }
        if let Some(entry) = p.acquisition_entry {
            store
                .set_asset_acquisition(asset_id, entry)
                .map_err(|e| e.to_string())?;
        }
        if let Some(basis) = basis {
            store
                .set_asset_basis(asset_id, basis)
                .map_err(|e| e.to_string())?;
        }
        let ledger = store.ledger();
        let a = ledger.get_asset(asset_id).expect("registered");
        Ok(Json(RegisterAssetOut {
            asset_id,
            tag,
            in_service_date: a.in_service_date.to_string(),
            acquired_date: ledger
                .asset_acquired_date(asset_id)
                .expect("registered")
                .to_string(),
            basis: basis_out(a)?,
            notes,
        }))
    }

    #[tool(
        name = "books.asset_schedule",
        title = "Depreciation schedule",
        description = "Month-by-month straight-line depreciation schedule for an asset: period, amount, accumulated, book value. Depreciation accrues by the day: a partial first month, the life's final month, and a month the schedule stops in (through a date, or a disposed asset's disposal day, which is held) are prorated by days; the lifetime total is exactly cost minus salvage.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn asset_schedule(
        &self,
        params: Parameters<AssetScheduleParams>,
    ) -> Result<Json<AssetScheduleOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let asset_id = resolve_asset(&store, p.asset_id, p.tag.as_deref())?;
        let asset = store
            .ledger()
            .get_asset(asset_id)
            .ok_or_else(|| format!("unknown asset {asset_id}"))?
            .clone();
        // The schedule clamps to the depreciation end internally, so an
        // unbounded default just means "the whole life".
        let through = match p.through {
            Some(t) => parse_date(&t)?,
            None => NaiveDate::MAX,
        };
        let schedule = depreciation_schedule(&asset, through)
            .map_err(|e| format!("asset {asset_id}'s schedule: {e}"))?
            .into_iter()
            .map(|l| ScheduleLineOut {
                period: l.period,
                amount: money(l.amount),
                accumulated: money(l.accumulated),
                book_value: money(l.book_value),
            })
            .collect();
        Ok(Json(AssetScheduleOut {
            asset_id: asset.id,
            description: asset.description.clone(),
            cost: money(asset.cost.amount),
            salvage_value: money(asset.salvage_value.amount),
            useful_life_months: asset.useful_life_months,
            status: match asset.status {
                AssetStatus::Active => "active".to_string(),
                AssetStatus::Disposed { .. } => "disposed".to_string(),
            },
            schedule,
        }))
    }

    #[tool(
        name = "books.list_assets",
        title = "List fixed assets",
        description = "List every registered fixed asset as of a date (as_of, default today): in-service and acquired dates, cost, and per asset accumulated_posted (what the books carry: runs posted, less reversals), accumulated_scheduled_through (what the schedule says is due by as_of), unposted_depreciation (the difference: run books.run_depreciation for it) and book_value (cost less posted: what the books carry); status (active or disposed) and held (acquired and not yet disposed) are as of as_of too, so a later disposal never shows. With its basis facts and tax figures. An asset whose figures can't be computed (one an earlier version took) lists with null figures and schedule_error.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_assets(
        &self,
        params: Parameters<ListAssetsParams>,
    ) -> Result<Json<ListAssetsOut>, String> {
        let p = params.0;
        let as_of = match &p.as_of {
            Some(d) => parse_date(d)?,
            None => today(),
        };
        let store = self.lock()?;
        let ledger = store.ledger();
        // The register's rows (one derivation, reports::register_row): an
        // asset acquired by as_of has one; a later one has no figures yet.
        let assets = ledger
            .registered_assets()
            .map(|a| {
                let acquired = ledger.asset_acquired_date(a.id).expect("registered");
                let disposed =
                    matches!(a.status, AssetStatus::Disposed { date, .. } if date <= as_of);
                // One asset whose figures can't be computed (one an earlier
                // version took) lists with why, never failing the whole
                // list.
                let row = ledger
                    .accumulated_posted(a.id, as_of)
                    .map(|posted| std::collections::BTreeMap::from([(a.id, posted)]))
                    .and_then(|posted| reports::register_row(ledger, a, as_of, &posted));
                let (row, schedule_error) = match row {
                    Ok(row) => (row, None),
                    Err(e) => (None, Some(e.to_string())),
                };
                let row = row.as_ref();
                let figure =
                    |f: fn(&ledger_core::AssetRegisterRow) -> Decimal| row.map(|r| money(f(r)));
                Ok(AssetSummary {
                    asset_id: a.id,
                    tag: a.tag.clone(),
                    serial: a.serial.clone(),
                    acquisition_entry: a.acquired_by,
                    basis: basis_out(a)?,
                    description: a.description.clone(),
                    in_service_date: a.in_service_date.to_string(),
                    acquired_date: acquired.to_string(),
                    cost: money(a.cost.amount),
                    useful_life_months: a.useful_life_months,
                    // As of as_of, like every figure here: a disposal
                    // after it hasn't happened yet (rules.md B-33).
                    status: if disposed { "disposed" } else { "active" }.to_string(),
                    held: acquired <= as_of && !disposed,
                    accumulated_posted: figure(|r| r.accumulated_posted),
                    accumulated_scheduled_through: figure(|r| r.accumulated_scheduled),
                    unposted_depreciation: figure(|r| {
                        r.accumulated_scheduled - r.accumulated_posted
                    }),
                    book_value: figure(|r| r.book_value),
                    schedule_error,
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(Json(ListAssetsOut {
            as_of: as_of.to_string(),
            assets,
        }))
    }

    #[tool(
        name = "books.run_depreciation",
        title = "Post depreciation",
        description = "Post one month of depreciation (Dr Depreciation Expense / Cr Accumulated Depreciation) for one asset, or for every active asset when asset_id is omitted: what the schedule says is due through the month's end, so a month skipped (or reversed) is caught up in the month run (caught_up says how much; a material miss is a restatement for the CPA, ASC 250). Idempotent: re-running a posted period returns the original entry. Nothing due (rounds to nothing, already caught up, life over) is listed under skipped. Exempt from the receipt policy: the asset and its schedule are the evidence.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn run_depreciation(
        &self,
        params: Parameters<RunDepreciationParams>,
    ) -> Result<Json<RunDepreciationOut>, String> {
        let p = params.0;
        let (py, pm) = parse_period_mcp(&p.period)?;
        let period_end = last_day_of_month(py, pm)?;
        let mut store = self.lock()?;
        let targets: Vec<(u64, String)> = match p.asset_id {
            Some(id) => {
                let a = store
                    .ledger()
                    .get_asset(id)
                    .ok_or_else(|| format!("unknown asset {id}"))?;
                vec![(id, a.description.clone())]
            }
            None => store
                .ledger()
                .registered_assets()
                .filter(|a| a.status == AssetStatus::Active)
                .map(|a| (a.id, a.description.clone()))
                .collect(),
        };
        let mut runs = Vec::with_capacity(targets.len());
        let mut skipped = Vec::new();
        for (id, description) in targets {
            let skip = |reason: String| SkippedDepreciation {
                asset_id: id,
                description: description.clone(),
                reason,
            };
            // Run-all skips assets not yet in service, and any whose
            // schedule can't be computed (an earlier version's), instead of
            // failing the whole batch.
            if p.asset_id.is_none()
                && let Some(asset) = store.ledger().get_asset(id)
                && asset.in_service_date > period_end
            {
                skipped.push(skip(format!(
                    "placed in service {}, after {}",
                    asset.in_service_date, p.period
                )));
                continue;
            }
            let r = match store.run_depreciation(id, &p.period) {
                Ok(r) => r,
                Err(ledger_store::StoreError::Ledger(
                    e @ (ledger_core::LedgerError::TooLargeToCompute(_)
                    | ledger_core::LedgerError::DateOutOfRange(_)),
                )) if p.asset_id.is_none() => {
                    skipped.push(skip(format!("its schedule can't be computed: {e}")));
                    continue;
                }
                Err(e) => return Err(e.to_string()),
            };
            let Some(entry_id) = r.entry_id else {
                skipped
                    .push(skip(r.nothing_due.unwrap_or_else(|| {
                        format!("nothing is due for {}", p.period)
                    })));
                continue;
            };
            runs.push(DepreciationRunOut {
                asset_id: r.asset_id,
                description,
                period: r.period,
                amount: money(r.amount),
                caught_up: money((r.amount - r.scheduled).max(Decimal::ZERO)),
                entry_id,
                already_posted: r.already_posted,
            });
        }
        Ok(Json(RunDepreciationOut { runs, skipped }))
    }

    #[tool(
        name = "books.dispose_asset",
        title = "Dispose of a fixed asset",
        description = "Dispose of a fixed asset (sell or retire). Without confirmed=true returns a preview (book value, gain/loss, the depreciation through the disposal day, which is held, and the disposal entry's lines) and posts nothing. With confirmed=true posts it after the receipt policy: file the bill of sale with books.file_document and pass its ids as receipt_attachment_ids. Depreciation a run posted past the disposal day is credited back (excess_depreciation); a later month's run is refused (reverse it first). Name the buyer (a party id or name): [books] parties = \"require\" refuses a sale without one.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn dispose_asset(
        &self,
        params: Parameters<DisposeAssetParams>,
    ) -> Result<Json<DisposeAssetOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let date = parse_date(&p.date)?;
        let base = store.ledger().base_currency();
        let proceeds = Money::new(parse_amount(&p.proceeds)?, base);
        let proceeds_account = resolve_account(&store, &p.proceeds_account)?;
        let gain_loss_account = resolve_account(&store, &p.gain_loss_account)?;
        let asset_id = resolve_asset(&store, p.asset_id, p.tag.as_deref())?;
        let buyer = p
            .buyer
            .as_deref()
            .map(|r| resolve_party(&store, r))
            .transpose()?;
        let asset = store
            .ledger()
            .get_asset(asset_id)
            .ok_or_else(|| format!("unknown asset {asset_id}"))?
            .clone();
        let preview = store
            .dispose_preview(asset_id, date, proceeds)
            .map_err(|e| e.to_string())?;

        // The would-be disposal entry's lines, for the preview and the
        // receipt-policy check.
        let raw: Vec<(AccountId, Side, Money)> = Store::disposal_lines(
            &asset,
            &preview,
            proceeds.amount,
            proceeds_account,
            gain_loss_account,
        )
        .into_iter()
        .map(|(a, side, amount)| (a, side, Money::new(amount, base)))
        .collect();
        // Disposals are money movement: the bill of sale is the receipt.
        let chart = store.ledger().chart();
        let code_of = |id: AccountId| {
            chart
                .get(id)
                .map(|a| a.code.clone())
                .ok_or_else(|| format!("unknown account {id}"))
        };
        let adjusts =
            !(preview.excess_depreciation.is_zero() && preview.catch_up_depreciation.is_zero());
        let codes: Vec<String> = [
            asset.accum_depr_account,
            proceeds_account,
            asset.cost_account,
            gain_loss_account,
        ]
        .into_iter()
        .chain(adjusts.then_some(asset.expense_account))
        .map(code_of)
        .collect::<Result<_, _>>()?;
        let code_refs: Vec<&str> = codes.iter().map(|s| s.as_str()).collect();
        let debits = preview.accumulated
            + proceeds.amount
            + preview.catch_up_depreciation
            + if preview.gain_loss < Decimal::ZERO {
                -preview.gain_loss
            } else {
                Decimal::ZERO
            };
        let receipt_required = self
            .settings()
            .policy
            .receipt_required_for(&code_refs, debits);

        let out = DisposeAssetOut {
            status: "preview".to_string(),
            asset_id,
            entry_id: 0,
            accumulated: money(preview.accumulated),
            book_value: money(preview.book_value),
            proceeds: money(proceeds.amount),
            gain_loss: money(preview.gain_loss),
            final_depreciation: money(preview.final_depreciation),
            excess_depreciation: money(preview.excess_depreciation),
            catch_up_depreciation: money(preview.catch_up_depreciation),
            lines: preview_lines_for(&store, &raw)?,
            receipt_required,
        };
        if !p.confirmed.unwrap_or(false) {
            return Ok(Json(out));
        }
        let attachment_ids = p.receipt_attachment_ids.unwrap_or_default();
        if receipt_required && attachment_ids.is_empty() {
            return Err(ReceiptRequiredError {
                code: "receipt_required".to_string(),
                message: format!(
                    "disposing {} moves {} in debits ({}); file the bill of sale with books.file_document, then retry with receipt_attachment_ids",
                    asset.description,
                    money(debits),
                    self.settings().policy.describe()
                ),
                entry_id: None,
                asset_id: Some(asset_id),
                debits: debits.to_string(),
                policy: self.settings().policy.describe(),
                remedy_tool: "books.file_document".to_string(),
                remedy_hint:
                    "file the bill of sale with books.file_document, then retry books.dispose_asset with confirmed=true and receipt_attachment_ids=[<id>]"
                        .to_string(),
                split_group: Vec::new(),
            }
            .to_tool_error());
        }
        for aid in &attachment_ids {
            store.check_linkable(*aid).map_err(|e| e.to_string())?;
        }
        if buyer.is_none()
            && !proceeds.amount.is_zero()
            && self.settings().parties == ledger_config::PartyPolicy::Require
        {
            return Err(party_required(
                "pass buyer: the party the asset was sold to",
            ));
        }
        let entry_id = store
            .dispose_asset(
                asset_id,
                date,
                proceeds,
                proceeds_account,
                gain_loss_account,
            )
            .map_err(|e| e.to_string())?;
        for aid in &attachment_ids {
            store
                .link_receipt(*aid, entry_id)
                .map_err(|e| e.to_string())?;
        }
        match buyer {
            Some(b) => store.set_entry_party(entry_id, Some(b)),
            None if proceeds.amount.is_zero() => {
                store.set_entry_no_party(entry_id, "asset retired with no proceeds")
            }
            None => Ok(()),
        }
        .map_err(|e| e.to_string())?;
        let mut posted = out;
        posted.status = "posted".to_string();
        posted.entry_id = entry_id;
        Ok(Json(posted))
    }
}
