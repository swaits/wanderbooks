//! Settings journaled in the books (plan/served.md 5.3).
//!
//! A company's settings (its name, fiscal year, members, receipt rule,
//! cash flow roles, ...) are facts about the books, so the books keep
//! them: each change is a `settings.set` journal line, replayed into
//! [`Settings`] like every other fact. A settings file is only text to
//! import from or export to (`ledger-config`).
//!
//! ```text
//! GOAL: one record of how the books are kept, with who decided each
//!       setting and when, that every reader (server, viewer, replay)
//!       sees the same way.
//! SOURCE: plan/served.md I-3 (the journal holds all books state) and 5.3
//!         (setting keys); the validators carried over from
//!         ledger-config's wanderbooks.toml parser, whose SOURCE lines
//!         they keep (IRS Pub. 538, Treas. Reg. §301.7701-3, IRC §704(a)).
//! INVARIANTS:
//!   1. Only keys in KEYS exist; a value is checked by its key's kind
//!      before it is journaled ([`check`]), and stored in one canonical
//!      spelling, so two spellings of one value are one value.
//!   2. A change is all or nothing: the settings after every change in it
//!      must hold together ([`Settings::check_whole`]), or none is made.
//!   3. Replay takes any value of the right shape ([`shape`]): a check a
//!      later version tightens never makes older books unreadable
//!      (rules.md B-28).
//!   4. A setting never holds a full TIN, except the company's own EIN
//!      (`company.ein`, decided 2026-10-16, D-5): nine digits standing
//!      alone, bare or printed the way an SSN or EIN is (3-2-4 or 2-7,
//!      joined by a dash or a space), are refused everywhere else, the
//!      D-U-N-S number's own key aside (nine digits that are no TIN).
//!   5. Unset keys mean the defaults the settings file had: "Unnamed
//!      company", a calendar year, accrual basis, receipts above 25.00,
//!      parties suggested, approval rules off, the computer's time zone.
//! ```

use crate::TaxForm;
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Who decided a change, and on what authority: kept with every setting,
/// convention, decision and open item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    /// Who decided: a member's or employee's name, "onboarding", ...
    pub decided_by: String,
    /// The day they decided, `YYYY-MM-DD`.
    pub decided_on: NaiveDate,
    /// The rule or document it rests on, if any ("IRS Pub. 538").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation: Option<String>,
    /// Anything else worth keeping with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Longest name or other one-line text a setting or source takes.
pub const MAX_TEXT: usize = 200;
/// Longest note, citation, address or other long text.
pub const MAX_LONG_TEXT: usize = 2_000;

impl Source {
    /// Check a source before it is journaled: who decided is named, the
    /// day is in the calendar the books keep, text is bounded and holds no
    /// full TIN (INVARIANT 4).
    pub fn check(&self) -> Result<(), String> {
        line_text("decided_by", &self.decided_by, MAX_TEXT)?;
        crate::calendar::check(self.decided_on).map_err(|e| e.to_string())?;
        if let Some(c) = &self.citation {
            long_text("citation", c, MAX_LONG_TEXT)?;
        }
        if let Some(n) = &self.note {
            long_text("note", n, MAX_LONG_TEXT)?;
        }
        Ok(())
    }
}

/// How visible a setting's value is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sensitivity {
    /// Shown as it is.
    Public,
    /// An identity number (the company's EIN): shown in full only in
    /// reports and packages, masked to its last four elsewhere.
    Identity,
}

/// What a setting holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// One line of text, 1 to [`MAX_TEXT`] characters.
    Text,
    /// Longer text (an address): 1 to [`MAX_LONG_TEXT`] characters.
    LongText,
    /// One of a fixed set of words; each canonical word may have aliases.
    Choice(&'static [(&'static str, &'static [&'static str])]),
    /// The first day of a month, `"MM-01"`.
    MonthStart,
    /// A day, `YYYY-MM-DD`, in the calendar the books keep.
    Date,
    /// A decimal amount at least zero.
    Amount,
    /// One account code.
    Code,
    /// A list of account codes.
    Codes,
    /// The members of a multi-member LLC.
    Members,
    /// An IANA time zone name ("America/Denver").
    TimeZone,
    /// The income tax return, as `TaxForm::parse` reads it, kept as
    /// `schedule_c`, `1065`, `1120s` or `1120`.
    TaxForm,
    /// An EIN, `NN-NNNNNNN`.
    Ein,
    /// A D-U-N-S number: nine digits.
    Duns,
    /// The IRS name control: one to four letters, digits, `&` or `-`.
    NameControl,
    /// An asset tag prefix: one to eight letters or digits.
    TagPrefix,
    /// A repository's address, as git takes it: one line, never one that
    /// reads as an option.
    Repository,
    /// A folder named from the root of its disk: one line, an absolute
    /// path.
    Folder,
}

/// One setting the books know.
#[derive(Debug)]
pub struct Key {
    /// Its name, e.g. `company.name`.
    pub name: &'static str,
    /// Where a settings file keeps it: `(table, key)`, e.g.
    /// `("company", "name")`.
    pub toml: (&'static str, &'static str),
    /// What it holds.
    pub kind: Kind,
    /// How visible its value is.
    pub sensitivity: Sensitivity,
    /// What it means, for `books.settings`.
    pub doc: &'static str,
}

const ACCRUAL_CASH: &[(&str, &[&str])] = &[("accrual", &[]), ("cash", &[])];
const PARTY_POLICY: &[(&str, &[&str])] =
    &[("off", &[]), ("suggest", &[]), ("require", &["required"])];
const APPROVAL_MODE: &[(&str, &[&str])] = &[("off", &[]), ("enforce", &[])];

/// A return's key as a setting keeps it.
pub fn tax_form_key(f: TaxForm) -> &'static str {
    match f {
        TaxForm::ScheduleC => "schedule_c",
        TaxForm::Form1065 => "1065",
        TaxForm::Form1120S => "1120s",
        TaxForm::Form1120 => "1120",
    }
}

/// Every setting the books know, in the order they are shown.
pub static KEYS: &[Key] = &[
    Key {
        name: "company.name",
        toml: ("company", "name"),
        kind: Kind::Text,
        sensitivity: Sensitivity::Public,
        doc: "Legal name, for report headers. Unset: \"Unnamed company\".",
    },
    Key {
        name: "company.entity_type",
        toml: ("company", "entity_type"),
        kind: Kind::Text,
        sensitivity: Sensitivity::Public,
        doc: "The kind of entity, as the formation papers say (\"LLC, multi-member, member-managed\").",
    },
    Key {
        name: "company.tax_classification",
        toml: ("company", "tax_classification"),
        kind: Kind::Text,
        sensitivity: Sensitivity::Public,
        doc: "Federal tax classification: partnership, S corporation, C corporation, or disregarded entity. Names the return unless company.tax_form is set.",
    },
    Key {
        name: "company.tax_form",
        toml: ("company", "tax_form"),
        kind: Kind::TaxForm,
        sensitivity: Sensitivity::Public,
        doc: "The income tax return: schedule_c, 1065, 1120s, or 1120. Wins over company.tax_classification.",
    },
    Key {
        name: "company.fiscal_year_start",
        toml: ("company", "fiscal_year_start"),
        kind: Kind::MonthStart,
        sensitivity: Sensitivity::Public,
        doc: "First day of the fiscal year, \"MM-01\" (a fiscal year ends on the last day of a month: IRS Pub. 538). Unset: \"01-01\".",
    },
    Key {
        name: "company.timezone",
        toml: ("company", "timezone"),
        kind: Kind::TimeZone,
        sensitivity: Sensitivity::Public,
        doc: "The company's time zone (IANA, e.g. \"America/Denver\"): what \"today\" means for the books. Unset: the computer's.",
    },
    Key {
        name: "company.formation_state",
        toml: ("company", "formation_state"),
        kind: Kind::Text,
        sensitivity: Sensitivity::Public,
        doc: "State (or country) of formation.",
    },
    Key {
        name: "company.formed",
        toml: ("company", "formed"),
        kind: Kind::Date,
        sensitivity: Sensitivity::Public,
        doc: "Date of formation, YYYY-MM-DD.",
    },
    Key {
        name: "company.state_entity_number",
        toml: ("company", "state_entity_number"),
        kind: Kind::Text,
        sensitivity: Sensitivity::Public,
        doc: "The formation state's entity number.",
    },
    Key {
        name: "company.ein",
        toml: ("company", "ein"),
        kind: Kind::Ein,
        sensitivity: Sensitivity::Identity,
        doc: "The company's EIN, NN-NNNNNNN: in full only in reports and packages, masked elsewhere.",
    },
    Key {
        name: "company.irs_name_control",
        toml: ("company", "irs_name_control"),
        kind: Kind::NameControl,
        sensitivity: Sensitivity::Public,
        doc: "The IRS name control (up to four characters, from the EIN letter): an e-filed return must match it.",
    },
    Key {
        name: "company.duns",
        toml: ("company", "duns"),
        kind: Kind::Duns,
        sensitivity: Sensitivity::Public,
        doc: "D-U-N-S number, nine digits.",
    },
    Key {
        name: "company.address",
        toml: ("company", "address"),
        kind: Kind::LongText,
        sensitivity: Sensitivity::Public,
        doc: "Principal business address.",
    },
    Key {
        name: "company.registered_agent",
        toml: ("company", "registered_agent"),
        kind: Kind::Text,
        sensitivity: Sensitivity::Public,
        doc: "Registered agent.",
    },
    Key {
        name: "company.partnership_representative",
        toml: ("company", "partnership_representative"),
        kind: Kind::Text,
        sensitivity: Sensitivity::Public,
        doc: "Partnership representative (BBA, IRC §6223), for partnerships.",
    },
    Key {
        name: "company.cpa",
        toml: ("company", "cpa"),
        kind: Kind::Text,
        sensitivity: Sensitivity::Public,
        doc: "The company's CPA.",
    },
    Key {
        name: "company.members",
        toml: ("company", "members"),
        kind: Kind::Members,
        sensitivity: Sensitivity::Public,
        doc: "Members of a multi-member LLC: [{name, share (\"50%\" or \"0.5\"), capital_account, distribution_account?, files_jointly_with?}]. Names and capital accounts distinct; shares sum to exactly 1 (IRC §704(a): shares come from the agreement).",
    },
    Key {
        name: "books.accounting_basis",
        toml: ("books", "accounting_basis"),
        kind: Kind::Choice(ACCRUAL_CASH),
        sensitivity: Sensitivity::Public,
        doc: "accrual or cash, for report footers. Unset: accrual.",
    },
    Key {
        name: "books.cash_accounts",
        toml: ("books", "cash_accounts"),
        kind: Kind::Codes,
        sensitivity: Sensitivity::Public,
        doc: "Asset account codes that hold cash (bank, savings, cash on hand): the cash flow statement explains their change.",
    },
    Key {
        name: "books.financing_accounts",
        toml: ("books", "financing_accounts"),
        kind: Kind::Codes,
        sensitivity: Sensitivity::Public,
        doc: "Liability account codes the cash flow statement treats as financing (loans, and what owners advanced: ASC 230-10-45-14(b)); equity is always financing. What a member or lender pays for the company through one is shown as borrowed and spent (D-42).",
    },
    Key {
        name: "books.investing_accounts",
        toml: ("books", "investing_accounts"),
        kind: Kind::Codes,
        sensitivity: Sensitivity::Public,
        doc: "Asset account codes the cash flow statement treats as investing (loans the company makes, investments it buys: ASC 230-10-20); registered fixed assets are investing unlisted.",
    },
    Key {
        name: "books.close_into",
        toml: ("books", "close_into"),
        kind: Kind::Code,
        sensitivity: Sensitivity::Public,
        doc: "The equity account a period close puts net income in (single-owner businesses). Unset: Retained Earnings, or the members' capital accounts.",
    },
    Key {
        name: "books.close_sweep",
        toml: ("books", "close_sweep"),
        kind: Kind::Codes,
        sensitivity: Sensitivity::Public,
        doc: "Equity accounts (draws, dividends, distributions) a close zeroes into books.close_into.",
    },
    Key {
        name: "books.asset_tag_prefix",
        toml: ("books", "asset_tag_prefix"),
        kind: Kind::TagPrefix,
        sensitivity: Sensitivity::Public,
        doc: "Prefix of generated asset tags (\"AC\" for AC-0001): one to eight letters or digits. Unset: the company name's initials.",
    },
    Key {
        name: "policy.evidence.threshold",
        toml: ("books", "receipt_threshold"),
        kind: Kind::Amount,
        sensitivity: Sensitivity::Public,
        doc: "An entry whose debits are strictly greater than this needs a receipt before it posts; \"0.00\": every entry. Unset: \"25.00\".",
    },
    Key {
        name: "policy.evidence.required_accounts",
        toml: ("books", "receipt_required_accounts"),
        kind: Kind::Codes,
        sensitivity: Sensitivity::Public,
        doc: "Account codes whose entries always need a receipt, whatever the amount.",
    },
    Key {
        name: "policy.evidence.exempt_accounts",
        toml: ("books", "receipt_exempt_accounts"),
        kind: Kind::Codes,
        sensitivity: Sensitivity::Public,
        doc: "Account codes exempt from the receipt rule: an entry is exempt only when every line is in one.",
    },
    Key {
        name: "policy.parties",
        toml: ("books", "parties"),
        kind: Kind::Choice(PARTY_POLICY),
        sensitivity: Sensitivity::Public,
        doc: "Whether entries name their counterparty: off, suggest (a draft without one names the likely one), or require (posting is refused until an entry names a party or is marked as having none). Unset: suggest.",
    },
    Key {
        name: "policy.approval_rules",
        toml: ("books", "approval_rules"),
        kind: Kind::Choice(APPROVAL_MODE),
        sensitivity: Sensitivity::Public,
        doc: "off: approval_rule conventions are recorded but never block a post. enforce: an entry a rule covers posts only once its approver approves it (books.approve_entry). Unset: off.",
    },
    Key {
        name: "backup.git_remote",
        toml: ("backup", "git_remote"),
        kind: Kind::Repository,
        sensitivity: Sensitivity::Public,
        doc: "A private repository the books back themselves up to (plan D-30): its address as its host shows it for cloning, over ssh or https (such as https://host/owner/books), or a path on this computer. Unset: none.",
    },
    Key {
        name: "backup.folder",
        toml: ("backup", "folder"),
        kind: Kind::Folder,
        sensitivity: Sensitivity::Public,
        doc: "A folder the books back themselves up to as well (D-30): an absolute path, an external disk or a synced folder; the backup is a repository in it named for the book. Unset: none.",
    },
];

/// The key named `name`.
pub fn key(name: &str) -> Option<&'static Key> {
    KEYS.iter().find(|k| k.name == name)
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
    pub tax_form: Option<TaxForm>,
    /// `books.close_into`: the equity account a period close puts net
    /// income in (a sole proprietor's capital, say). Unset: Retained
    /// Earnings, or members' capital accounts when there are members.
    pub close_into: Option<String>,
    /// `books.close_sweep`: equity accounts (draws, dividends,
    /// distributions) a close zeroes into `close_into`.
    pub close_sweep: Vec<String>,
    /// Every simple company fact (name, EIN, formation date, ...), as
    /// `(name, value)`: the full profile for display.
    pub profile: Vec<(String, String)>,
}

impl Default for Company {
    fn default() -> Self {
        Self {
            name: DEFAULT_NAME.to_string(),
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

/// The company name of books that never set one.
pub const DEFAULT_NAME: &str = "Unnamed company";

/// The receipt threshold of books that never set one.
pub const DEFAULT_THRESHOLD: &str = "25.00";

impl Company {
    /// The members as party shares see them ([`crate::PartyMember`]):
    /// each member whose name (ignoring case and surrounding space) is a
    /// party with the member role, and whose capital account is in the
    /// chart. The party not archived of that name when there is one, else
    /// the most recent archived one: archiving a member ("no longer dealt
    /// with; history stays") never takes their history away. Members
    /// missing either are left out, so their shares can't be derived (and
    /// the viewer shows no capital statement for them).
    pub fn party_members(&self, ledger: &crate::Ledger) -> Vec<crate::PartyMember> {
        let is_member = |p: &&crate::Party| p.roles.contains(&crate::Role::Member);
        self.members
            .iter()
            .filter_map(|m| {
                let key = crate::parties::name_key(&m.name);
                let party = match ledger.party_by_name(&m.name) {
                    Some(p) => Some(p).filter(is_member)?,
                    None => ledger
                        .parties()
                        .filter(|p| p.archived && crate::parties::name_key(&p.name) == key)
                        .filter(is_member)
                        .max_by_key(|p| p.id)?,
                };
                Some(crate::PartyMember {
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

/// `policy.parties`: whether entries name their counterparty
/// (docs/parties-and-attachments.md "Counterparties"; DECISION, owner,
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

    /// From its wire name ("required" too).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(PartyPolicy::Off),
            "suggest" => Some(PartyPolicy::Suggest),
            "require" | "required" => Some(PartyPolicy::Require),
            _ => None,
        }
    }
}

/// `policy.approval_rules`: whether approval rules block posting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ApprovalMode {
    /// Rules are recorded, never enforced (the default: light governance,
    /// decided 2026-10-16).
    #[default]
    Off,
    /// An entry a rule covers posts only once approved.
    Enforce,
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

/// Whether `text` holds a number that reads as a full TIN (INVARIANT 4):
/// nine digits standing alone (no digit right before or after), bare or
/// printed the way an SSN (3-2-4) or an EIN (2-7) is, the groups joined by
/// a hyphen or any other dash, a dot, or a space (or a dash with spaces
/// around it); full-width digits count as digits.
pub fn holds_printed_tin(text: &str) -> bool {
    // Full-width digits read as the digits they are.
    let c: Vec<char> = text
        .chars()
        .map(|ch| match ch {
            '\u{ff10}'..='\u{ff19}' => char::from_digit(ch as u32 - 0xff10, 10).unwrap_or(ch),
            _ => ch,
        })
        .collect();
    let digit = |i: usize| c.get(i).is_some_and(char::is_ascii_digit);
    let dash = |ch: char| matches!(ch, '-' | '\u{2010}'..='\u{2015}' | '\u{2212}' | '.');
    // A joiner at `i`: a dash, dot or space, or a dash with spaces around
    // it; returns where the next group starts.
    let joiner = |i: usize| -> Option<usize> {
        let mut j = i;
        while c.get(j) == Some(&' ') {
            j += 1;
        }
        let mut dashes = 0;
        while c.get(j).copied().is_some_and(dash) {
            j += 1;
            dashes += 1;
        }
        while c.get(j) == Some(&' ') {
            j += 1;
        }
        let spaces = j - i - dashes;
        (j > i && dashes <= 1 && spaces <= 2).then_some(j)
    };
    let groups_at = |at: usize, groups: &[usize]| -> Option<usize> {
        let mut i = at;
        for (n, g) in groups.iter().enumerate() {
            if n > 0 {
                i = joiner(i)?;
            }
            for _ in 0..*g {
                if !digit(i) {
                    return None;
                }
                i += 1;
            }
        }
        Some(i)
    };
    (0..c.len()).any(|at| {
        (at == 0 || !digit(at - 1))
            && [&[9][..], &[3, 2, 4][..], &[2, 7][..]]
                .iter()
                .any(|g| groups_at(at, g).is_some_and(|end| !digit(end)))
    })
}

fn no_tin(what: &str, s: &str) -> Result<(), String> {
    if holds_printed_tin(s) {
        return Err(format!(
            "{what} holds a number that reads as a full SSN or EIN (nine digits, bare or printed 3-2-4 or 2-7): a full TIN goes only into the vault (books.set_tin), the company's EIN into company.ein and its D-U-N-S number into company.duns; for any other nine-digit number, write its last four"
        ));
    }
    Ok(())
}

/// One line of text: trimmed non-empty, at most `max` characters, no line
/// breaks or other control characters, no full TIN.
pub fn line_text(what: &str, s: &str, max: usize) -> Result<String, String> {
    let t = s.trim();
    if t.is_empty() {
        return Err(format!("{what} is blank"));
    }
    if t.chars().count() > max {
        return Err(format!("{what} is longer than {max} characters"));
    }
    if t.chars().any(char::is_control) {
        return Err(format!(
            "{what} holds a line break or other control character"
        ));
    }
    no_tin(what, t)?;
    Ok(t.to_string())
}

/// Longer text: trimmed non-empty, at most `max` characters, line breaks
/// and tabs allowed but no other control characters, no full TIN.
pub fn long_text(what: &str, s: &str, max: usize) -> Result<String, String> {
    let t = s.trim();
    if t.is_empty() {
        return Err(format!("{what} is blank"));
    }
    if t.chars().count() > max {
        return Err(format!("{what} is longer than {max} characters"));
    }
    if t.chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
    {
        return Err(format!("{what} holds a control character"));
    }
    no_tin(what, t)?;
    Ok(t.to_string())
}

/// Parse a share: "50%" or "0.5", in (0, 1].
pub fn parse_share(name: &str, s: &str) -> Result<Decimal, String> {
    let s = s.trim();
    let bad = || format!("member {name}: share {s:?} must be like \"50%\" or \"0.5\"");
    let share = match s.strip_suffix('%') {
        Some(pct) => decimal(pct.trim())
            .ok_or_else(bad)?
            .checked_div(Decimal::from(100))
            .ok_or_else(bad)?,
        None => decimal(s).ok_or_else(bad)?,
    };
    if share <= Decimal::ZERO || share > Decimal::ONE {
        return Err(bad());
    }
    Ok(share)
}

/// Parse `"MM-DD"`: the first day of a month (a fiscal year ends on the
/// last day of a month: IRS Pub. 538, docs/spec/sources/
/// irs-periods-and-methods.md).
pub fn parse_month_day(s: &str) -> Result<(u32, u32), String> {
    let bad = || format!("fiscal_year_start {s:?}: expected \"MM-DD\", e.g. \"01-01\"");
    let (m, d) = s.split_once('-').ok_or_else(bad)?;
    let all_digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
    if !all_digits(m) || !all_digits(d) {
        return Err(bad());
    }
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

/// The return a federal tax classification files (docs/spec/sources/
/// irs-entity-classification.md), read by whole words, ignoring case:
/// "partnership" → 1065; "S corporation" (S corp, S-corp, scorp) →
/// 1120-S; "C corporation", or "corporation", "corp", or "association"
/// alone → 1120; "disregarded", "sole proprietor(ship)", or
/// "single-member" → Schedule C. Anything else, or words naming two
/// returns ("corporation or S corporation"), names none.
pub fn form_for_classification(c: &str) -> Option<TaxForm> {
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

/// An account code as written, trimmed; a blank one is refused.
fn code(what: &str, s: &str) -> Result<String, String> {
    let c = s.trim();
    if c.is_empty() {
        return Err(format!("{what}: an account code is blank"));
    }
    line_text(what, c, 64)
}

fn want_str<'a>(name: &str, v: &'a Value) -> Result<&'a str, String> {
    v.as_str()
        .ok_or_else(|| format!("{name} takes a string, not {v}"))
}

fn want_list<'a>(name: &str, v: &'a Value) -> Result<&'a Vec<Value>, String> {
    v.as_array()
        .ok_or_else(|| format!("{name} takes a list, not {v}"))
}

/// Check a value for `key` before it is journaled, and return it in its
/// canonical spelling (INVARIANT 1). `null` is never a value: clearing a
/// key is its own change.
pub fn check(key: &Key, v: &Value) -> Result<Value, String> {
    let name = key.name;
    match key.kind {
        Kind::Text => Ok(json!(line_text(name, want_str(name, v)?, MAX_TEXT)?)),
        Kind::LongText => Ok(json!(long_text(name, want_str(name, v)?, MAX_LONG_TEXT)?)),
        Kind::Choice(choices) => {
            let s = want_str(name, v)?.trim().to_lowercase();
            choices
                .iter()
                .find(|(canon, aliases)| *canon == s || aliases.contains(&s.as_str()))
                .map(|(canon, _)| json!(canon))
                .ok_or_else(|| {
                    let names: Vec<&str> = choices.iter().map(|(c, _)| *c).collect();
                    format!("{name} {s:?}: expected one of {}", names.join(", "))
                })
        }
        Kind::MonthStart => {
            let (m, d) = parse_month_day(want_str(name, v)?.trim())?;
            Ok(json!(format!("{m:02}-{d:02}")))
        }
        Kind::Date => {
            let s = want_str(name, v)?.trim();
            let d = NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .ok()
                .filter(|d| d.format("%Y-%m-%d").to_string() == s)
                .ok_or_else(|| format!("{name} {s:?}: expected a date, YYYY-MM-DD"))?;
            crate::calendar::check(d).map_err(|e| format!("{name}: {e}"))?;
            Ok(json!(s))
        }
        Kind::Amount => {
            let s = want_str(name, v)?.trim();
            let a = decimal(s).ok_or_else(|| format!("{name} {s:?} is not a decimal"))?;
            if a.is_sign_negative() && !a.is_zero() {
                return Err(format!("{name} {s:?} is below zero"));
            }
            if a > crate::kernel::MAX_LINE_AMOUNT {
                return Err(format!("{name} {s:?} is out of range"));
            }
            // Kept to its places ("0.00" stays "0.00": the rule quotes
            // it), without leading zeros or a sign ("025.00" is "25.00").
            Ok(json!(a.abs().to_string()))
        }
        Kind::Code => Ok(json!(code(name, want_str(name, v)?)?)),
        Kind::Codes => {
            let mut out = Vec::new();
            for c in want_list(name, v)? {
                let c = code(name, want_str(name, c)?)?;
                if out.contains(&c) {
                    return Err(format!("{name}: account {c} is listed twice"));
                }
                out.push(c);
            }
            Ok(json!(out))
        }
        Kind::Members => Ok(members_json(&check_members(want_list(name, v)?)?)),
        Kind::TimeZone => {
            let s = want_str(name, v)?.trim();
            let zone = jiff::tz::TimeZone::get(s).map_err(|_| {
                format!("{name} {s:?} is not a time zone (an IANA name, like \"America/Denver\")")
            })?;
            // The database's own spelling ("america/denver" is
            // "America/Denver"): one zone, one value.
            let canonical = zone.iana_name().unwrap_or(s);
            line_text(name, canonical, 64).map(|s| json!(s))
        }
        Kind::Ein => {
            let s = want_str(name, v)?.trim();
            let digits: String = s.chars().filter(char::is_ascii_digit).collect();
            let shape_ok = s.chars().all(|c| c.is_ascii_digit() || c == '-')
                && digits.len() == 9
                && (s.len() == 9 || (s.len() == 10 && s.as_bytes()[2] == b'-'));
            if !shape_ok {
                return Err(format!("{name}: expected an EIN, NN-NNNNNNN"));
            }
            Ok(json!(format!("{}-{}", &digits[..2], &digits[2..])))
        }
        Kind::Duns => {
            // Nine digits, bare or printed as D-U-N-S numbers are (2-3-4);
            // never in an SSN's or EIN's shape (INVARIANT 4).
            let s = want_str(name, v)?.trim();
            let printed = s.len() == 11
                && s.as_bytes()[2] == b'-'
                && s.as_bytes()[6] == b'-'
                && s.chars().filter(char::is_ascii_digit).count() == 9;
            let bare = s.len() == 9 && s.chars().all(|c| c.is_ascii_digit());
            if !(bare || printed) {
                return Err(format!(
                    "{name}: expected a D-U-N-S number, nine digits (or NN-NNN-NNNN)"
                ));
            }
            Ok(json!(s.replace('-', "")))
        }
        Kind::TaxForm => {
            let s = want_str(name, v)?.trim();
            TaxForm::parse(s)
                .map(|f| json!(tax_form_key(f)))
                .ok_or_else(|| format!("{name} {s:?}: expected schedule_c, 1065, 1120s, or 1120"))
        }
        Kind::NameControl => {
            let s = want_str(name, v)?.trim().to_uppercase();
            if s.is_empty()
                || s.len() > 4
                || !s
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '&' || c == '-')
            {
                return Err(format!("{name} {s:?}: one to four letters, digits, & or -"));
            }
            Ok(json!(s))
        }
        Kind::TagPrefix => {
            let s = want_str(name, v)?;
            if s.is_empty() || s.len() > 8 || !s.chars().all(|c| c.is_ascii_alphanumeric()) {
                return Err(format!("{name} {s:?}: 1 to 8 letters or digits"));
            }
            Ok(json!(s))
        }
        Kind::Repository => {
            let s = line_text(name, want_str(name, v)?, MAX_TEXT)?;
            if s.starts_with('-') {
                return Err(format!(
                    "{name} {s:?}: a repository's address never starts with -"
                ));
            }
            // A path on this computer is named from the root of its disk:
            // the server's working folder is no place to name one from.
            let networked = match s.split_once("://") {
                Some((scheme, _)) => scheme != "file",
                None => s
                    .split_once(':')
                    .is_some_and(|(host, _)| !host.contains('/')),
            };
            if !networked && !s.starts_with("file://") && !std::path::Path::new(&s).is_absolute() {
                return Err(format!(
                    "{name} {s:?}: a repository on this computer is named from the root of its disk (such as /srv/books)"
                ));
            }
            Ok(json!(s))
        }
        Kind::Folder => {
            let s = line_text(name, want_str(name, v)?, MAX_TEXT)?;
            if !std::path::Path::new(&s).is_absolute() {
                return Err(format!(
                    "{name} {s:?}: name the folder from the root of its disk (such as /media/backup)"
                ));
            }
            Ok(json!(s))
        }
    }
}

/// Whether a journaled value has its key's shape (INVARIANT 3): the JSON
/// type, and for members each one's fields. Replay asks no more.
pub fn shape(key: &Key, v: &Value) -> Result<(), String> {
    let name = key.name;
    match key.kind {
        Kind::Codes => {
            for c in want_list(name, v)? {
                want_str(name, c)?;
            }
            Ok(())
        }
        Kind::Members => {
            // Shares in (0, 1], as every version wrote them: a share past
            // that could overflow the figures that multiply by it.
            for m in members_from_json(v)? {
                if m.share <= Decimal::ZERO || m.share > Decimal::ONE {
                    return Err(format!(
                        "company.members: {}'s share {} is outside (0, 1]",
                        m.name, m.share
                    ));
                }
            }
            Ok(())
        }
        Kind::Amount => {
            let s = want_str(name, v)?;
            decimal(s)
                .map(|_| ())
                .ok_or_else(|| format!("{name} {s:?} is not a decimal"))
        }
        Kind::MonthStart => parse_month_start_lenient(want_str(name, v)?).map(|_| ()),
        _ => want_str(name, v).map(|_| ()),
    }
}

/// A journaled fiscal year start, "MM-DD", any month and day that exist in
/// every year (replay: the first of a month is checked for new changes).
fn parse_month_start_lenient(s: &str) -> Result<(u32, u32), String> {
    let bad = || format!("fiscal_year_start {s:?}: expected \"MM-DD\"");
    let (m, d) = s.split_once('-').ok_or_else(bad)?;
    let (m, d): (u32, u32) = (m.parse().map_err(|_| bad())?, d.parse().map_err(|_| bad())?);
    // A day that exists in every year, so fiscal_year_containing can't fail.
    NaiveDate::from_ymd_opt(2001, m, d).ok_or_else(bad)?;
    Ok((m, d))
}

fn check_members(list: &[Value]) -> Result<Vec<Member>, String> {
    let mut members = Vec::new();
    for (i, m) in list.iter().enumerate() {
        let o = m.as_object().ok_or_else(|| {
            format!(
                "member {}: expected {{name, share, capital_account}}",
                i + 1
            )
        })?;
        for k in o.keys() {
            if ![
                "name",
                "share",
                "capital_account",
                "distribution_account",
                "files_jointly_with",
            ]
            .contains(&k.as_str())
            {
                return Err(format!("member {}: unknown field {k:?}", i + 1));
            }
        }
        let field = |f: &str| -> Result<Option<String>, String> {
            match o.get(f) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
                Some(Value::String(s)) => {
                    line_text(&format!("member {} {f}", i + 1), s, MAX_TEXT).map(Some)
                }
                Some(other) => Err(format!(
                    "member {} {f}: expected a string, not {other}",
                    i + 1
                )),
            }
        };
        let name = field("name")?.ok_or_else(|| format!("member {}: name is blank", i + 1))?;
        let share = parse_share(
            &name,
            field("share")?
                .ok_or_else(|| format!("member {name}: share is required"))?
                .as_str(),
        )?;
        let capital_account = field("capital_account")?
            .ok_or_else(|| format!("member {name}: capital_account is required"))?;
        members.push(Member {
            name,
            share,
            capital_account,
            distribution_account: field("distribution_account")?,
            files_jointly_with: field("files_jointly_with")?,
        });
    }
    for (i, m) in members.iter().enumerate() {
        for other in &members[..i] {
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
        for other in &members {
            if other.distribution_account.as_deref() == Some(m.capital_account.as_str()) {
                return Err(format!(
                    "{}'s distributions account {} is {}'s capital account",
                    other.name, m.capital_account, m.name
                ));
            }
        }
    }
    // files_jointly_with names another member, and both say so.
    for m in &members {
        if let Some(j) = &m.files_jointly_with {
            if *j == m.name {
                return Err(format!(
                    "member {}: files_jointly_with names themself",
                    m.name
                ));
            }
            match members.iter().find(|o| o.name == *j) {
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
    if !members.is_empty() {
        let total = members
            .iter()
            .try_fold(Decimal::ZERO, |t, m| t.checked_add(m.share))
            .ok_or("member shares are too large to add")?;
        if total != Decimal::ONE {
            return Err(format!("member shares sum to {total}, not 100%"));
        }
    }
    Ok(members)
}

fn members_json(members: &[Member]) -> Value {
    Value::Array(
        members
            .iter()
            .map(|m| {
                let mut o = serde_json::Map::new();
                o.insert("name".into(), json!(m.name));
                o.insert("share".into(), json!(m.share.normalize().to_string()));
                o.insert("capital_account".into(), json!(m.capital_account));
                if let Some(d) = &m.distribution_account {
                    o.insert("distribution_account".into(), json!(d));
                }
                if let Some(j) = &m.files_jointly_with {
                    o.insert("files_jointly_with".into(), json!(j));
                }
                Value::Object(o)
            })
            .collect(),
    )
}

/// Members as journaled (shape only: the fields' types, and a share that
/// is a decimal).
fn members_from_json(v: &Value) -> Result<Vec<Member>, String> {
    let list = want_list("company.members", v)?;
    list.iter()
        .enumerate()
        .map(|(i, m)| {
            let s = |f: &str| m.get(f).and_then(Value::as_str).map(str::to_string);
            let bad = |f: &str| format!("company.members {}: bad {f}", i + 1);
            Ok(Member {
                name: s("name").ok_or_else(|| bad("name"))?,
                share: s("share")
                    .as_deref()
                    .and_then(decimal)
                    .ok_or_else(|| bad("share"))?,
                capital_account: s("capital_account").ok_or_else(|| bad("capital_account"))?,
                distribution_account: s("distribution_account"),
                files_jointly_with: s("files_jointly_with"),
            })
        })
        .collect()
}

/// A setting as journaled, with where it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct Recorded {
    /// The value, canonical (INVARIANT 1).
    pub value: Value,
    /// The journal line that set it.
    pub seq: u64,
    /// Who decided it.
    pub source: Source,
}

/// One change in a `settings.set`: a key and its new value, or `None` to
/// clear it back to its default.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    /// The key.
    pub key: String,
    /// The new value; `None` clears it.
    pub value: Option<Value>,
}

/// The settings the journal holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    values: BTreeMap<String, Recorded>,
    /// The seq of the last `settings.set` replayed or made, if any: books
    /// whose journal holds one keep their settings there (and nowhere
    /// else).
    last_seq: Option<u64>,
}

impl Settings {
    /// Whether the journal holds settings (any `settings.set`). Books
    /// whose journal holds none still read a settings file (legacy mode,
    /// until they import one).
    pub fn is_journaled(&self) -> bool {
        self.last_seq.is_some()
    }

    /// The seq of the last `settings.set`, if any.
    pub fn last_seq(&self) -> Option<u64> {
        self.last_seq
    }

    /// The setting `name`, if set.
    pub fn get(&self, name: &str) -> Option<&Recorded> {
        self.values.get(name)
    }

    /// Every setting that is set, in [`KEYS`] order.
    pub fn iter(&self) -> impl Iterator<Item = (&'static Key, &Recorded)> {
        KEYS.iter()
            .filter_map(|k| self.values.get(k.name).map(|r| (k, r)))
    }

    fn str(&self, name: &str) -> Option<&str> {
        self.values.get(name).and_then(|r| r.value.as_str())
    }

    fn codes(&self, name: &str) -> Vec<String> {
        self.values
            .get(name)
            .and_then(|r| r.value.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The company, as the settings describe it (INVARIANT 5 for what is
    /// unset). `base_currency` is the books' own (fixed when they were
    /// created): the profile shows it.
    pub fn company(&self, base_currency: &str) -> Company {
        let mut c = Company::default();
        if let Some(n) = self.str("company.name") {
            c.name = n.to_string();
        }
        if let Some(fy) = self.str("company.fiscal_year_start")
            && let Ok(md) = parse_month_start_lenient(fy)
        {
            c.fiscal_year_start = md;
        }
        if let Some(m) = self.values.get("company.members") {
            c.members = members_from_json(&m.value).unwrap_or_default();
        }
        if self.str("books.accounting_basis") == Some("cash") {
            c.basis = "Cash".to_string();
        }
        c.tax_form = match (
            self.str("company.tax_form"),
            self.str("company.tax_classification"),
        ) {
            (Some(f), _) => TaxForm::parse(f),
            (None, Some(cl)) => form_for_classification(cl),
            (None, None) => None,
        };
        c.close_into = self.str("books.close_into").map(str::to_string);
        c.close_sweep = self.codes("books.close_sweep");
        let mut profile = vec![("base_currency".to_string(), base_currency.to_string())];
        for (k, r) in self.iter() {
            if k.toml.0 == "company"
                && k.kind != Kind::Members
                && let Some(s) = r.value.as_str()
            {
                profile.push((k.toml.1.to_string(), s.to_string()));
            }
        }
        // The profile in a settings file's order: name first.
        profile.sort_by_key(|(k, _)| (k != "name", k != "base_currency"));
        c.profile = profile;
        c
    }

    /// `policy.evidence.threshold` as set.
    pub fn receipt_threshold(&self) -> Option<&str> {
        self.str("policy.evidence.threshold")
    }

    /// `policy.evidence.required_accounts`.
    pub fn receipt_required_accounts(&self) -> Vec<String> {
        self.codes("policy.evidence.required_accounts")
    }

    /// `policy.evidence.exempt_accounts`.
    pub fn receipt_exempt_accounts(&self) -> Vec<String> {
        self.codes("policy.evidence.exempt_accounts")
    }

    /// `books.cash_accounts`.
    pub fn cash_accounts(&self) -> Vec<String> {
        self.codes("books.cash_accounts")
    }

    /// `books.financing_accounts`.
    pub fn financing_accounts(&self) -> Vec<String> {
        self.codes("books.financing_accounts")
    }

    /// `books.investing_accounts`.
    pub fn investing_accounts(&self) -> Vec<String> {
        self.codes("books.investing_accounts")
    }

    /// `books.asset_tag_prefix`, if set.
    pub fn asset_tag_prefix(&self) -> Option<&str> {
        self.str("books.asset_tag_prefix")
    }

    /// `policy.parties`.
    pub fn parties(&self) -> PartyPolicy {
        self.str("policy.parties")
            .and_then(PartyPolicy::parse)
            .unwrap_or_default()
    }

    /// `policy.approval_rules`.
    pub fn approval_rules(&self) -> ApprovalMode {
        match self.str("policy.approval_rules") {
            Some("enforce") => ApprovalMode::Enforce,
            _ => ApprovalMode::Off,
        }
    }

    /// `backup.git_remote`, if set.
    pub fn backup_git_remote(&self) -> Option<&str> {
        self.str("backup.git_remote")
    }

    /// `backup.folder`, if set.
    pub fn backup_folder(&self) -> Option<&str> {
        self.str("backup.folder")
    }

    /// `company.timezone`, if set.
    pub fn timezone(&self) -> Option<&str> {
        self.str("company.timezone")
    }

    /// Today in the company's time zone (`company.timezone`), else in the
    /// computer's. Not UTC: in the evening (US time) UTC is already
    /// tomorrow, and "through today" must not include it.
    pub fn today(&self) -> NaiveDate {
        today_in(self.timezone())
    }

    /// Check that the settings hold together (INVARIANT 2): what no one
    /// key's check can see.
    ///
    /// ```text
    /// INVARIANTS:
    ///   1. With members, books.close_into is unset (a close goes to each
    ///      member's capital account).
    ///   2. Without company.tax_form, a company.tax_classification names
    ///      exactly one return.
    /// ```
    pub fn check_whole(&self) -> Result<(), String> {
        let has_members = self
            .values
            .get("company.members")
            .and_then(|r| r.value.as_array())
            .is_some_and(|a| !a.is_empty());
        if has_members && self.values.contains_key("books.close_into") {
            return Err(
                "books.close_into is for single-owner businesses; with company.members a close goes to each member's capital account"
                    .to_string(),
            );
        }
        if self.str("company.tax_form").is_none()
            && let Some(c) = self.str("company.tax_classification")
            && form_for_classification(c).is_none()
        {
            return Err(format!(
                "company.tax_classification {c:?}: expected partnership, S corporation, C corporation, or disregarded entity (or set company.tax_form)"
            ));
        }
        Ok(())
    }

    /// Check a change before it is journaled, and return it with every
    /// value canonical and every change that changes nothing left out
    /// (INVARIANTS 1, 2, 4). An empty result changes nothing.
    pub fn check_changes(
        &self,
        changes: &[Change],
        source: &Source,
    ) -> Result<Vec<Change>, String> {
        source.check()?;
        let mut next = self.clone();
        let mut out: Vec<Change> = Vec::new();
        for ch in changes {
            let k = key(&ch.key).ok_or_else(|| {
                format!(
                    "unknown setting {:?}: books.settings lists every key",
                    ch.key
                )
            })?;
            if out.iter().any(|o| o.key == ch.key) {
                return Err(format!("{} is changed twice", ch.key));
            }
            let value = match &ch.value {
                None | Some(Value::Null) => None,
                Some(v) => Some(check(k, v)?),
            };
            if next.values.get(k.name).map(|r| &r.value) == value.as_ref() {
                continue;
            }
            next.apply_one(k.name, value.clone(), 0, source);
            out.push(Change {
                key: k.name.to_string(),
                value,
            });
        }
        next.check_whole()?;
        Ok(out)
    }

    fn apply_one(&mut self, name: &str, value: Option<Value>, seq: u64, source: &Source) {
        match value {
            Some(value) => {
                self.values.insert(
                    name.to_string(),
                    Recorded {
                        value,
                        seq,
                        source: source.clone(),
                    },
                );
            }
            None => {
                self.values.remove(name);
            }
        }
    }

    /// Apply a journaled `settings.set` (replay, or a change already
    /// checked and journaled): every key known and every value of its
    /// key's shape (INVARIANT 3), or nothing changes.
    pub fn apply(&mut self, changes: &[Change], source: &Source, seq: u64) -> Result<(), String> {
        for ch in changes {
            let k = key(&ch.key).ok_or_else(|| format!("unknown setting {:?}", ch.key))?;
            if let Some(v) = &ch.value {
                shape(k, v)?;
            }
        }
        for ch in changes {
            self.apply_one(&ch.key, ch.value.clone(), seq, source);
        }
        self.last_seq = Some(seq);
        Ok(())
    }
}

/// Today in `timezone` (an IANA name), else in the computer's time zone;
/// an unknown name (books a later version wrote) counts as unset.
pub fn today_in(timezone: Option<&str>) -> NaiveDate {
    let zone = timezone
        .and_then(|z| jiff::tz::TimeZone::get(z).ok())
        .unwrap_or_else(jiff::tz::TimeZone::system);
    let d = jiff::Timestamp::now().to_zoned(zone).date();
    NaiveDate::from_ymd_opt(i32::from(d.year()), d.month() as u32, d.day() as u32)
        .unwrap_or_else(|| chrono::Local::now().date_naive())
}

/// Mask an identity value to its last four: "**-***5893".
pub fn mask(value: &str) -> String {
    let n = value.chars().count();
    value
        .chars()
        .enumerate()
        .map(|(i, c)| {
            if i + 4 < n && c.is_ascii_digit() {
                '*'
            } else {
                c
            }
        })
        .collect()
}

/// A setting's value as text, for display (`books.start`, the viewer):
/// strings as they are, lists joined, identity values masked.
pub fn display(key: &Key, v: &Value, full: bool) -> String {
    let s = match v {
        Value::String(s) => s.clone(),
        Value::Array(a) if key.kind == Kind::Members => a
            .iter()
            .map(|m| {
                format!(
                    "{} {}",
                    m.get("name").and_then(Value::as_str).unwrap_or("?"),
                    m.get("share").and_then(Value::as_str).unwrap_or("?")
                )
            })
            .collect::<Vec<_>>()
            .join(", "),
        Value::Array(a) => a
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", "),
        other => other.to_string(),
    };
    if key.sensitivity == Sensitivity::Identity && !full {
        mask(&s)
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn where_backups_go_is_checked() {
        let remote = key("backup.git_remote").unwrap();
        let folder = key("backup.folder").unwrap();
        for ok in [
            "git@host:owner/books.git",
            "https://host/owner/books",
            "/srv/books.git",
        ] {
            assert_eq!(check(remote, &json!(ok)).unwrap(), json!(ok), "{ok}");
        }
        assert_eq!(
            check(remote, &json!("  ssh://h/b  ")).unwrap(),
            json!("ssh://h/b")
        );
        for bad in [
            "--upload-pack=x",
            "-",
            "",
            "a\nb",
            "acme.git",
            "backups/acme.git",
            "~/x.git",
            "./x",
        ] {
            assert!(check(remote, &json!(bad)).is_err(), "{bad}");
        }
        assert!(check(remote, &json!("file:///srv/b.git")).is_ok());
        #[cfg(unix)]
        assert_eq!(
            check(folder, &json!("/media/backup")).unwrap(),
            json!("/media/backup")
        );
        for bad in ["backup", "./backup", "~/backup", "-x", ""] {
            assert!(check(folder, &json!(bad)).is_err(), "{bad}");
        }
    }

    fn src() -> Source {
        Source {
            decided_by: "Owner".into(),
            decided_on: NaiveDate::from_ymd_opt(2026, 10, 17).unwrap(),
            citation: None,
            note: None,
        }
    }

    fn set(s: &Settings, k: &str, v: Value) -> Result<Vec<Change>, String> {
        s.check_changes(
            &[Change {
                key: k.into(),
                value: Some(v),
            }],
            &src(),
        )
    }

    /// INVARIANT 1: one value, one spelling; a change to the same value
    /// changes nothing.
    #[test]
    fn values_are_canonical() {
        let s = Settings::default();
        let got = set(&s, "policy.parties", json!(" Required ")).unwrap();
        assert_eq!(got[0].value, Some(json!("require")));
        let got = set(&s, "company.ein", json!("288525893")).unwrap();
        assert_eq!(got[0].value, Some(json!("28-8525893")));
        let got = set(&s, "company.tax_form", json!("Form 1120-S")).unwrap();
        assert_eq!(got[0].value, Some(json!("1120s")));
        let mut s = Settings::default();
        let ch = set(&s, "company.name", json!(" Acme LLC ")).unwrap();
        s.apply(&ch, &src(), 7).unwrap();
        assert!(
            set(&s, "company.name", json!("Acme LLC"))
                .unwrap()
                .is_empty()
        );
        assert_eq!(s.get("company.name").unwrap().seq, 7);
    }

    /// INVARIANT 4: no full TIN outside company.ein.
    #[test]
    fn no_full_tin_but_the_companys_ein() {
        let s = Settings::default();
        assert!(set(&s, "company.address", json!("SSN 123-45-6789")).is_err());
        assert!(set(&s, "company.cpa", json!("EIN 12-3456789")).is_err());
        assert!(set(&s, "company.ein", json!("12-3456789")).is_ok());
        // Not TIN-shaped: a state number, a phone number, a DUNS.
        assert!(set(&s, "company.state_entity_number", json!("14745117-0160")).is_ok());
        assert!(set(&s, "company.address", json!("call 801-555-0142")).is_ok());
        assert!(set(&s, "company.duns", json!("149976972")).is_ok());
        // Bare nine digits read as an SSN anywhere but the D-U-N-S key.
        assert!(set(&s, "company.cpa", json!("Pat 149976972")).is_err());
        let mut bad = src();
        bad.note = Some("their SSN is 123-45-6789".into());
        assert!(
            Settings::default()
                .check_changes(
                    &[Change {
                        key: "company.name".into(),
                        value: Some(json!("X"))
                    }],
                    &bad
                )
                .is_err()
        );
    }

    /// Review of P1 (2026-10-17): one value, one spelling, for zones,
    /// amounts and returns; a D-U-N-S never in a TIN's shape; members'
    /// shares, as journaled, in (0, 1].
    #[test]
    fn spellings_and_shapes() {
        let s = Settings::default();
        let one = |k: &str, v: Value| set(&s, k, v).map(|c| c[0].value.clone().unwrap());
        assert_eq!(
            one("company.timezone", json!("america/denver")).unwrap(),
            json!("America/Denver")
        );
        assert_eq!(
            one("company.timezone", json!("utc")).unwrap(),
            one("company.timezone", json!("UTC")).unwrap()
        );
        assert_eq!(
            one("policy.evidence.threshold", json!("025.00")).unwrap(),
            json!("25.00")
        );
        assert_eq!(
            one("policy.evidence.threshold", json!("-0.00")).unwrap(),
            json!("0.00")
        );
        for (spelling, key) in [
            ("Schedule-C", "schedule_c"),
            ("Form_1065", "1065"),
            ("form 1120-S", "1120s"),
            ("1120", "1120"),
        ] {
            assert_eq!(
                one("company.tax_form", json!(spelling)).unwrap(),
                json!(key),
                "{spelling}"
            );
        }
        assert!(one("company.tax_form", json!("990")).is_err());
        assert_eq!(
            one("company.duns", json!("14-997-6972")).unwrap(),
            json!("149976972")
        );
        for tin in ["123-45-6789", "12-3456789", "1234-56789"] {
            assert!(one("company.duns", json!(tin)).is_err(), "{tin}");
        }
        let k = key("company.members").unwrap();
        assert!(
            shape(
                k,
                &json!([{"name": "A", "share": "1", "capital_account": "3000"}])
            )
            .is_ok()
        );
        for share in ["0", "-0.5", "1.0001", "79228162514264337593543950335"] {
            assert!(
                shape(
                    k,
                    &json!([{"name": "A", "share": share, "capital_account": "3000"}])
                )
                .is_err(),
                "{share}"
            );
        }
    }

    #[test]
    fn printed_tins() {
        for yes in [
            "123.45.6789",
            "\u{ff11}\u{ff12}\u{ff13}-\u{ff14}\u{ff15}-\u{ff16}\u{ff17}\u{ff18}\u{ff19}",
            "123 \u{2013} 45 \u{2013} 6789",
            "123-45-6789",
            "a 12-3456789.",
            "(123-45-6789)",
            "1-123-45-6789",
            "12-3456789-0",
            "123\u{2010}45\u{2010}6789",
            "123 45 6789",
            "SSN 123456789",
            "12\u{2212}3456789",
        ] {
            assert!(holds_printed_tin(yes), "{yes}");
        }
        for no in [
            "1123-45-6789",
            "123-45-67890",
            "12-34567890",
            "1234567890",
            "14745117-0160",
            "801-555-0142",
            "2026-10-04",
            "100 Example Way, Midvale, UT 84047",
        ] {
            assert!(!holds_printed_tin(no), "{no}");
        }
    }

    /// INVARIANT 2: the settings after a change hold together.
    #[test]
    fn changes_hold_together() {
        let s = Settings::default();
        let members = json!([
            {"name": "A", "share": "50%", "capital_account": "3000"},
            {"name": "B", "share": "0.5", "capital_account": "3001"}
        ]);
        let both = s.check_changes(
            &[
                Change {
                    key: "company.members".into(),
                    value: Some(members.clone()),
                },
                Change {
                    key: "books.close_into".into(),
                    value: Some(json!("3000")),
                },
            ],
            &src(),
        );
        assert!(both.unwrap_err().contains("close_into"));
        assert!(
            set(&s, "company.tax_classification", json!("trust"))
                .unwrap_err()
                .contains("tax_classification")
        );
        // A tax form makes any classification acceptable (it isn't read).
        let ok = s.check_changes(
            &[
                Change {
                    key: "company.tax_classification".into(),
                    value: Some(json!("trust")),
                },
                Change {
                    key: "company.tax_form".into(),
                    value: Some(json!("1120")),
                },
            ],
            &src(),
        );
        assert!(ok.is_ok());
        let got = set(&s, "company.members", members).unwrap();
        assert_eq!(
            got[0].value.as_ref().unwrap()[0]["share"],
            json!("0.5"),
            "\"50%\" is kept as 0.5"
        );
    }

    /// The members' checks carried over from the settings file.
    #[test]
    fn members_are_checked() {
        let s = Settings::default();
        let two = |a: Value, b: Value| set(&s, "company.members", json!([a, b]));
        let m = |n: &str, sh: &str, c: &str| json!({"name": n, "share": sh, "capital_account": c});
        assert!(two(m("A", "50%", "3000"), m("B", "40%", "3001")).is_err());
        assert!(two(m("A", "60%", "3000"), m("B", "40%", "3001")).is_ok());
        assert!(two(m("A", "50%", "3000"), m("A", "50%", "3001")).is_err());
        assert!(two(m("A", "50%", "3000"), m("B", "50%", "3000")).is_err());
        assert!(
            set(
                &s,
                "company.members",
                json!([{"name": "A", "share": "100%"}])
            )
            .is_err()
        );
        assert!(
            set(
                &s,
                "company.members",
                json!([{"name": "A", "share": "1e0", "capital_account": "3000"}])
            )
            .is_err()
        );
        let j = |a: &str, b: &str| {
            two(
                json!({"name": "A", "share": "50%", "capital_account": "3000", "files_jointly_with": a}),
                json!({"name": "B", "share": "50%", "capital_account": "3001", "files_jointly_with": b}),
            )
        };
        assert!(j("B", "A").is_ok());
        assert!(j("B", "").unwrap_err().contains("set it on both"));
        assert!(j("C", "").unwrap_err().contains("isn't a member"));
        assert!(j("A", "").unwrap_err().contains("themself"));
    }

    #[test]
    fn kinds_refuse_what_they_cant_honor() {
        let s = Settings::default();
        for (k, v) in [
            ("company.fiscal_year_start", json!("02-29")),
            ("company.fiscal_year_start", json!("13-01")),
            ("company.fiscal_year_start", json!("07-15")),
            ("policy.evidence.threshold", json!("25 dollars")),
            ("policy.evidence.threshold", json!("-1")),
            ("policy.evidence.threshold", json!("1e3")),
            ("books.accounting_basis", json!("modified")),
            ("company.timezone", json!("Mars/Olympus")),
            ("company.formed", json!("2026-02-30")),
            ("company.formed", json!("10000-01-01")),
            ("company.duns", json!("12345678")),
            ("company.irs_name_control", json!("WALNX")),
            ("books.asset_tag_prefix", json!("W-L")),
            ("books.cash_accounts", json!(["1000", "1000"])),
            ("books.cash_accounts", json!(["1000", " "])),
            ("company.name", json!("two\nlines")),
            ("company.name", json!(5)),
            ("no.such.key", json!("x")),
        ] {
            assert!(set(&s, k, v.clone()).is_err(), "{k} = {v}");
        }
        assert_eq!(
            set(&s, "company.timezone", json!("America/Denver")).unwrap()[0].value,
            Some(json!("America/Denver"))
        );
        assert_eq!(
            set(&s, "policy.evidence.threshold", json!("0.00")).unwrap()[0].value,
            Some(json!("0.00"))
        );
    }

    /// INVARIANT 5: unset keys are the old file's defaults.
    #[test]
    fn unset_is_the_default() {
        let s = Settings::default();
        let c = s.company("USD");
        assert_eq!(
            (c.name.as_str(), c.fiscal_year_start, c.basis.as_str()),
            (DEFAULT_NAME, (1, 1), "Accrual")
        );
        assert_eq!(s.parties(), PartyPolicy::Suggest);
        assert_eq!(s.approval_rules(), ApprovalMode::Off);
        assert!(!s.is_journaled());
    }

    /// INVARIANT 3: replay takes any value of the right shape, and refuses
    /// a wrong shape or an unknown key without changing anything.
    #[test]
    fn replay_takes_shapes() {
        let mut s = Settings::default();
        // A threshold no command would write, but a decimal: replayed.
        s.apply(
            &[Change {
                key: "policy.evidence.threshold".into(),
                value: Some(json!("-5")),
            }],
            &src(),
            3,
        )
        .unwrap();
        assert_eq!(s.last_seq(), Some(3));
        let before = s.clone();
        assert!(
            s.apply(
                &[
                    Change {
                        key: "company.name".into(),
                        value: Some(json!("X"))
                    },
                    Change {
                        key: "books.cash_accounts".into(),
                        value: Some(json!("1000"))
                    },
                ],
                &src(),
                4
            )
            .is_err()
        );
        assert_eq!(s, before, "all or nothing");
        assert!(
            s.apply(
                &[Change {
                    key: "no.such".into(),
                    value: Some(json!("x"))
                }],
                &src(),
                5
            )
            .is_err()
        );
    }

    #[test]
    fn masks_identity() {
        let k = key("company.ein").unwrap();
        assert_eq!(display(k, &json!("28-8525893"), false), "**-***5893");
        assert_eq!(display(k, &json!("28-8525893"), true), "28-8525893");
    }
}
