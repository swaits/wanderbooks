use crate::*;

/// A JSON object: the shape every figure query answers in.
pub(crate) type JsonMap = serde_json::Map<String, Value>;

/// The `AccountType` an example's account type names.
pub(crate) fn account_type(a: &AccountSpec, id: &str) -> AccountType {
    match a.kind.as_str() {
        "asset" => AccountType::Asset,
        "liability" => AccountType::Liability,
        "equity" => AccountType::Equity,
        "revenue" => AccountType::Revenue,
        "expense" => AccountType::Expense,
        other => panic!("{id}: account type {other:?}"),
    }
}

/// The period `[from, to]` a period query covers, each date omitted
/// resolved by the one rule (BS-period-3) on `setup.today`; panics on
/// `as_of`, on an omitted date without `setup.today`, and on a period the
/// rule refuses.
pub(crate) fn period(ex: &Example, x: &Expect) -> (NaiveDate, NaiveDate) {
    let q = x.query();
    resolved_q_period(ex, &q).unwrap_or_else(|e| panic!("{}: {}: {e}", ex.id, x.query))
}

/// [`period`], or the rule's refusal.
pub(crate) fn resolved_period(
    ex: &Example,
    x: &Expect,
) -> Result<(NaiveDate, NaiveDate), LedgerError> {
    resolved_q_period(ex, &x.query())
}

/// A query's period, or the rule's refusal.
pub(crate) fn resolved_q_period(
    ex: &Example,
    q: &Query,
) -> Result<(NaiveDate, NaiveDate), LedgerError> {
    let id = &ex.id;
    assert!(
        q.as_of.is_none(),
        "{id}: {} covers a period (from, to), not as_of",
        q.name
    );
    let from = q.from.as_deref().map(|s| date(s, id));
    let to = q.to.as_deref().map(|s| date(s, id));
    // A `period` query asks the rule itself, both dates given or not.
    if let (Some(from), Some(to)) = (from, to)
        && q.name != "period"
    {
        assert!(from <= to, "{id}: {} from {from} after to {to}", q.name);
        return Ok((from, to));
    }
    defaults(ex, &q.name).statement(from, to)
}

/// The day an as-of query is made as of (omitted: `setup.today`,
/// BS-period-3); panics on `from` or `to`.
pub(crate) fn day(ex: &Example, x: &Expect) -> NaiveDate {
    let q = x.query();
    resolved_q_day(ex, &q).unwrap_or_else(|e| panic!("{}: {}: {e}", ex.id, x.query))
}

/// A query's day, or the rule's refusal.
pub(crate) fn resolved_q_day(ex: &Example, q: &Query) -> Result<NaiveDate, LedgerError> {
    let id = &ex.id;
    assert!(
        q.from.is_none() && q.to.is_none(),
        "{id}: {} is as of a day (as_of), not a period",
        q.name
    );
    match q.as_of.as_deref() {
        Some(s) => Ok(date(s, id)),
        None => defaults(ex, &q.name).as_of(None).pipe(Ok),
    }
}

/// Rust has no pipe; this keeps the day rule a single expression.
pub(crate) trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}
/// What an omitted date means for `ex` (BS-period-3): its fiscal year
/// on `setup.today`, which `query` needs.
pub(crate) fn defaults(ex: &Example, query: &str) -> PeriodDefaults {
    let today = ex.setup.today.as_deref().unwrap_or_else(|| {
        panic!(
            "{}: {query} omits a date: setup.today says what it means (BS-period-3)",
            ex.id
        )
    });
    let company = Company {
        fiscal_year_start: fiscal_year_start(ex),
        ..Company::default()
    };
    period_defaults(&company, date(today, &format!("{} today", ex.id)))
}

/// `setup.fiscal_year_start`, MM-DD, as (month, day).
pub(crate) fn fiscal_year_start(ex: &Example) -> (u32, u32) {
    let s = &ex.setup.fiscal_year_start;
    let (m, d) = s
        .split_once('-')
        .unwrap_or_else(|| panic!("{}: fiscal_year_start {s:?}", ex.id));
    let md = (
        m.parse()
            .unwrap_or_else(|_| panic!("{}: fiscal_year_start {s:?}", ex.id)),
        d.parse()
            .unwrap_or_else(|_| panic!("{}: fiscal_year_start {s:?}", ex.id)),
    );
    assert!(
        NaiveDate::from_ymd_opt(2001, md.0, md.1).is_some(),
        "{}: fiscal_year_start {s:?}",
        ex.id
    );
    md
}

/// A refusal in the examples' words (bookspec/README.md), engine-neutral:
/// what was refused, never the engine's message.
pub(crate) fn refusal(e: &LedgerError) -> String {
    match e {
        LedgerError::InvalidDateRange { .. } => "end_before_start",
        LedgerError::StartAfterToday { .. } => "start_after_today",
        LedgerError::EmptyAccountName => "blank",
        LedgerError::AccountNameUnchanged { .. } => "unchanged",
        LedgerError::DuplicateAccountName { .. } => "sibling_has_it",
        LedgerError::AccountNameHoldsTin => "holds_a_tin",
        LedgerError::InvalidAccountName(_) => "not_one_line",
        LedgerError::DuplicateAccountCode(_) => "code_taken",
        LedgerError::TaxCategoryRequired { .. } => "tax_category_required",
        LedgerError::DepreciationPastDisposal(_) => "depreciation_past_disposal",
        // A reversal dated before its original (D-73) is the examples'
        // "entry_irreversible" too: bookspec's depreciation example words
        // the refusal that way, whichever engine check makes it.
        LedgerError::AssetEntryIrreversible(..) | LedgerError::ReversalBeforeOriginal { .. } => {
            "entry_irreversible"
        }
        LedgerError::AssetAlreadyDisposed(_) => "already_disposed",
        LedgerError::InvalidDepreciationPeriod(_) => "invalid_depreciation_period",
        LedgerError::InvalidSchedule(_) => "invalid_schedule",
        LedgerError::UntaggedControlLine { .. } => "untagged_control_line",
        LedgerError::PaymentBeforeDocument { .. } => "payment_before_document",
        LedgerError::DuplicateVendorRef { .. } => "duplicate_vendor_reference",
        // An invoice with a bill, and documents of two parties, are the one
        // refusal: a customer and a vendor are two parties (BS-arap-6).
        LedgerError::SettlementMixedKinds(_) | LedgerError::SettlementPartiesDiffer(..) => {
            "settlement_mixes_parties"
        }
        LedgerError::SettlementDuplicateDocument(_) => "settlement_repeats_a_document",
        LedgerError::Overpayment(..) => "application_above_owed",
        LedgerError::DiscountNeedsReason { .. } => "discount_needs_reason",
        LedgerError::DiscountOnAssetBill { .. } => "discount_on_asset_bill",
        LedgerError::FeeAboveCash { .. } => "fee_above_cash",
        LedgerError::UnbalancedEntry { .. } => "unbalanced",
        LedgerError::TooFewLines => "too_few_lines",
        LedgerError::NegativeAmountLine => "negative_amount",
        LedgerError::ZeroAmountLine => "zero_amount",
        LedgerError::FinerThanMinorUnit { .. } => "finer_than_minor_unit",
        LedgerError::ParentTypeMismatch { .. } => "parent_type_mismatch",
        LedgerError::EntryAlreadyReversed(..) => "already_reversed",
        LedgerError::ClosedPeriod(..) => "period_closed",
        other => return format!("another refusal: {other}"),
    }
    .to_string()
}

/// [`refusal`] of a command the books refused: the engine's reason, or the
/// store's own (none is in the examples' words).
pub(crate) fn store_refusal(e: &StoreError) -> String {
    match e {
        StoreError::Ledger(e) => refusal(e),
        other => format!("another refusal: {other}"),
    }
}

/// The text of the bookspec example file `file`, carried by the pinned
/// `bookspec-checker` crate.
pub(crate) fn example_text(file: &str) -> &'static str {
    bookspec_checker::examples()
        .into_iter()
        .find(|(name, _)| *name == file)
        .unwrap_or_else(|| panic!("no bookspec example {file}"))
        .1
}

pub(crate) fn date(s: &str, what: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap_or_else(|e| panic!("{what}: {s:?}: {e}"))
}

/// An exact decimal string: digits, an optional sign and point; never a
/// float's exponent.
pub(crate) fn exact(s: &str, what: &str) -> Decimal {
    assert!(
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_digit() || c == '.' || c == '-')
            && s.matches('.').count() <= 1,
        "{what}: {s:?} is not an exact decimal string"
    );
    s.parse()
        .unwrap_or_else(|e| panic!("{what}: {s:?} is not a decimal: {e}"))
}

/// The refusal a store error is reported as: the examples' words when the
/// engine's reason is one of them, the engine's own otherwise.
pub(crate) fn store_refusal_obj(e: &StoreError) -> Refusal {
    match e {
        StoreError::Ledger(e) => Refusal::of_reason(refusal(e)),
        other => Refusal::of_reason(format!("another refusal: {other}")),
    }
}

/// The worker's refusal for an engine error the examples' words don't
/// name.
pub(crate) fn engine_refusal(rule: RuleId, e: impl std::fmt::Display) -> Refusal {
    Refusal::new(rule, format!("another refusal: {e}"))
}

/// A JSON object of named strings (every figure an exact decimal string,
/// never a number).
pub(crate) fn json_figures<I>(figures: I) -> Value
where
    I: IntoIterator<Item = (String, String)>,
{
    Value::Object(
        figures
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect(),
    )
}

/// The accounts the codes name.
pub(crate) fn codes(l: &Ledger, cs: &[String], id: &str) -> Vec<ledger_core::AccountId> {
    cs.iter()
        .map(|c| {
            l.chart()
                .find_by_code(c)
                .unwrap_or_else(|| panic!("{id}: no account {c}"))
                .id
        })
        .collect()
}

/// Every key is one of `keys`, and every one of `keys` is given: a figure
/// query lists everything it covers, so an example can't leave a figure
/// unchecked by leaving it out.
pub(crate) fn exactly(ex: &Example, x: &Expect, keys: &BTreeSet<String>) {
    let given: BTreeSet<String> = x.figures(&ex.id).keys().cloned().collect();
    assert_eq!(
        &given, keys,
        "{}: {} lists exactly what it covers",
        ex.id, x.query
    );
}

/// No dates: a query about one thing the books did, not about a day.
pub(crate) fn undated(ex: &Example, x: &Expect) {
    assert!(
        x.from.is_none() && x.to.is_none() && x.as_of.is_none(),
        "{}: {} takes no dates",
        ex.id,
        x.query
    );
}

/// A month key, "YYYY-MM", checked.
pub(crate) fn month_key(key: &str, what: &str) {
    assert!(
        key.len() == 7 && NaiveDate::parse_from_str(&format!("{key}-01"), "%Y-%m-%d").is_ok(),
        "{what}: {key:?} is no month (YYYY-MM)"
    );
}

/// Check expectations whose values are named figures: each key one of
/// `figures`' (an unknown one refused), its value the figure's text.
pub(crate) fn named<K: Borrow<str> + Ord>(
    ex: &Example,
    x: &Expect,
    what: &str,
    figures: &BTreeMap<K, String>,
) {
    for (key, want) in x.figures(&ex.id) {
        let got = figures
            .get(key.as_str())
            .unwrap_or_else(|| panic!("{what}: no figure {key:?}"));
        assert_eq!(got, want, "{what}: {key}");
    }
}

/// A decimal figure as an example writes it: two places at least, as the
/// books keep money (exact; "0.00", "-14.20").
pub(crate) fn figure(d: Decimal) -> String {
    let mut d = d.normalize();
    if d.scale() < 2 {
        d.rescale(2);
    }
    d.to_string()
}
