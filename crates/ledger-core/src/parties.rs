//! Parties: everyone the company deals with, in one directory.
//!
//! A party is a vendor, a contractor, a customer, an employee, a member, a
//! payout platform, a tax agency, a bank or card issuer, a landlord, or
//! several of these. Vendor and customer *records* (bills, invoices,
//! payments, aging) stay where they are and link to their party; a party
//! adds who they are (names, facts) and links entries to them.
//!
//! ```text
//! GOAL: answer "who is this, and what has the company done with them?"
//!       for anyone the books touch, without a CRM's sprawl.
//! SOURCE: docs/parties-and-attachments.md; DECISION (owner,
//!         2026-10-15); IRS truncated TINs: Treas. Reg. §301.6109-4.
//! INVARIANTS:
//!   1. No party field holds a full taxpayer identification number: any
//!      run of nine digits, in any script, separated or not (`has_full_tin`:
//!      up to three spaces, hyphens, dots, slashes or underscores, in any
//!      of their Unicode forms, between neighbors, any run of spaces
//!      counting once; or a 3-2-4 or 2-7 group with commas or parentheses
//!      between its groups) is refused. A W-9's TIN is kept as its last
//!      four digits only.
//!   2. W-9 facts need a 1099 role (contractor or landlord); W-4 facts need
//!      the employee role. Taxpayer-ID facts (`TinFacts`: the TIN's type,
//!      last four, and vault record) are allowed for any role: members
//!      need theirs for K-1s.
//!   2a. A party's TIN facts and W-9 facts never disagree: both on file
//!      means the same last four (DECISION, owner, 2026-10-15).
//!   3. Names are unique, ignoring case and surrounding space, among parties
//!      not archived.
//!   4. Every vendor and customer record links to exactly one party.
//!   5. A party's `creditor_of` names only liability accounts that are
//!      not placeholders, payables or payroll withholdings: what the
//!      company owes that party itself on (a card's issuer, a loan's
//!      lender, a member for what the members advanced). A credit to one
//!      never counts as paying that party (rules.md B-35; DECISION, owner,
//!      2026-10-20).
//! ```

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Opaque party handle.
pub type PartyId = u64;

/// What a party is to the company.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Sells to the company (has a vendor record: bills, payments).
    Vendor,
    /// A vendor paid for services who gets a 1099-NEC.
    Contractor,
    /// Buys from the company (has a customer record: invoices).
    Customer,
    /// Works for the company on payroll.
    Employee,
    /// A member (owner) of the company, as in wanderbooks.toml.
    Member,
    /// Pays the company out of sales (an app store, a payment processor).
    PayoutPlatform,
    /// Taxes the company (the IRS, a state).
    TaxAgency,
    /// Regulates or registers the company without taxing it (a secretary
    /// of state, a division of corporations, a licensing board).
    GovernmentAgency,
    /// Holds the company's money or credit (a bank, a card issuer).
    FinancialInstitution,
    /// Rents property to the company (gets a 1099-MISC for rent).
    Landlord,
}

impl Role {
    /// Every role, in display order.
    pub const ALL: [Role; 10] = [
        Role::Vendor,
        Role::Contractor,
        Role::Customer,
        Role::Employee,
        Role::Member,
        Role::PayoutPlatform,
        Role::TaxAgency,
        Role::GovernmentAgency,
        Role::FinancialInstitution,
        Role::Landlord,
    ];

    /// Its wire name, e.g. `payout_platform`.
    pub fn key(self) -> &'static str {
        match self {
            Role::Vendor => "vendor",
            Role::Contractor => "contractor",
            Role::Customer => "customer",
            Role::Employee => "employee",
            Role::Member => "member",
            Role::PayoutPlatform => "payout_platform",
            Role::TaxAgency => "tax_agency",
            Role::GovernmentAgency => "government_agency",
            Role::FinancialInstitution => "financial_institution",
            Role::Landlord => "landlord",
        }
    }

    /// The role named `key`.
    pub fn parse(key: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|r| r.key() == key)
    }

    /// Whether this role files a 1099 (and so may carry W-9 facts).
    pub fn gets_1099(self) -> bool {
        matches!(self, Role::Contractor | Role::Landlord)
    }
}

/// What a W-9 said, without the number itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct W9Facts {
    /// When it was received.
    pub received: NaiveDate,
    /// Line 3 federal tax classification, e.g. "individual/sole proprietor".
    pub entity_type: String,
    /// Line 4 exempt payee code, if any.
    #[serde(default)]
    pub exempt_payee_code: Option<String>,
    /// Line 4 FATCA exemption code, if any.
    #[serde(default)]
    pub fatca_code: Option<String>,
    /// The TIN's last four digits (exactly four digits).
    pub tin_last4: String,
    /// Where the form itself is kept, e.g. "wanderbooks vault" or "CPA portal".
    pub kept_at: String,
    /// SHA-256 of the vault record holding the full TIN, when one exists:
    /// proves the vault held it without the books holding it.
    #[serde(default)]
    pub vault_record: Option<String>,
}

/// A taxpayer identification number's type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TinType {
    /// Social Security number (an individual).
    Ssn,
    /// Employer identification number (a business).
    Ein,
    /// IRS individual taxpayer identification number.
    Itin,
}

impl TinType {
    /// Its wire name: ssn, ein, or itin.
    pub fn key(self) -> &'static str {
        match self {
            TinType::Ssn => "ssn",
            TinType::Ein => "ein",
            TinType::Itin => "itin",
        }
    }

    /// How a form names it: SSN, EIN, or ITIN.
    pub fn label(self) -> &'static str {
        match self {
            TinType::Ssn => "SSN",
            TinType::Ein => "EIN",
            TinType::Itin => "ITIN",
        }
    }
}

/// What the books know of a party's taxpayer identification number,
/// without the number: its type, its last four digits, and the hash of
/// the TIN vault record that holds it in full. Any role may have one
/// (a member's SSN for their K-1, a contractor's from their W-9).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TinFacts {
    /// SSN, EIN, or ITIN.
    pub tin_type: TinType,
    /// The last four digits (exactly four digits).
    pub last4: String,
    /// SHA-256 of the vault record holding the full TIN; `None` once the
    /// record is purged (or when the vault never held it).
    #[serde(default)]
    pub vault_record: Option<String>,
    /// When and why the vault's copy was purged, if it was.
    #[serde(default)]
    pub purged: Option<String>,
}

impl TinFacts {
    /// Check the facts' form. Returns why not.
    pub fn check(&self) -> Result<(), String> {
        if self.last4.len() != 4 || !self.last4.chars().all(|c| c.is_ascii_digit()) {
            return Err("a TIN's last four is exactly four digits".into());
        }
        if let Some(h) = &self.vault_record
            && (h.len() != 64 || !h.chars().all(|c| c.is_ascii_hexdigit()))
        {
            return Err("vault_record is a SHA-256 in hex".into());
        }
        if let Some(t) = &self.purged {
            check_text("purged", t)?;
        }
        Ok(())
    }
}

/// What a W-4 said, without the SSN.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct W4Facts {
    /// When it was received.
    pub received: NaiveDate,
    /// The work state, two letters.
    pub state: String,
    /// Where the form itself is kept.
    pub kept_at: String,
}

/// One party.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Party {
    /// Opaque handle.
    pub id: PartyId,
    /// Legal or trade name, as on documents.
    pub name: String,
    /// A shorter name to show, if any.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Roles set by hand (vendor and customer also follow their records).
    pub roles: BTreeSet<Role>,
    /// Website.
    #[serde(default)]
    pub website: Option<String>,
    /// Email address, where the company writes to them (A-13): copied onto
    /// an invoice's bill-to snapshot at issue.
    #[serde(default)]
    pub email: Option<String>,
    /// The terms this party grants the company as a customer: an invoice's
    /// due date falls back to these (A-13).
    #[serde(default)]
    pub default_terms_as_customer: Option<crate::Terms>,
    /// The terms this party, as a vendor, gives the company: a bill's due
    /// date falls back to these (A-13).
    #[serde(default)]
    pub default_terms_as_vendor: Option<crate::Terms>,
    /// Business address.
    #[serde(default)]
    pub address: Option<String>,
    /// Notes.
    #[serde(default)]
    pub notes: Option<String>,
    /// No longer dealt with; history stays.
    #[serde(default)]
    pub archived: bool,
    /// W-9 facts (contractor, landlord).
    #[serde(default)]
    pub w9: Option<W9Facts>,
    /// W-4 facts (employee).
    #[serde(default)]
    pub w4: Option<W4Facts>,
    /// Taxpayer-ID facts (any role): the TIN's type and last four, and
    /// its vault record. The full number is only ever in the vault.
    #[serde(default)]
    pub tin: Option<TinFacts>,
    /// Its vendor record, if it bills the company.
    #[serde(default)]
    pub vendor: Option<crate::VendorId>,
    /// Its customer record, if the company invoices it.
    #[serde(default)]
    pub customer: Option<crate::CustomerId>,
    /// The liability accounts the company owes this party itself on: the
    /// card it issued, the loan it made, what the members advanced (each
    /// member). INVARIANT 5.
    #[serde(default)]
    pub creditor_of: BTreeSet<crate::AccountId>,
}

impl Party {
    /// The name to show: the display name, else the name.
    pub fn label(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.name)
    }

    /// The last four digits of its TIN, when the books know them: its TIN
    /// facts' (`books.set_tin`), else its W-9 facts'.
    ///
    /// ```text
    /// GOAL: one answer to "which TIN is on file for them?" wherever the
    ///       books show a last four (defect 11: the directory showed a
    ///       member's as missing because it read W-9 facts only).
    /// SOURCE: rules.md B-26 (Treas. Reg. §301.6109-4(a): a truncated TIN
    ///         shows its last four); INVARIANT 2a (DECISION, owner,
    ///         2026-10-15: the two never disagree, so either answers).
    /// SCENARIOS:
    ///   1. A member with TIN facts and no W-9: the TIN facts' last four.
    ///   2. A contractor with W-9 facts only: the W-9's.
    ///   3. Both on file: the same four (2a), the TIN facts'.
    ///   4. Neither: None.
    ///   5. A TIN purged from the vault keeps its facts' last four
    ///      (books.purge_tin drops only the vault record): still shown.
    ///   6. NEVER more than four digits: facts whose last four isn't
    ///      exactly four digits (built by hand, not through the checking
    ///      setters) show nothing from that source.
    /// ```
    pub fn tin_last4(&self) -> Option<&str> {
        // Exactly four digits or nothing, however the facts were built
        // (the setters check it; this never shows more, B-26).
        let four = |s: &str| s.len() == 4 && s.chars().all(|c| c.is_ascii_digit());
        self.tin
            .as_ref()
            .map(|t| t.last4.as_str())
            .filter(|s| four(s))
            .or_else(|| {
                self.w9
                    .as_ref()
                    .map(|w| w.tin_last4.as_str())
                    .filter(|s| four(s))
            })
    }

    /// Its roles: the ones set by hand, plus vendor and customer when it
    /// has those records.
    pub fn all_roles(&self) -> BTreeSet<Role> {
        let mut r = self.roles.clone();
        if self.vendor.is_some() {
            r.insert(Role::Vendor);
        }
        if self.customer.is_some() {
            r.insert(Role::Customer);
        }
        r
    }
}

/// The key names are compared by: case and surrounding space ignored.
pub fn name_key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// The characters that may sit between a TIN's digits (`291-12-2846`,
/// `291.12.2846`, `291/12/2846`, `291 - 12 - 2846`): in ASCII a space,
/// hyphen, dot, slash or underscore (no other, not a tab or a line
/// break); past ASCII, every form of them Unicode has, so a TIN can't slip
/// through typed in another script (`docs/spec/sources/unicode-tin-text.md`,
/// Unicode 17.0): every space (general category Zs: the no-break, thin and
/// ideographic spaces), every dash (property Dash: the hyphens, en and em
/// dashes, minus sign, Hebrew maqaf, fullwidth and small hyphen-minus),
/// and the characters whose compatibility form is a dot, slash or
/// underscore (the fullwidth and small full stops, the one dot leader,
/// the fullwidth solidus, the fullwidth and presentation low lines).
fn tin_separator(c: char) -> bool {
    match c {
        ' ' | '-' | '.' | '/' | '_' => true,
        c if c.is_ascii() => false,
        // White_Space less the line breaks (NEL, LS, PS): the Zs spaces.
        c if c.is_whitespace() => !matches!(c, '\u{85}' | '\u{2028}' | '\u{2029}'),
        c => matches!(
            c,
            // Dash (PropList.txt), less U+002D above.
            '\u{58A}'
                | '\u{5BE}'
                | '\u{1400}'
                | '\u{1806}'
                | '\u{2010}'..='\u{2015}'
                | '\u{2053}'
                | '\u{207B}'
                | '\u{208B}'
                | '\u{2212}'
                | '\u{2E17}'
                | '\u{2E1A}'
                | '\u{2E3A}'..='\u{2E3B}'
                | '\u{2E40}'
                | '\u{2E5D}'
                | '\u{301C}'
                | '\u{3030}'
                | '\u{30A0}'
                | '\u{FE31}'..='\u{FE32}'
                | '\u{FE58}'
                | '\u{FE63}'
                | '\u{FF0D}'
                | '\u{10D6E}'
                | '\u{10EAD}'
                // NFKC '.', '/', '_' (UnicodeData.txt decompositions).
                | '\u{2024}'
                | '\u{FE52}'
                | '\u{FF0E}'
                | '\u{FF0F}'
                | '\u{FE33}'..='\u{FE34}'
                | '\u{FE4D}'..='\u{FE4F}'
                | '\u{FF3F}'
        ),
    }
}

/// Whether `c` is invisible, so a TIN's digits read as neighbors across it
/// however many there are (`1\u{200B}2` shows as `12`): Unicode's
/// Default_Ignorable_Code_Point (DerivedCoreProperties.txt, 17.0): the
/// soft hyphen, the zero-width space and joiners, the left-to-right and
/// right-to-left marks and the other directional formatting characters,
/// the word joiner, the byte-order mark, the variation selectors, the tags
/// and the fillers. None is ASCII.
fn tin_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{AD}'
            | '\u{34F}'
            | '\u{61C}'
            | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFF8}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E0FFF}'
    )
}

/// The most separators that may sit between two of a TIN's digits
/// (`" - "` is three); a run of spaces counts once however long it is,
/// and invisible characters ([`tin_invisible`]) don't count.
const MAX_TIN_GAP: usize = 3;

/// Whether `c` is one of the spaces between a TIN's digits: a run of
/// them, however long, counts as one separator ([`tin_gap`]), so
/// `12    3456789` reads as an EIN.
fn tin_space(c: char) -> bool {
    c.is_whitespace() && tin_separator(c)
}

/// Whether `c` is a comma or a parenthesis: these separate a TIN's
/// *groups* (`123,45,6789`, `(123) 45-6789`) and never an ungrouped run,
/// so an amount's thousands separators (`1,234,567.89`, 1-3-3-2) are not
/// a TIN ([`grouped_tin_runs`]).
fn grouped_tin_separator(c: char) -> bool {
    matches!(c, ',' | '(' | ')')
}

/// The gap from `chars[at..]` between two digits: how many characters it
/// takes, or None when the next thing that isn't a separator or invisible
/// isn't a digit, or it holds more than [`MAX_TIN_GAP`] separators. A run
/// of spaces ([`tin_space`]) counts once, however long; invisible
/// characters count not at all; `grouped` also reads a comma, a
/// parenthesis ([`grouped_tin_separator`]).
fn tin_gap(chars: &[char], at: usize, grouped: bool) -> Option<usize> {
    let (mut i, mut count, mut in_spaces) = (at, 0, false);
    while i < chars.len() {
        let c = chars[i];
        if tin_invisible(c) {
            i += 1;
            continue;
        }
        if tin_space(c) {
            if !in_spaces {
                count += 1;
                in_spaces = true;
            }
            i += 1;
            continue;
        }
        if tin_separator(c) || (grouped && grouped_tin_separator(c)) {
            count += 1;
            in_spaces = false;
            i += 1;
            continue;
        }
        break;
    }
    (i > at && count <= MAX_TIN_GAP && chars.get(i).is_some_and(|c| c.is_numeric()))
        .then_some(i - at)
}

/// Each run in `chars` that holds nine digits or more, as (first, last)
/// char indices of its digits: digits with at most [`MAX_TIN_GAP`]
/// separators ([`tin_separator`]) between neighbors, any run of spaces
/// counting once, and any number of invisible characters. Also every
/// 3-2-4 or 2-7 group ([`grouped_tin_runs`]), whose groups a comma or a
/// parenthesis may join.
fn full_tin_runs(chars: &[char]) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_numeric() {
            i += 1;
            continue;
        }
        let (mut digits, mut last, mut j) = (1, i, i + 1);
        while j < chars.len() {
            if chars[j].is_numeric() {
                digits += 1;
                last = j;
                j += 1;
                continue;
            }
            match tin_gap(chars, j, false) {
                Some(consumed) => j += consumed,
                None => break,
            }
        }
        if digits >= 9 {
            runs.push((i, last));
        }
        i = last + 1;
    }
    runs.extend(grouped_tin_runs(chars));
    runs.sort_unstable();
    runs
}

/// Each 3-2-4 or 2-7 group in `chars`, as (first, last) char indices of
/// its digits: a TIN's shapes (`123-45-6789`, `12-3456789`,
/// `123,45,6789`, `(123) 45-6789`). A comma or a parenthesis separates
/// these groups and no other run, so the amount `1,234,567.89` (1-3-3-2)
/// is no TIN however it is grouped.
fn grouped_tin_runs(chars: &[char]) -> Vec<(usize, usize)> {
    const SHAPES: [&[usize]; 2] = [&[3, 2, 4], &[2, 7]];
    let mut runs = Vec::new();
    for at in 0..chars.len() {
        if !chars[at].is_numeric() {
            continue;
        }
        'shapes: for shape in SHAPES {
            let mut i = at;
            for (n, &len) in shape.iter().enumerate() {
                if n > 0 {
                    match tin_gap(chars, i, true) {
                        Some(consumed) => i += consumed,
                        None => continue 'shapes,
                    }
                }
                for _ in 0..len {
                    if !chars.get(i).is_some_and(|c| c.is_numeric()) {
                        continue 'shapes;
                    }
                    i += 1;
                }
            }
            runs.push((at, i - 1));
        }
    }
    runs
}

/// Whether `text` holds a run of nine digits, with at most three
/// separators (a space, hyphen, dot, slash or underscore, in any of
/// their Unicode forms, and any run of spaces counting once) between
/// neighbors, invisible characters not counted, or one of a TIN's groups
/// (3-2-4 or 2-7) with a comma or a parenthesis between them: an SSN,
/// EIN, or ITIN however it's written (`291-12-2846`, `291.12.2846`,
/// `291 - 12 - 2846`, `291,12,2846`, `(291) 12-2846`, `２９１－１２－２８４６`).
/// Any Unicode numeral counts as a digit (full-width, Arabic-Indic, ...),
/// so a number can't slip through in another script. In ASCII, the space,
/// hyphen, dot, slash and underscore separate, and so do commas and
/// parentheses between a TIN's groups: a comma-grouped amount
/// (`1,234,567.89`) is no TIN.
pub fn has_full_tin(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    if REPLAYING.with(std::cell::Cell::get) {
        return !journaled_tin_runs(&chars).is_empty();
    }
    !full_tin_runs(&chars).is_empty()
}

std::thread_local! {
    /// Set while a journaled line is replayed ([`replaying`]).
    static REPLAYING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `f` as the replay of a journaled line: within it,
/// [`has_full_tin`] (and so [`check_text`] and the facts' checks) finds a
/// TIN as the version before P3 did, never more.
///
/// ```text
/// GOAL: books an earlier version wrote open, forever, however a later
///       version widens what reads as a full TIN.
/// SOURCE: rules.md B-28 ("Replay accepts every line an earlier version
///         accepted": the bounds are the commands', never replay's);
///         B-26 (no full TIN in the books).
/// INVARIANTS:
///   1. Within it, a text is refused only if P2's detector
///      ([`journaled_tin_runs`]) refuses it: every name, reason and fact
///      P2 took replays.
///   2. Outside it (every command), the detector is P3's, which finds
///      everything P2's did and more (Unicode spaces and dashes,
///      invisible characters).
///   3. Restored on the way out, a panic included.
/// SCENARIOS:
///   1. IF a P2 journal holds a vendor named with a fullwidth-dash run of
///      nine digits: SHOULD replay (P3 review, 2026-10-21: P3 refused to
///      open such books).
/// ```
pub fn replaying<T>(f: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            REPLAYING.with(|r| r.set(self.0));
        }
    }
    let _restore = Restore(REPLAYING.with(|r| r.replace(true)));
    f()
}

/// Whether a journaled line is being replayed now ([`replaying`]): the
/// rules a later version added that refuse new work (the control guard,
/// a document's bounds, rules.md B-38) never refuse a line an earlier
/// version accepted (B-28).
pub fn is_replaying() -> bool {
    REPLAYING.with(std::cell::Cell::get)
}

/// The separators P2 (and every version before P3) read between a TIN's
/// digits: space, hyphen, dot, slash, underscore, the no-break space and
/// the dashes U+2010 to U+2014 and U+2212.
fn journaled_tin_separator(c: char) -> bool {
    matches!(
        c,
        ' ' | '-' | '.' | '/' | '_' | '\u{a0}' | '\u{2010}'..='\u{2014}' | '\u{2212}'
    )
}

/// [`full_tin_runs`] as P2 found them: [`journaled_tin_separator`] only,
/// one to [`MAX_TIN_GAP`] between neighbors, no invisible characters.
fn journaled_tin_runs(chars: &[char]) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_numeric() {
            i += 1;
            continue;
        }
        let (mut digits, mut last, mut j) = (1, i, i + 1);
        while j < chars.len() {
            if chars[j].is_numeric() {
                digits += 1;
                last = j;
                j += 1;
                continue;
            }
            let gap = chars[j..]
                .iter()
                .take_while(|c| journaled_tin_separator(**c))
                .count();
            if (1..=MAX_TIN_GAP).contains(&gap)
                && chars.get(j + gap).is_some_and(|c| c.is_numeric())
            {
                j += gap;
            } else {
                break;
            }
        }
        if digits >= 9 {
            runs.push((i, last));
        }
        i = last + 1;
    }
    runs
}

/// `text` with every run [`has_full_tin`] finds masked: each of its
/// digits becomes `•`, separators kept. For refusals that quote what they
/// were given, so no error echoes a full TIN (docs/spec/rules.md B-26).
pub fn redact_full_tins(text: &str) -> std::borrow::Cow<'_, str> {
    let mut chars: Vec<char> = text.chars().collect();
    let runs = full_tin_runs(&chars);
    if runs.is_empty() {
        return std::borrow::Cow::Borrowed(text);
    }
    for (first, last) in runs {
        for c in &mut chars[first..=last] {
            if c.is_numeric() {
                *c = '•';
            }
        }
    }
    std::borrow::Cow::Owned(chars.into_iter().collect())
}

/// Check one party text field: not a full TIN, at most 500 characters, no
/// control characters except newline (addresses and notes run several
/// lines). Returns why not.
pub fn check_text(field: &str, text: &str) -> Result<(), String> {
    if has_full_tin(text) {
        return Err(format!(
            "{field} looks like it holds a full taxpayer identification number; \
             the books keep only its last four digits (the full number belongs in the vault)"
        ));
    }
    if text.chars().count() > 500 {
        return Err(format!("{field} is at most 500 characters"));
    }
    if text.chars().any(|c| c.is_control() && c != '\n') {
        return Err(format!("{field} can't contain control characters"));
    }
    Ok(())
}

/// Check an email address: one line of party text ([`check_text`], which
/// refuses a full TIN) in a plain address shape, one `@` and no spaces. Not
/// an RFC grammar test: the books only print it, they never send (A-4).
pub fn check_email(field: &str, text: &str) -> Result<(), String> {
    check_text(field, text)?;
    if text.chars().filter(|c| *c == '@').count() != 1 || text.chars().any(char::is_whitespace) {
        return Err(format!(
            "{field} {text:?}: expected one address, like billing@example.com"
        ));
    }
    Ok(())
}

impl W9Facts {
    /// Check the facts' form. Returns why not.
    pub fn check(&self) -> Result<(), String> {
        if self.tin_last4.len() != 4 || !self.tin_last4.chars().all(|c| c.is_ascii_digit()) {
            return Err("tin_last4 is exactly four digits".into());
        }
        if self.entity_type.trim().is_empty() {
            return Err("a W-9's entity type (line 3) is required".into());
        }
        if self.kept_at.trim().is_empty() {
            return Err("say where the W-9 itself is kept".into());
        }
        for (f, t) in [
            ("entity_type", Some(self.entity_type.as_str())),
            ("exempt_payee_code", self.exempt_payee_code.as_deref()),
            ("fatca_code", self.fatca_code.as_deref()),
            ("kept_at", Some(self.kept_at.as_str())),
        ] {
            if let Some(t) = t {
                check_text(f, t)?;
            }
        }
        if let Some(h) = &self.vault_record
            && (h.len() != 64 || !h.chars().all(|c| c.is_ascii_hexdigit()))
        {
            return Err("vault_record is a SHA-256 in hex".into());
        }
        Ok(())
    }
}

impl W4Facts {
    /// Check the facts' form. Returns why not.
    pub fn check(&self) -> Result<(), String> {
        if self.state.len() != 2 || !self.state.chars().all(|c| c.is_ascii_uppercase()) {
            return Err("state is two capital letters, e.g. UT".into());
        }
        if self.kept_at.trim().is_empty() {
            return Err("say where the W-4 itself is kept".into());
        }
        check_text("kept_at", &self.kept_at)
    }
}

/// One fact of a party to change ([`crate::Ledger::update_party`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "field", content = "value")]
pub enum PartyField {
    /// The legal or trade name.
    Name(String),
    /// The display name (`None` clears it).
    DisplayName(Option<String>),
    /// The website.
    Website(Option<String>),
    /// The business address.
    Address(Option<String>),
    /// Notes.
    Notes(Option<String>),
    /// Archived (no longer dealt with) or not.
    Archived(bool),
    /// The email address (A-13; `None` clears it).
    Email(Option<String>),
    /// The terms it grants the company as a customer (A-13; `None` clears).
    DefaultTermsAsCustomer(Option<crate::Terms>),
    /// The terms it gives the company as a vendor (A-13; `None` clears).
    DefaultTermsAsVendor(Option<crate::Terms>),
}

impl PartyField {
    /// Every field's wire name, as `party.updated` journals it.
    pub const FIELDS: [&'static str; 9] = [
        "name",
        "display_name",
        "website",
        "address",
        "notes",
        "archived",
        "email",
        "default_terms_as_customer",
        "default_terms_as_vendor",
    ];

    /// Whether `field` is a field this version knows. A `party.updated`
    /// naming another was written by a newer version (rules.md B-27).
    pub fn known_field(field: &str) -> bool {
        Self::FIELDS.contains(&field)
    }

    /// Whether the field needs the `documents_v2` feature: the three A-13
    /// facts an older version would drop.
    pub fn needs_documents_v2(&self) -> bool {
        matches!(
            self,
            PartyField::Email(_)
                | PartyField::DefaultTermsAsCustomer(_)
                | PartyField::DefaultTermsAsVendor(_)
        )
    }
}

/// Which of a party's totals to report (rules.md B-35 item 4). Not the
/// books' accounting method (`books.accounting_basis`): party totals count
/// what moved, whichever method the books keep.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Basis {
    /// Money: through the company's cash accounts, a card, the members'
    /// financing or a loan; on a member's capital, what the K-1 counts as
    /// cash. The default.
    #[default]
    Cash,
    /// Money and property contributed or distributed in kind: for a
    /// member, what the statement of members' equity counts.
    All,
}

impl Basis {
    /// Every basis, the default first.
    pub const ALL: [Basis; 2] = [Basis::Cash, Basis::All];

    /// Its wire name: `cash` or `all`.
    pub fn key(self) -> &'static str {
        match self {
            Basis::Cash => "cash",
            Basis::All => "all",
        }
    }

    /// The basis named `key` (surrounding space ignored); None for any
    /// other word.
    pub fn parse(key: &str) -> Option<Basis> {
        Basis::ALL.into_iter().find(|b| b.key() == key.trim())
    }
}

/// What the books did with one party over a span of dates
/// ([`activity_with`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Activity {
    /// The party's entries, drafts included, by id.
    pub entries: BTreeSet<crate::EntryId>,
    /// Paid to it in money in the span (basis cash), net of reversals of
    /// payments (a refund recorded as its own entry is received from it).
    pub paid: rust_decimal::Decimal,
    /// Received from it in money in the span (basis cash), net of
    /// reversals of receipts.
    pub received: rust_decimal::Decimal,
    /// Given to it as property in the span (a distribution in kind), net
    /// of reversals: basis all adds it to `paid`.
    pub paid_in_kind: rust_decimal::Decimal,
    /// Received from it as property in the span (a contribution in kind),
    /// net of reversals: basis all adds it to `received`.
    pub received_in_kind: rust_decimal::Decimal,
    /// Posted entries in the span naming it whose shares don't resolve
    /// against the entry and the members given ([`crate::shares::resolve`]),
    /// or whose part is past exact arithmetic: counted in no total.
    pub unresolved: BTreeSet<crate::EntryId>,
}

impl Activity {
    /// (paid, received) on `basis`: cash is `paid` and `received`; all
    /// adds what moved in kind. [`activity_with`] admits an entry only when
    /// every one of these sums stays exact, so this never overflows for an
    /// activity it made.
    pub fn totals(&self, basis: Basis) -> (rust_decimal::Decimal, rust_decimal::Decimal) {
        match basis {
            Basis::Cash => (self.paid, self.received),
            Basis::All => (
                self.paid
                    .checked_add(self.paid_in_kind)
                    .expect("activity_with keeps the all-basis sums exact"),
                self.received
                    .checked_add(self.received_in_kind)
                    .expect("activity_with keeps the all-basis sums exact"),
            ),
        }
    }
}

/// The accounts [`entry_flow`] reads money from (rules.md B-35, defect
/// 24): the cash accounts (every asset account but receivables when none
/// are given), and the money liabilities: the cards ([`card_accounts`]),
/// what the members advanced and loans (`books.financing_accounts`).
/// Never payables, the payroll withholdings, or any other liability (an
/// accrued expense, deferred revenue): an accrual is a promise, counted
/// when it is settled, never also when it is booked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MoneyAccounts {
    /// Cash: bank, savings, cash on hand.
    pub cash: std::collections::HashSet<crate::AccountId>,
    /// Liabilities that pay for the company or are paid down.
    pub liabilities: std::collections::HashSet<crate::AccountId>,
}

impl MoneyAccounts {
    /// These accounts less every liability owed to one of `parties`
    /// ([`Party::creditor_of`]): for an entry naming them, a credit to
    /// what the company owes a party itself never counts as paying it (a
    /// fee charged on its own card, interest added to its own loan,
    /// equipment bought on its own note, a distribution still owed to a
    /// member), and a debit to it is what the money paid for (rules.md
    /// B-35; DECISION, owner, 2026-10-20).
    pub fn for_parties(
        &self,
        ledger: &crate::Ledger,
        parties: &[PartyId],
    ) -> std::borrow::Cow<'_, MoneyAccounts> {
        let owed: Vec<crate::AccountId> = parties
            .iter()
            .filter_map(|p| ledger.get_party(*p))
            .flat_map(|p| p.creditor_of.iter().copied())
            .filter(|a| self.liabilities.contains(a))
            .collect();
        if owed.is_empty() {
            return std::borrow::Cow::Borrowed(self);
        }
        let mut out = self.clone();
        for a in owed {
            out.liabilities.remove(&a);
        }
        std::borrow::Cow::Owned(out)
    }
}

/// The company's cards: the one rule for what a card is, which party
/// money, the monthly reconciliation reminders
/// (`ledger_store::reconcilable`) and the viewer all read, so none
/// disagrees on it.
///
/// ```text
/// GOAL: a card is known from what the books record, and a promise (a
///       gift card sold) or a member's own card is never the company's.
/// SOURCE: rules.md B-35 item 5 (DECISION, owner, 2026-10-20: money is
///         cash, cards, the members' financing and loans), B-19 (payment
///         methods, plan P3 defect 12).
/// INVARIANTS:
///   1. When the books record a business-card payment method: exactly
///      the liability accounts of those methods
///      (crate::Ledger::card_accounts), whatever the accounts are named.
///   2. Else (books from before payment methods, or that never record
///      one; CONVENTION): the liability accounts whose names say "card"
///      (ignoring case) but not "gift" (gift cards the company sold are
///      deferred revenue it owes its customers, a promise, not a card it
///      pays with).
///   3. Never the account of a member's card, a member's cash or check,
///      or a payment platform: a member's card pays through the members'
///      financing account, a platform through its own balance.
///   4. In chart order (ascending account id), each once.
/// ```
pub fn card_accounts(ledger: &crate::Ledger) -> Vec<crate::AccountId> {
    let recorded = ledger.card_accounts();
    if !recorded.is_empty() {
        return recorded.into_iter().collect();
    }
    let not_cards: std::collections::BTreeSet<crate::AccountId> = ledger
        .instruments()
        .filter(|i| i.kind != crate::InstrumentKind::BusinessCard)
        .map(|i| i.account)
        .collect();
    let mut cards: Vec<crate::AccountId> = ledger
        .chart()
        .all()
        .filter(|a| {
            let name = a.name.to_lowercase();
            a.account_type == crate::AccountType::Liability
                && name.contains("card")
                && !name.contains("gift")
                && !not_cards.contains(&a.id)
        })
        .map(|a| a.id)
        .collect();
    cards.sort_unstable();
    cards.dedup();
    cards
}

/// The [`MoneyAccounts`] of `ledger`: `cash` the books' cash accounts
/// (every asset account but receivables when empty), `financing` the
/// books' financing liabilities (`books.financing_accounts`: what the
/// members advanced, loans), with the cards ([`card_accounts`]).
pub fn money_accounts(
    ledger: &crate::Ledger,
    cash: &[crate::AccountId],
    financing: &[crate::AccountId],
) -> MoneyAccounts {
    use crate::AccountType;
    let not_money: std::collections::HashSet<crate::AccountId> = crate::PAYROLL[2..]
        .iter()
        .chain([&crate::PAYABLES, &crate::RECEIVABLES])
        .filter_map(|c| ledger.role_account(c))
        .collect();
    let cards = card_accounts(ledger);
    let mut out = MoneyAccounts::default();
    for a in ledger.chart().all().filter(|a| !not_money.contains(&a.id)) {
        match a.account_type {
            AccountType::Asset if cash.is_empty() || cash.contains(&a.id) => {
                out.cash.insert(a.id);
            }
            AccountType::Liability if cards.contains(&a.id) || financing.contains(&a.id) => {
                out.liabilities.insert(a.id);
            }
            _ => {}
        }
    }
    out
}

/// The original a reversal chain starts from, and how many reversals lie
/// between it and `entry` (zero when `entry` reverses nothing): followed
/// up `reversal_of`, O(depth). Ids strictly fall along the chain (a
/// reversal is made after its original), so the walk always ends.
pub fn root_original<'a>(
    ledger: &'a crate::Ledger,
    entry: &'a crate::JournalEntry,
) -> (&'a crate::JournalEntry, usize) {
    let mut root = entry;
    let mut depth = 0;
    while let Some(o) = root.reversal_of() {
        match ledger.get_entry(o) {
            Some(orig) if orig.id() < root.id() => {
                root = orig;
                depth += 1;
            }
            _ => break,
        }
    }
    (root, depth)
}

/// What one entry moved with its counterparty ([`entry_flow`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntryFlow {
    /// Paid to the counterparty (positive) or received from it (negative).
    pub net: rust_decimal::Decimal,
    /// What for, by account (debits − credits): positive is what the money
    /// paid for, negative where money received came from. Sums to `net`.
    pub what_for: Vec<(crate::AccountId, rust_decimal::Decimal)>,
}

/// The money one entry moved with its counterparty, and what for.
///
/// ```text
/// GOAL: "how much did we pay them" right however it was paid: from the
///       bank, on a card, by a member for the company, or by paying down
///       what the company owes them (a reimbursement, a card payment).
/// SOURCE: DECISION (owner, 2026-10-15), after a reimbursement to the
///         members netted to zero under the earlier rule (cash and
///         liabilities alike as money).
/// DEFINITIONS (over the entry's lines, in `money`):
///   cash_cr, cash_dr: credits and debits on cash accounts;
///   liab_cr, liab_dr: credits and debits on money liabilities;
///   down = min(liab_dr, cash_cr): cash paying down a liability (its
///     creditor is paid; the liability is what for);
///   up = min(liab_cr, cash_dr): a liability raising cash (borrowing: the
///     lender pays the company);
///   left(a), for each money liability a: its debits less what of them
///     `down` takes, less its credits less what of them `up` takes (`down`
///     and `up` taken from its lines in line order), so one account's
///     debits and credits offset first;
///   across = min(Σ left(a) > 0, Σ −left(a) < 0): one liability paying
///     down another (a member paying the card from their own funds: Dr
///     2050 / Cr 2100; a loan paying off a card): the debited liability's
///     creditor is paid, by the credited one's.
/// INVARIANTS:
///   1. net = (cash_cr − cash_dr) + (liab_cr − liab_dr) + down − up
///      + across.
///   2. what_for: every line not on a money account (debits − credits),
///      plus `down` on the debited liabilities and −`up` on the credited
///      ones (in line order), plus `across` on the liabilities with
///      left(a) > 0 (in order of first appearance), zero sums left out,
///      accounts merged; its sum is net exactly, because the entry
///      balances.
/// SCENARIOS:
///   1. Expense from the bank, on a card, or paid by a member for the
///      company (Dr expense / Cr 1000, 2050, or 2100): net is the amount.
///   2. Reimbursing members (Dr 2100 / Cr 1000) or paying a card (Dr 2050 /
///      Cr 1000): net is the amount, what for is the liability.
///   3. A loan (Dr 1000 / Cr loan): net is minus the amount (received).
///   4. A refund (Dr 1000 or Dr card / Cr expense): negative net. A charge
///      and a refund on the same card in one entry net on that card: no
///      card paid down.
///   5. A transfer between the company's own cash accounts: zero.
///   6. A member contributing property (Dr equipment / Cr capital): zero.
///   7. A member paying the card bill from their own funds (Dr 2050 / Cr
///      2100), or a loan paying it off (Dr 2050 / Cr loan): net is the
///      amount, what for is the card, as if paid from the bank.
///   8. An accrual through a liability that isn't money (Dr expense / Cr
///      accrued expenses): zero; its settlement (Dr accrued / Cr 1000) is
///      the payment (defect 24: counted once, not twice).
/// Line-level: not odd for one liability paying down another (a
/// reversal of SCENARIO 7 reads as another payment, `across` again), so a
/// party's figures read a reversal through [`flow`], never its lines.
/// ```
pub fn entry_flow(
    entry: &crate::JournalEntry,
    money: &MoneyAccounts,
) -> Result<EntryFlow, crate::LedgerError> {
    use crate::Side;
    use rust_decimal::Decimal;
    // A draft's lines are as journaled, of any size (rules.md B-28): every
    // sum is checked, and one past exact arithmetic is refused.
    let too_large =
        || crate::LedgerError::TooLargeToCompute(format!("entry {}'s money flow", entry.id()));
    let plus = |a: Decimal, b: Decimal| a.checked_add(b).ok_or_else(too_large);
    let minus = |a: Decimal, b: Decimal| a.checked_sub(b).ok_or_else(too_large);
    let (mut cash_cr, mut cash_dr, mut liab_cr, mut liab_dr) =
        (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
    for l in entry.lines() {
        let a = l.amount.amount;
        match (
            money.cash.contains(&l.account),
            money.liabilities.contains(&l.account),
            l.side,
        ) {
            (true, _, Side::Credit) => cash_cr = plus(cash_cr, a)?,
            (true, _, Side::Debit) => cash_dr = plus(cash_dr, a)?,
            (false, true, Side::Credit) => liab_cr = plus(liab_cr, a)?,
            (false, true, Side::Debit) => liab_dr = plus(liab_dr, a)?,
            _ => {}
        }
    }
    let down = liab_dr.min(cash_cr);
    let up = liab_cr.min(cash_dr);
    let mut by: std::collections::BTreeMap<crate::AccountId, Decimal> = Default::default();
    let (mut down_left, mut up_left) = (down, up);
    let mut order: Vec<crate::AccountId> = Vec::new();
    // left(a) per money liability, in order of first appearance.
    let mut left: Vec<(crate::AccountId, Decimal)> = Vec::new();
    let add = |by: &mut std::collections::BTreeMap<crate::AccountId, Decimal>,
               order: &mut Vec<crate::AccountId>,
               account: crate::AccountId,
               part: Decimal|
     -> Result<(), crate::LedgerError> {
        if !part.is_zero() {
            if !by.contains_key(&account) {
                order.push(account);
            }
            let sum = by.entry(account).or_default();
            *sum = plus(*sum, part)?;
        }
        Ok(())
    };
    for l in entry.lines() {
        let a = l.amount.amount;
        let part = if money.cash.contains(&l.account) {
            Decimal::ZERO
        } else if money.liabilities.contains(&l.account) {
            let (part, rest) = match l.side {
                Side::Debit => {
                    let take = a.min(down_left);
                    down_left -= take;
                    (take, a - take)
                }
                Side::Credit => {
                    let take = a.min(up_left);
                    up_left -= take;
                    (-take, -(a - take))
                }
            };
            match left.iter_mut().find(|(x, _)| *x == l.account) {
                Some((_, v)) => *v = plus(*v, rest)?,
                None => left.push((l.account, rest)),
            }
            part
        } else {
            match l.side {
                Side::Debit => a,
                Side::Credit => -a,
            }
        };
        add(&mut by, &mut order, l.account, part)?;
    }
    let owed_down = left
        .iter()
        .map(|(_, v)| *v)
        .filter(|v| *v > Decimal::ZERO)
        .try_fold(Decimal::ZERO, &plus)?;
    let owed_up = -left
        .iter()
        .map(|(_, v)| *v)
        .filter(|v| *v < Decimal::ZERO)
        .try_fold(Decimal::ZERO, &plus)?;
    let across = owed_down.min(owed_up);
    let mut across_left = across;
    for (account, v) in left {
        if v > Decimal::ZERO && !across_left.is_zero() {
            let take = v.min(across_left);
            across_left -= take;
            add(&mut by, &mut order, account, take)?;
        }
    }
    let net = plus(
        plus(minus(cash_cr, cash_dr)?, minus(liab_cr, liab_dr)?)?,
        plus(minus(down, up)?, across)?,
    )?;
    Ok(EntryFlow {
        net,
        what_for: order
            .into_iter()
            .map(|a| (a, by[&a]))
            .filter(|(_, v)| !v.is_zero())
            .collect(),
    })
}

/// The money an entry moved with the parties it names, and what for,
/// read as party figures read it: a reversal's is exactly its original's
/// negated (B-9), never re-read from its own lines; and a liability owed
/// to a party named is not money for the entry
/// ([`MoneyAccounts::for_parties`]).
///
/// ```text
/// GOAL: what a party's entry paid for, right for reversals too.
/// SOURCE: rules.md B-9 ("a mistake is corrected by an opposite entry"),
///         B-35.
/// INVARIANTS:
///   1. An entry reversing none: entry_flow of its lines, with
///      money.for_parties(the parties of `shares`).
///   2. flow(reversal) = −flow(its original): net and every what_for
///      negated, accounts in the same order; a reversal of a reversal
///      gives the original's back. Found up reversal_of (root_original),
///      O(depth).
/// SCENARIOS:
///   1. A member pays the card from their own funds (Dr 2050 / Cr 2100,
///      tagged with the issuer): +100.00, for 2050. Its reversal (Dr 2100
///      / Cr 2050) is −100.00, though entry_flow reads its lines as
///      another +100.00 (the across term isn't odd).
/// ```
pub fn flow(
    ledger: &crate::Ledger,
    id: crate::EntryId,
    shares: &[crate::PartyShare],
    money: &MoneyAccounts,
) -> Result<EntryFlow, crate::LedgerError> {
    let entry = ledger
        .get_entry(id)
        .ok_or(crate::LedgerError::UnknownEntry(id))?;
    let (root, depth) = root_original(ledger, entry);
    let named: Vec<PartyId> = shares.iter().map(|s| s.party).collect();
    let f = entry_flow(root, &money.for_parties(ledger, &named))?;
    if depth % 2 == 0 {
        return Ok(f);
    }
    // Every figure a flow holds is nonzero or a plain zero, and a
    // Decimal's negation is exact.
    let neg = |v: rust_decimal::Decimal| {
        if v.is_zero() {
            rust_decimal::Decimal::ZERO
        } else {
            -v
        }
    };
    Ok(EntryFlow {
        net: neg(f.net),
        what_for: f.what_for.into_iter().map(|(a, v)| (a, neg(v))).collect(),
    })
}

/// One party's entries, and what was paid to and received from it in
/// `start..=end`, with no members ([`activity_with`] with none: an entry
/// naming several parties then needs explicit shares to count).
pub fn activity(
    ledger: &crate::Ledger,
    party: PartyId,
    span: (NaiveDate, NaiveDate),
    money: &MoneyAccounts,
) -> Activity {
    activity_with(ledger, party, span, money, &[])
}

/// One party's entries, and its part of what was paid to and received
/// from the entries' parties in `start..=end`, on both bases.
///
/// ```text
/// GOAL: one answer to "what did we pay them this year, and what did we
///       get from them?" for the MCP server and the viewer alike, right
///       when an entry was with several parties, for property as well as
///       money, and for reversals.
/// SOURCE: docs/parties-and-attachments.md (Parties, Shares); rules.md
///         B-35 (DECISION, owner, 2026-10-15 and 2026-10-20).
/// INVARIANTS:
///   1. entries: every entry Ledger::entry_parties names the party in
///      (hand tags, vendor tags, bills and their payments, invoices and
///      their payments, and reversals of any of these carry it).
///   2. Each posted entry dated in the span counts the party's part of it
///      (crate::shares::resolve with `money` and `members`): its cash in
///      paid or received, its in_kind in paid_in_kind or received_in_kind,
///      by the direction of the entry's root original (Target::counts_as),
///      so a reversal subtracts on its original's side, and a reversal of
///      that reversal adds back there.
///   3. An entry whose target is direction neither at its root (a bill,
///      an invoice, a close, an accrual, a fee on the party's own card),
///      or whose part counts nothing (a distribution still owed), counts
///      nothing and is never unresolved: its target is found before its
///      split.
///   4. An entry whose target or split is refused (shares that don't fit,
///      a sum past exact arithmetic), or whose part would take any total,
///      on either basis, past exact arithmetic, counts nothing and is
///      listed in `unresolved`.
///   5. Over all the parties an entry names, the parts counted sum to its
///      target's cash and in_kind exactly (crate::shares INVARIANT 6).
///   6. A card charge counts as paid (the company paid them, by card); a
///      bill doesn't until it is paid; a member's property contribution is
///      received in kind. Not the 1099 amount: that is
///      form1099::vendor_1099_report.
/// ```
pub fn activity_with(
    ledger: &crate::Ledger,
    party: PartyId,
    (start, end): (NaiveDate, NaiveDate),
    money: &MoneyAccounts,
    members: &[crate::PartyMember],
) -> Activity {
    use crate::kernel::Direction;
    let entries: BTreeSet<crate::EntryId> = (0..ledger.next_entry_id())
        .filter(|&id| ledger.entry_names_party(id, party))
        .collect();
    let mut a = Activity::default();
    for &id in &entries {
        let Some(e) = ledger.get_entry(id) else {
            continue;
        };
        if e.status() != crate::EntryStatus::Posted || e.date() < start || e.date() > end {
            continue;
        }
        let shares = ledger.entry_parties(id);
        // The target first: an entry that counts nothing is never
        // unresolved for shares that don't fit it (rules.md B-35 item 4).
        let Ok(t) = crate::shares::target(ledger, id, shares, money) else {
            a.unresolved.insert(id);
            continue;
        };
        if t.counts_as == Direction::Neither || (t.cash.is_zero() && t.in_kind.is_zero()) {
            continue;
        }
        let Ok(parts) = crate::shares::split(e, shares, &t, members) else {
            a.unresolved.insert(id);
            continue;
        };
        let Some(p) = parts.iter().find(|p| p.party == party) else {
            continue;
        };
        // Paid adds, received subtracts (a receipt is negative), each on
        // the root original's side; every total, the all-basis sums
        // included, stays exact or the entry counts nothing.
        let next = match t.counts_as {
            Direction::Paid => a
                .paid
                .checked_add(p.cash)
                .zip(a.paid_in_kind.checked_add(p.in_kind))
                .map(|(c, k)| (c, k, a.received, a.received_in_kind)),
            _ => a
                .received
                .checked_sub(p.cash)
                .zip(a.received_in_kind.checked_sub(p.in_kind))
                .map(|(c, k)| (a.paid, a.paid_in_kind, c, k)),
        };
        match next.filter(|(pc, pk, rc, rk)| {
            pc.checked_add(*pk).is_some() && rc.checked_add(*rk).is_some()
        }) {
            Some((pc, pk, rc, rk)) => {
                a.paid = pc;
                a.paid_in_kind = pk;
                a.received = rc;
                a.received_in_kind = rk;
            }
            None => {
                a.unresolved.insert(id);
            }
        }
    }
    a.entries = entries;
    a
}
