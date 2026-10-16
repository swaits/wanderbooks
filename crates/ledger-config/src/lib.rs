//! Company settings from the books dir's `wanderbooks.toml`.
//!
//! `wanderbooks init` writes that file next to `books-data/`; onboarding
//! fills in `[company]`. The server reads the company name (report headers),
//! the fiscal year start (fiscal-year-to-date reports), the members of a
//! multi-member LLC (`[[company.members]]`: name, share, capital account —
//! for the statement of members' equity), `[books] accounting_basis`
//! (report footers), and `[books] receipt_threshold` (the receipt policy,
//! unless `config.json` sets one). The fiscal year *end* is never stored: it is the day before
//! the next start, so the two can't disagree.

pub mod receipt_policy;
pub use receipt_policy::ReceiptPolicy;

use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::path::Path;

/// The company the books belong to.
#[derive(Clone, Debug, PartialEq)]
pub struct Company {
    /// Legal name, for report headers.
    pub name: String,
    /// Month and day the fiscal year starts. Default January 1.
    pub fiscal_year_start: (u32, u32),
    /// Members of a multi-member LLC; empty for other entities.
    pub members: Vec<Member>,
    /// "Accrual" or "Cash", for report footers. Default "Accrual".
    pub basis: String,
    /// The income tax return the business files, if known: from
    /// `tax_form`, else from `tax_classification`.
    pub tax_form: Option<ledger_core::TaxForm>,
    /// `[books] close_into`: the equity account a period close puts net
    /// income in (a sole proprietor's capital, say). Unset: Retained
    /// Earnings, or members' capital accounts when there are members.
    pub close_into: Option<String>,
    /// `[books] close_sweep`: equity accounts (draws, dividends,
    /// distributions) a close zeroes into `close_into`.
    pub close_sweep: Vec<String>,
    /// Every simple `[company]` setting (name, EIN, formation date, ...),
    /// in file order, as written: the full profile for display.
    pub profile: Vec<(String, String)>,
}

/// One member of a multi-member LLC.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    /// Name, as it appears on reports.
    pub name: String,
    /// Share of income, e.g. 0.50.
    pub share: Decimal,
    /// Code of their capital account in the chart.
    pub capital_account: String,
    /// Code of their distributions account, if distributions are recorded
    /// separately from capital (zeroed into capital at close).
    pub distribution_account: Option<String>,
    /// `files_jointly_with`: the other member this member files a joint
    /// individual return with (spouses). Each still gets their own K-1
    /// (a partnership reports each partner separately, IRS Instructions
    /// for Form 1065, Schedule K-1); both K-1s go on the one Form 1040
    /// (Schedule E, Part II). Informational: the CPA package says so.
    pub files_jointly_with: Option<String>,
}

impl Default for Company {
    fn default() -> Self {
        Self {
            name: "Unnamed company".to_string(),
            fiscal_year_start: (1, 1),
            members: Vec::new(),
            basis: "Accrual".to_string(),
            tax_form: None,
            close_into: None,
            close_sweep: Vec::new(),
            profile: Vec::new(),
        }
    }
}

impl Company {
    /// The members as party shares see them ([`ledger_core::PartyMember`]):
    /// each member whose name (ignoring case and surrounding space) is a
    /// party with the member role, and whose capital account is in the
    /// chart. The party not archived of that name when there is one, else
    /// the most recent archived one: archiving a member ("no longer dealt
    /// with; history stays") never takes their history away. Members
    /// missing either are left out, so their shares can't be derived (and
    /// the viewer shows no capital statement for them).
    pub fn party_members(&self, ledger: &ledger_core::Ledger) -> Vec<ledger_core::PartyMember> {
        let is_member = |p: &&ledger_core::Party| p.roles.contains(&ledger_core::Role::Member);
        self.members
            .iter()
            .filter_map(|m| {
                let key = ledger_core::parties::name_key(&m.name);
                let party = match ledger.party_by_name(&m.name) {
                    Some(p) => Some(p).filter(is_member)?,
                    None => ledger
                        .parties()
                        .filter(|p| p.archived && ledger_core::parties::name_key(&p.name) == key)
                        .filter(is_member)
                        .max_by_key(|p| p.id)?,
                };
                Some(ledger_core::PartyMember {
                    party: party.id,
                    share: m.share,
                    capital: ledger.chart().find_by_code(&m.capital_account)?.id,
                    distributions: m
                        .distribution_account
                        .as_deref()
                        .and_then(|c| ledger.chart().find_by_code(c))
                        .map(|a| a.id),
                })
            })
            .collect()
    }

    /// The fiscal year containing `d`: `(first day, last day)`, inclusive.
    pub fn fiscal_year_containing(&self, d: NaiveDate) -> (NaiveDate, NaiveDate) {
        let (m, day) = self.fiscal_year_start;
        let on = |year| NaiveDate::from_ymd_opt(year, m, day).expect("validated month-day");
        let start = if on(d.year()) <= d {
            on(d.year())
        } else {
            on(d.year() - 1)
        };
        let end = on(start.year() + 1)
            .pred_opt()
            .expect("not the minimum date");
        (start, end)
    }
}

/// What the server takes from `wanderbooks.toml`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BooksSettings {
    /// `[company]`.
    pub company: Company,
    /// `[books] receipt_threshold`, validated as a decimal string.
    pub receipt_threshold: Option<String>,
    /// `[books] cash_accounts`: account codes that hold cash (bank,
    /// savings, cash on hand), for cash reports and dashboards. Empty:
    /// callers decide (e.g. by account name).
    pub cash_accounts: Vec<String>,
    /// `[books] financing_accounts`: liability account codes the cash flow
    /// statement treats as financing (loans, and what owners advanced:
    /// ASC 230-10-45-14(b)). Empty: callers decide.
    pub financing_accounts: Vec<String>,
    /// `[books] investing_accounts`: asset account codes the cash flow
    /// statement treats as investing (loans the company makes, investments
    /// it buys: ASC 230-10-20). Registered fixed assets need not be listed.
    pub investing_accounts: Vec<String>,
    /// `[books] asset_tag_prefix`: the prefix of generated asset tags
    /// ("WL" for "WL-0001"). Unset: [`BooksSettings::asset_tag_prefix`]
    /// derives it from the company name.
    pub asset_tag_prefix: Option<String>,
    /// `[books] parties`: how hard the agent asks for each entry's
    /// counterparty. Unset: [`PartyPolicy::Suggest`].
    pub parties: PartyPolicy,
    /// `[vault] key_file`: the TIN vault's key file. Unset: the vault's
    /// default (`~/.config/wanderbooks/vault.key`).
    pub vault_key_file: Option<std::path::PathBuf>,
}

impl BooksSettings {
    /// The prefix generated asset tags use: `[books] asset_tag_prefix`, or
    /// else the initials of the company name's words, legal-form words
    /// (LLC, Inc, Corp, ...) left out, at most 8 ("Wanderland Labs LLC" →
    /// "WL"); "FA" (fixed asset) when the name gives none.
    pub fn asset_tag_prefix(&self) -> String {
        if let Some(p) = &self.asset_tag_prefix {
            return p.clone();
        }
        const FORMS: &[&str] = &[
            "llc",
            "l",
            "c",
            "inc",
            "incorporated",
            "corp",
            "corporation",
            "co",
            "company",
            "ltd",
            "limited",
            "llp",
            "lp",
            "pllc",
            "pc",
            "pa",
            "the",
            "and",
            "of",
        ];
        let initials: String = self
            .company
            .name
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| !w.is_empty() && !FORMS.contains(&w.to_ascii_lowercase().as_str()))
            .filter_map(|w| w.chars().next())
            .filter(char::is_ascii_alphabetic)
            .map(|c| c.to_ascii_uppercase())
            .take(8)
            .collect();
        if initials.is_empty() || self.company.name == Company::default().name {
            "FA".to_string()
        } else {
            initials
        }
    }
}

#[derive(Deserialize)]
struct BooksToml {
    #[serde(default)]
    company: CompanyToml,
    #[serde(default)]
    books: BooksSection,
    #[serde(default)]
    vault: VaultSection,
}

#[derive(Default, Deserialize)]
struct VaultSection {
    key_file: Option<String>,
}

#[derive(Default, Deserialize)]
struct CompanyToml {
    name: Option<String>,
    fiscal_year_start: Option<String>,
    tax_form: Option<String>,
    tax_classification: Option<String>,
    #[serde(default)]
    members: Vec<MemberToml>,
}

#[derive(Deserialize)]
struct MemberToml {
    name: String,
    share: String,
    capital_account: Option<String>,
    distribution_account: Option<String>,
    files_jointly_with: Option<String>,
}

#[derive(Default, Deserialize)]
struct BooksSection {
    receipt_threshold: Option<String>,
    accounting_basis: Option<String>,
    close_into: Option<String>,
    #[serde(default)]
    close_sweep: Vec<String>,
    #[serde(default)]
    cash_accounts: Vec<String>,
    #[serde(default)]
    financing_accounts: Vec<String>,
    #[serde(default)]
    investing_accounts: Vec<String>,
    asset_tag_prefix: Option<String>,
    parties: Option<String>,
}

/// The return a federal tax classification files (docs/spec/sources/
/// irs-entity-classification.md), read by whole words, ignoring case:
/// "partnership" → 1065; "S corporation" (S corp, S-corp, scorp) →
/// 1120-S; "C corporation", or "corporation", "corp", or "association"
/// alone → 1120; "disregarded", "sole proprietor(ship)", or
/// "single-member" → Schedule C. Anything else, or words naming two
/// returns ("corporation or S corporation"), names none.
fn form_for_classification(c: &str) -> Option<ledger_core::TaxForm> {
    use ledger_core::TaxForm;
    let lower = c.to_lowercase();
    let words: Vec<&str> = lower
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let has = |w: &str| words.contains(&w);
    let mut found = Vec::new();
    if has("partnership") {
        found.push(TaxForm::Form1065);
    }
    for (i, w) in words.iter().enumerate() {
        let prev = i.checked_sub(1).map(|p| words[p]);
        let form = match *w {
            "scorp" => Some(TaxForm::Form1120S),
            "ccorp" | "association" => Some(TaxForm::Form1120),
            "corp" | "corporation" if prev == Some("s") => Some(TaxForm::Form1120S),
            "corp" | "corporation" => Some(TaxForm::Form1120),
            _ => None,
        };
        found.extend(form);
    }
    if has("disregarded") || has("sole") || words.windows(2).any(|p| p == ["single", "member"]) {
        found.push(TaxForm::ScheduleC);
    }
    found.sort();
    found.dedup();
    match found.as_slice() {
        [one] => Some(*one),
        _ => None,
    }
}

/// Parse a share: "50%" or "0.5".
fn parse_share(name: &str, s: &str) -> Result<Decimal, String> {
    let s = s.trim();
    let bad = || format!("member {name}: share {s:?} must be like \"50%\" or \"0.5\"");
    let share = match s.strip_suffix('%') {
        Some(pct) => pct.trim().parse::<Decimal>().map_err(|_| bad())? / Decimal::from(100),
        None => s.parse::<Decimal>().map_err(|_| bad())?,
    };
    if share <= Decimal::ZERO || share > Decimal::ONE {
        return Err(bad());
    }
    Ok(share)
}

/// Parse `"MM-DD"`: the first day of a month (a fiscal year ends on the
/// last day of a month: IRS Pub. 538, docs/spec/sources/
/// irs-periods-and-methods.md).
fn parse_month_day(s: &str) -> Result<(u32, u32), String> {
    let bad = || format!("fiscal_year_start {s:?}: expected \"MM-DD\", e.g. \"01-01\"");
    let (m, d) = s.split_once('-').ok_or_else(bad)?;
    let (m, d): (u32, u32) = (m.parse().map_err(|_| bad())?, d.parse().map_err(|_| bad())?);
    if !(1..=12).contains(&m) {
        return Err(bad());
    }
    if d != 1 {
        return Err(format!(
            "fiscal_year_start {s:?}: a fiscal year starts on the first of a month (it ends on the last day of one: IRS Pub. 538)"
        ));
    }
    Ok((m, d))
}

/// Account codes as written, trimmed; a blank one is refused.
fn codes(key: &str, list: Vec<String>) -> Result<Vec<String>, String> {
    list.into_iter()
        .map(|c| {
            let c = c.trim().to_string();
            if c.is_empty() {
                Err(format!("{key}: an account code is blank"))
            } else {
                Ok(c)
            }
        })
        .collect()
}

/// A decimal string as the books take one: exactly `-?digits[.digits]`.
/// The decimal parser alone also takes "1_000", "+5", ".5", and
/// exponents ("1e3"); a setting or tool argument never means those.
pub fn decimal(s: &str) -> Option<Decimal> {
    let digits = s.strip_prefix('-').unwrap_or(s);
    let (whole, frac) = match digits.split_once('.') {
        Some((w, f)) => (w, Some(f)),
        None => (digits, None),
    };
    let all_digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
    if !all_digits(whole) || frac.is_some_and(|f| !all_digits(f)) {
        return None;
    }
    s.parse().ok()
}

/// The receipt policy in force: `config_json`'s `receipt_policy` when the
/// file sets one, else `[books] receipt_threshold`, else the default
/// ($25.00); validated (crate::receipt_policy INVARIANT 4). The server and
/// the viewer both call this, so they enforce and show one rule.
pub fn effective_receipt_policy(
    config_json: &Path,
    settings: &BooksSettings,
) -> Result<ReceiptPolicy, String> {
    #[derive(Deserialize)]
    struct ConfigJson {
        #[serde(default)]
        receipt_policy: Option<ReceiptPolicy>,
    }
    let from_json = if config_json.exists() {
        let raw = std::fs::read_to_string(config_json)
            .map_err(|e| format!("{}: {e}", config_json.display()))?;
        serde_json::from_str::<ConfigJson>(&raw)
            .map_err(|e| format!("bad config {}: {e}", config_json.display()))?
            .receipt_policy
    } else {
        None
    };
    let policy = match (from_json, &settings.receipt_threshold) {
        (Some(p), _) => p,
        (None, Some(t)) => ReceiptPolicy {
            require_receipts_above: t.clone(),
            ..ReceiptPolicy::default()
        },
        (None, None) => ReceiptPolicy::default(),
    };
    policy.validate()?;
    Ok(policy)
}

/// Parse `wanderbooks.toml` text.
///
/// ```text
/// GOAL: read the company's settings, refusing any the books can't honor
///       rather than guessing.
/// SOURCE: IRS Pub. 538 (docs/spec/sources/irs-periods-and-methods.md:
///         a fiscal year ends on the last day of a month); Treas. Reg.
///         §301.7701-3 and the Schedule C instructions
///         (docs/spec/sources/irs-entity-classification.md); IRC §704(a)
///         (shares from the agreement: docs/spec/sources/irs-partnerships.md).
/// INVARIANTS:
///   1. Every setting is optional; an empty file is a calendar-year,
///      accrual-basis "Unnamed company" with no members.
///   2. fiscal_year_start MUST be the first of a month ("MM-01").
///   3. Members: each MUST have a name and a capital account, names and
///      capital accounts MUST be distinct (and no member's distributions
///      account another's capital), and shares (each in (0, 1], "50%" or
///      "0.5") MUST sum to exactly 1.
///   4. tax_form wins over tax_classification (which is then not read);
///      a classification is read by whole words, ignoring case, and MUST
///      name exactly one return (source table). tax_form, basis, and
///      classification values ignore case.
///   5. receipt_threshold MUST be a decimal at least zero.
///   6. Account codes (cash, financing, investing, close_into,
///      close_sweep, members' capital and distributions) are trimmed; a
///      blank one is refused (a blank distributions account means none). close_into is
///      refused with members (a close goes to their capital accounts).
///   7. profile holds every simple [company] value in file order:
///      strings as written, numbers, booleans, and dates in TOML's
///      spelling; tables and arrays (members) are left out.
///   8. NEVER accept a file that would be read differently than written:
///      an unknown basis, tax form, or classification is refused.
/// ```
pub fn parse(text: &str) -> Result<BooksSettings, String> {
    let raw: BooksToml = toml::from_str(text).map_err(|e| e.to_string())?;
    let mut company = Company::default();
    if let Some(name) = raw.company.name.filter(|n| !n.trim().is_empty()) {
        company.name = name.trim().to_string();
    }
    if let Some(fy) = raw.company.fiscal_year_start {
        company.fiscal_year_start = parse_month_day(fy.trim())?;
    }
    for m in raw.company.members {
        let share = parse_share(&m.name, &m.share)?;
        let capital_account = m
            .capital_account
            .filter(|c| !c.trim().is_empty())
            .ok_or_else(|| format!("member {}: capital_account is required", m.name))?;
        company.members.push(Member {
            name: m.name.trim().to_string(),
            share,
            capital_account: capital_account.trim().to_string(),
            distribution_account: m
                .distribution_account
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty()),
            files_jointly_with: m
                .files_jointly_with
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty()),
        });
    }
    for (i, m) in company.members.iter().enumerate() {
        if m.name.is_empty() {
            return Err(format!("member {}: name is blank", i + 1));
        }
        for other in &company.members[..i] {
            if other.name == m.name {
                return Err(format!("member {} is listed twice", m.name));
            }
            if other.capital_account == m.capital_account {
                return Err(format!(
                    "members {} and {} share capital account {}",
                    other.name, m.name, m.capital_account
                ));
            }
        }
        for other in &company.members {
            if other.distribution_account.as_deref() == Some(m.capital_account.as_str()) {
                return Err(format!(
                    "{}'s distributions account {} is {}'s capital account",
                    other.name, m.capital_account, m.name
                ));
            }
        }
    }
    // files_jointly_with names another member, and both say so.
    for m in &company.members {
        if let Some(j) = &m.files_jointly_with {
            if *j == m.name {
                return Err(format!(
                    "member {}: files_jointly_with names themself",
                    m.name
                ));
            }
            match company.members.iter().find(|o| o.name == *j) {
                None => {
                    return Err(format!(
                        "member {}: files_jointly_with {j:?} isn't a member",
                        m.name
                    ));
                }
                Some(o) if o.files_jointly_with.as_deref() != Some(m.name.as_str()) => {
                    return Err(format!(
                        "member {} files jointly with {j}, but {j}'s files_jointly_with doesn't name {}: set it on both",
                        m.name, m.name
                    ));
                }
                Some(_) => {}
            }
        }
    }
    if !company.members.is_empty() {
        let total: Decimal = company.members.iter().map(|m| m.share).sum();
        if total != Decimal::ONE {
            return Err(format!("member shares sum to {total}, not 100%"));
        }
    }
    company.tax_form =
        match (&raw.company.tax_form, &raw.company.tax_classification) {
            (Some(f), _) => Some(ledger_core::TaxForm::parse(f).ok_or_else(|| {
                format!("tax_form {f:?}: expected schedule_c, 1065, 1120s, or 1120")
            })?),
            (None, Some(c)) => Some(form_for_classification(c).ok_or_else(|| {
                format!(
                    "tax_classification {c:?}: expected partnership, S corporation, C corporation, or disregarded entity (or set tax_form)"
                )
            })?),
            (None, None) => None,
        };
    if !company.members.is_empty() && raw.books.close_into.is_some() {
        return Err(
            "close_into is for single-owner businesses; with [[company.members]] a close goes to each member's capital account"
                .to_string(),
        );
    }
    company.close_into = match raw.books.close_into {
        Some(c) => Some(codes("close_into", vec![c])?.remove(0)),
        None => None,
    };
    company.close_sweep = codes("close_sweep", raw.books.close_sweep)?;
    if let Some(basis) = raw.books.accounting_basis {
        company.basis = match basis.trim().to_lowercase().as_str() {
            "accrual" => "Accrual".to_string(),
            "cash" => "Cash".to_string(),
            other => {
                return Err(format!(
                    "accounting_basis {other:?}: expected accrual or cash"
                ));
            }
        };
    }
    let receipt_threshold = match raw.books.receipt_threshold {
        Some(t) => {
            let v = decimal(t.trim())
                .ok_or_else(|| format!("receipt_threshold {t:?} is not a decimal"))?;
            if v.is_sign_negative() && !v.is_zero() {
                return Err(format!("receipt_threshold {t:?} is below zero"));
            }
            Some(t.trim().to_string())
        }
        None => None,
    };
    let table: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
    if let Some(toml::Value::Table(c)) = table.get("company") {
        company.profile = c
            .iter()
            .filter_map(|(k, v)| {
                let s = match v {
                    toml::Value::String(s) => s.clone(),
                    toml::Value::Integer(i) => i.to_string(),
                    toml::Value::Float(f) => f.to_string(),
                    toml::Value::Boolean(b) => b.to_string(),
                    toml::Value::Datetime(d) => d.to_string(),
                    _ => return None,
                };
                Some((k.clone(), s))
            })
            .collect();
    }
    Ok(BooksSettings {
        company,
        receipt_threshold,
        cash_accounts: codes("cash_accounts", raw.books.cash_accounts)?,
        financing_accounts: codes("financing_accounts", raw.books.financing_accounts)?,
        investing_accounts: codes("investing_accounts", raw.books.investing_accounts)?,
        vault_key_file: match raw.vault.key_file {
            Some(p) if p.trim().is_empty() => {
                return Err("[vault] key_file is empty".into());
            }
            Some(p) => Some(std::path::PathBuf::from(match p.strip_prefix("~/") {
                Some(rest) => format!("{}/{rest}", std::env::var("HOME").unwrap_or_default()),
                None => p,
            })),
            None => None,
        },
        asset_tag_prefix: match raw.books.asset_tag_prefix {
            Some(p)
                if !p.is_empty()
                    && p.len() <= 8
                    && p.chars().all(|c| c.is_ascii_alphanumeric()) =>
            {
                Some(p)
            }
            Some(p) => {
                return Err(format!(
                    "[books] asset_tag_prefix {p:?}: 1 to 8 letters or digits"
                ));
            }
            None => None,
        },
        parties: match raw.books.parties.as_deref() {
            None | Some("suggest") => PartyPolicy::Suggest,
            Some("off") => PartyPolicy::Off,
            Some("require") | Some("required") => PartyPolicy::Require,
            Some(other) => {
                return Err(format!(
                    "[books] parties {other:?}: \"off\", \"suggest\", or \"require\""
                ));
            }
        },
    })
}

/// `[books] parties`: whether entries name their counterparty
/// (docs/parties-and-attachments.md "Counterparties"; DECISION, Stephen,
/// 2026-10-15: the default suggests, books may require or turn it off).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PartyPolicy {
    /// Never mention counterparties.
    Off,
    /// A draft or preview without a party names the likely one; nothing
    /// is refused.
    #[default]
    Suggest,
    /// Posting is refused until the entry names a party or is marked as
    /// having none.
    Require,
}

impl PartyPolicy {
    /// Its wire name.
    pub fn key(self) -> &'static str {
        match self {
            PartyPolicy::Off => "off",
            PartyPolicy::Suggest => "suggest",
            PartyPolicy::Require => "require",
        }
    }
}

/// Load `wanderbooks.toml` from a path.
pub fn load(path: &Path) -> Result<BooksSettings, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// files_jointly_with: names another member, on both sides.
    #[test]
    fn files_jointly_with() {
        let m = |a: &str, b: &str| {
            format!(
                "[company]\nname = \"X\"\n[[company.members]]\nname = \"A\"\nshare = \"50%\"\ncapital_account = \"3000\"\n{a}\n[[company.members]]\nname = \"B\"\nshare = \"50%\"\ncapital_account = \"3001\"\n{b}\n"
            )
        };
        let ok = parse(&m(
            "files_jointly_with = \"B\"",
            "files_jointly_with = \"A\"",
        ))
        .unwrap();
        assert_eq!(
            ok.company.members[0].files_jointly_with.as_deref(),
            Some("B")
        );
        assert!(
            parse(&m("files_jointly_with = \"B\"", ""))
                .unwrap_err()
                .contains("set it on both")
        );
        assert!(
            parse(&m("files_jointly_with = \"C\"", ""))
                .unwrap_err()
                .contains("isn't a member")
        );
        assert!(
            parse(&m("files_jointly_with = \"A\"", ""))
                .unwrap_err()
                .contains("themself")
        );
        assert_eq!(
            parse(&m("", "")).unwrap().company.members[1].files_jointly_with,
            None
        );
    }

    /// `[books] parties`: suggest by default; off, suggest, require (and
    /// "required") parse; anything else is refused, naming the choices.
    #[test]
    fn parties_policy() {
        let base = "[company]\nname = \"Acme LLC\"\n[books]\n";
        assert_eq!(parse(base).unwrap().parties, PartyPolicy::Suggest);
        for (v, want) in [
            ("off", PartyPolicy::Off),
            ("suggest", PartyPolicy::Suggest),
            ("require", PartyPolicy::Require),
            ("required", PartyPolicy::Require),
        ] {
            let s = parse(&format!("{base}parties = \"{v}\"\n")).unwrap();
            assert_eq!(s.parties, want, "{v}");
        }
        let e = parse(&format!("{base}parties = \"always\"\n")).unwrap_err();
        assert!(e.contains("require"), "{e}");
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn calendar_year_by_default() {
        let c = parse("").unwrap().company;
        assert_eq!(c.fiscal_year_start, (1, 1));
        assert_eq!(
            c.fiscal_year_containing(d(2026, 10, 11)),
            (d(2026, 1, 1), d(2026, 12, 31))
        );
        assert_eq!(
            c.fiscal_year_containing(d(2026, 1, 1)),
            (d(2026, 1, 1), d(2026, 12, 31))
        );
    }

    #[test]
    fn a_july_fiscal_year_spans_two_calendar_years() {
        let c = parse("[company]\nfiscal_year_start = \"07-01\"\n")
            .unwrap()
            .company;
        assert_eq!(
            c.fiscal_year_containing(d(2026, 3, 15)),
            (d(2025, 7, 1), d(2026, 6, 30))
        );
        assert_eq!(
            c.fiscal_year_containing(d(2026, 7, 1)),
            (d(2026, 7, 1), d(2027, 6, 30))
        );
        assert_eq!(
            c.fiscal_year_containing(d(2026, 6, 30)),
            (d(2025, 7, 1), d(2026, 6, 30))
        );
    }

    #[test]
    fn reads_the_scaffolded_file() {
        let s = parse(
            "[company]\nname = \"Wanderland Labs LLC\"\nentity_type = \"x\"\nfiscal_year_start = \"01-01\"\n\n[books]\ndata_dir = \"books-data\"\nreceipt_threshold = \"25.00\"\n",
        )
        .unwrap();
        assert_eq!(s.company.name, "Wanderland Labs LLC");
        assert_eq!(s.receipt_threshold.as_deref(), Some("25.00"));
        // An empty name (fresh scaffold) keeps the placeholder.
        assert_eq!(
            parse("[company]\nname = \"\"\n").unwrap().company.name,
            "Unnamed company"
        );
    }

    #[test]
    fn keeps_the_whole_profile_in_order() {
        let s = parse(
            "[company]\nname = \"Acme LLC\"\nein = \"12-3456789\"\nformed = \"2026-10-04\"\n[books]\ncash_accounts = [\"1000\", \"1010\"]\nfinancing_accounts = [\"2100\"]\ninvesting_accounts = [\"1400\"]\n",
        )
        .unwrap();
        assert_eq!(
            s.company.profile,
            [
                ("name".to_string(), "Acme LLC".to_string()),
                ("ein".to_string(), "12-3456789".to_string()),
                ("formed".to_string(), "2026-10-04".to_string()),
            ]
        );
        assert_eq!(s.cash_accounts, ["1000", "1010"]);
        assert_eq!(s.financing_accounts, ["2100"]);
        assert_eq!(s.investing_accounts, ["1400"]);
    }

    #[test]
    fn reads_members_and_basis() {
        let s = parse(
            "[company]\n[[company.members]]\nname = \"Stephen Waits\"\nshare = \"50%\"\ncapital_account = \"3000\"\n[[company.members]]\nname = \"Katherine Pendergast\"\nshare = \"0.5\"\ncapital_account = \"3001\"\n[books]\naccounting_basis = \"cash\"\n",
        )
        .unwrap();
        let half = Decimal::new(5, 1);
        assert_eq!(
            s.company.members,
            vec![
                Member {
                    name: "Stephen Waits".into(),
                    share: half,
                    capital_account: "3000".into(),
                    distribution_account: None,
                    files_jointly_with: None,
                },
                Member {
                    name: "Katherine Pendergast".into(),
                    share: half,
                    capital_account: "3001".into(),
                    distribution_account: None,
                    files_jointly_with: None,
                },
            ]
        );
        assert_eq!(s.company.basis, "Cash");
        assert_eq!(parse("").unwrap().company.basis, "Accrual");
        // Shares must add up; every member needs a capital account.
        let two = |a: &str, b: &str| {
            format!(
                "[company]\n[[company.members]]\nname = \"A\"\nshare = \"{a}\"\ncapital_account = \"3000\"\n[[company.members]]\nname = \"B\"\nshare = \"{b}\"\ncapital_account = \"3001\"\n"
            )
        };
        assert!(parse(&two("50%", "40%")).is_err());
        assert!(parse(&two("60%", "40%")).is_ok());
        assert!(parse("[company]\n[[company.members]]\nname = \"A\"\nshare = \"100%\"\n").is_err());
        assert!(parse("[books]\naccounting_basis = \"modified\"\n").is_err());
    }

    #[test]
    fn tax_form_from_setting_or_classification() {
        use ledger_core::TaxForm;
        let form = |toml: &str| parse(toml).unwrap().company.tax_form;
        assert_eq!(
            form("[company]\ntax_form = \"1120s\"\n"),
            Some(TaxForm::Form1120S)
        );
        assert_eq!(
            form("[company]\ntax_classification = \"partnership\"\n"),
            Some(TaxForm::Form1065)
        );
        assert_eq!(
            form("[company]\ntax_classification = \"S corporation\"\n"),
            Some(TaxForm::Form1120S)
        );
        assert_eq!(
            form("[company]\ntax_classification = \"disregarded entity\"\n"),
            Some(TaxForm::ScheduleC)
        );
        assert_eq!(
            form("[company]\ntax_classification = \"partnership\"\ntax_form = \"1120\"\n"),
            Some(TaxForm::Form1120)
        );
        assert_eq!(form(""), None);
        assert!(parse("[company]\ntax_form = \"990\"\n").is_err());
    }

    #[test]
    fn rejects_bad_settings() {
        assert!(parse("[company]\nfiscal_year_start = \"02-29\"\n").is_err());
        assert!(parse("[company]\nfiscal_year_start = \"13-01\"\n").is_err());
        assert!(parse("[company]\nfiscal_year_start = \"Jan 1\"\n").is_err());
        assert!(parse("[books]\nreceipt_threshold = \"25 dollars\"\n").is_err());
    }
}
