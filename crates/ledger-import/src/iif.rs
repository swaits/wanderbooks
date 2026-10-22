//! Generate Intuit Interchange Format (IIF) journal entries for QuickBooks
//! Desktop, which many CPAs still run.
//!
//! ```text
//! GOAL: hand a CPA's QuickBooks Desktop the posted entries, landing on the
//!       right accounts, with nothing invented and nothing changed without
//!       saying so.
//! SOURCE: docs/spec/sources/intuit-iif.md (Intuit's IIF Overview, the IIF
//!         Import Kit's header help and its 2007+ General Journal example);
//!         rules.md B-34; Intuit's "Character limitations for fields in
//!         QuickBooks" (account name 31, account number 7, memo 4,095);
//!         DECISION (owner, 2026-10-20, plan D-36: plain ASCII,
//!         transliterated and listed; CR LF line endings); the kit's
//!         `!CLASS` header help and its list declarations example (a
//!         class's NAME; lists before transactions); a class name of 31
//!         characters at most (Intuit's qbXML Onscreen Reference,
//!         `ClassAdd` Name).
//! INVARIANTS:
//!   1. Accounts MUST be declared first (`!ACCNT`: NAME, ACCNTTYPE, ACCNUM),
//!      each after its parent, BECAUSE QuickBooks creates an unknown
//!      account "as a new bank account", and the kit says lists come
//!      before what uses them.
//!   2. A subaccount's NAME MUST be its path, highest level first, joined by
//!      colons; a colon inside one account's own name is replaced, or it
//!      would read as another level. Each level is trimmed, "?" if nothing
//!      is left, and at most [`NAME_MAX`] (31) characters, cut there:
//!      QuickBooks Desktop holds no longer account name (Intuit's limits),
//!      and a longer one, cut when its chart is imported but not in its
//!      transactions (a QuickBooks reseller's note), would miss them. A
//!      subaccount MUST have its parent's type ("The parent and subaccount
//!      must be the same account type", Intuit), so one whose type differs
//!      is written at the top level, its own subaccounts with it.
//!   3. Each transaction is a TRNS line, its SPL lines, then ENDTRNS, all
//!      `GENERAL JOURNAL`, dated M/D/YYYY (the kit's 2007+ example), with
//!      debits positive and credits negative, so the lines sum to zero.
//!      DOCNUM ("the transaction's number") is the entry's id in the books,
//!      on the TRNS and every SPL, so the CPA can trace it. A MEMO is at
//!      most [`MEMO_MAX`] (4,095) characters, cut there.
//!   4. The file is ASCII text (IIF "is ASCII text") and every line ends
//!      CR LF (the kit's example file). A field holds no tab or line break
//!      (no escape exists): each becomes a space, any other control
//!      character '?', a double quote (QuickBooks reads it as a text
//!      delimiter) a single one, and a character past ASCII its
//!      transliteration (deunicode: "René" is "Rene", "北京" is "Bei Jing"),
//!      or '?' when it has none.
//!   5. NEVER write an entry whose account wasn't declared: refuse instead.
//!   6. NEVER give two accounts one NAME (QuickBooks identifies an account
//!      by its NAME, so it would merge them): [`iif_accounts`] follows the
//!      own name of each account whose path would match another's,
//!      ignoring case, with " (its code)", until none matches, and
//!      [`journal_to_iif`] refuses a match that remains. NEVER give two
//!      accounts one ACCNUM, nor one longer than [`NUMBER_MAX`] (7, QuickBooks
//!      Desktop's longest): such a number is left out (ACCNUM is optional;
//!      the NAME identifies the account).
//!   7. Nothing changes silently: every account name, number or place and
//!      every memo the file writes differently from the books is in
//!      [`Iif::changes`], and [`Iif::warnings`] says each, one line each
//!      (the memos grouped by what is written differently), never with a
//!      full TIN in it.
//!   8. The file names no customer or vendor (no NAME column): Intuit
//!      states no such need for an IIF journal, though QuickBooks Desktop
//!      asks for one on a journal entry to accounts receivable or payable
//!      and takes one such account a transaction (Intuit staff and help;
//!      sources/intuit-iif.md). [`Iif::receivable_payable`] lists the
//!      entries with such a line, for the human to be told.
//!   9. Each line's class (a project) MUST be in its CLASS field, and the
//!      classes declared first (`!CLASS`: NAME), after the accounts and
//!      before the transactions, BECAUSE the kit puts every list before
//!      what uses it (a name it doesn't know lands on the wrong list or
//!      takes a default). A class's NAME is written as an account's own
//!      level is (INVARIANTS 2, 4: plain ASCII, a colon a '-' since a
//!      colon starts a subclass, trimmed, at most [`NAME_MAX`]); classes
//!      whose NAMEs would match ignoring case get " (their id)" until none
//!      does, as accounts get their codes, except that the names already
//!      written differently yield first (another id each time need be): a name
//!      written as the books have it is kept unless another class's own
//!      name is the same ignoring case. NEVER write a line whose class
//!      wasn't declared: refuse instead. Books with no class write no
//!      `!CLASS` list, as before.
//! ```
//!
//! Layout (each `→` is a tab; every line ends CR LF):
//!
//! ```text
//! !ACCNT→NAME→ACCNTTYPE→ACCNUM
//! ACCNT→Cash→BANK→1000
//! !CLASS→NAME
//! CLASS→Fern & Field rebrand
//! !TRNS→TRNSID→TRNSTYPE→DATE→ACCNT→CLASS→AMOUNT→DOCNUM→MEMO
//! !SPL→SPLID→TRNSTYPE→DATE→ACCNT→CLASS→AMOUNT→DOCNUM→MEMO
//! !ENDTRNS
//! TRNS→→GENERAL JOURNAL→1/15/2026→Cash→→1000.00→1→Sale
//! SPL→→GENERAL JOURNAL→1/15/2026→Revenue→Fern & Field rebrand→-1000.00→1→Sale
//! ENDTRNS
//! ```
//!
//! (DOCNUM 1: the sale is entry 1 in its books; its revenue line carries
//! the project.)

use chrono::Datelike;
use deunicode::AsciiChars;
use ledger_core::parties::{has_full_tin, redact_full_tins};
use ledger_core::{AccountId, ClassId, EntryId, JournalEntry, Ledger, Side};
use rust_decimal::Decimal;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use thiserror::Error;

/// How every line of the file ends (INVARIANT 4: CR LF, as Intuit's own
/// example file).
const EOL: &str = "\r\n";

/// The most characters of an account's own name, one level of its NAME,
/// that QuickBooks Desktop holds (Intuit, "Character limitations for
/// fields in QuickBooks": Account Name 31; sources/intuit-iif.md).
pub const NAME_MAX: usize = 31;

/// The most characters of an account number (ACCNUM) QuickBooks Desktop
/// holds (the same table: Account Numbers 7).
pub const NUMBER_MAX: usize = 7;

/// The most characters of a memo QuickBooks Desktop holds (the same
/// table: Memo 4,095).
pub const MEMO_MAX: usize = 4095;

/// A QuickBooks account type: the IIF `ACCNTTYPE` keywords (Intuit's IIF
/// Import Kit, `!ACCNT` header help). Wanderbooks' five types don't say
/// which of these an account is (checking is `BANK`, not "other current
/// asset"), so the caller decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QbAccountType {
    /// `BANK`: checking or savings.
    Bank,
    /// `AR`: accounts receivable.
    AccountsReceivable,
    /// `OCASSET`: other current asset.
    OtherCurrentAsset,
    /// `FIXASSET`: fixed asset.
    FixedAsset,
    /// `OASSET`: other asset.
    OtherAsset,
    /// `AP`: accounts payable.
    AccountsPayable,
    /// `CCARD`: credit card.
    CreditCard,
    /// `OCLIAB`: other current liability.
    OtherCurrentLiability,
    /// `LTLIAB`: long term liability.
    LongTermLiability,
    /// `EQUITY`: capital/equity.
    Equity,
    /// `INC`: income.
    Income,
    /// `COGS`: cost of goods sold.
    CostOfGoodsSold,
    /// `EXP`: expense.
    Expense,
    /// `EXINC`: other income.
    OtherIncome,
    /// `EXEXP`: other expense.
    OtherExpense,
}

impl QbAccountType {
    /// The IIF keyword.
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Bank => "BANK",
            Self::AccountsReceivable => "AR",
            Self::OtherCurrentAsset => "OCASSET",
            Self::FixedAsset => "FIXASSET",
            Self::OtherAsset => "OASSET",
            Self::AccountsPayable => "AP",
            Self::CreditCard => "CCARD",
            Self::OtherCurrentLiability => "OCLIAB",
            Self::LongTermLiability => "LTLIAB",
            Self::Equity => "EQUITY",
            Self::Income => "INC",
            Self::CostOfGoodsSold => "COGS",
            Self::Expense => "EXP",
            Self::OtherIncome => "EXINC",
            Self::OtherExpense => "EXEXP",
        }
    }
}

/// One account as QuickBooks will know it, and as the books keep it.
#[derive(Clone, Debug, PartialEq)]
pub struct IifAccount {
    /// The wanderbooks account.
    pub id: AccountId,
    /// Its NAME in the file: its own name with its parents', highest
    /// first, joined by colons, each already made safe for IIF
    /// ([`iif_accounts`] builds these).
    pub path: String,
    /// Its account number: the wanderbooks code made safe for IIF and
    /// trimmed. [`journal_to_iif`] writes it as ACCNUM, or leaves it out
    /// when QuickBooks can't hold it (INVARIANT 6).
    pub number: String,
    /// Its QuickBooks type.
    pub qb_type: QbAccountType,
    /// Its own name as the books keep it (for [`Iif::changes`]).
    pub name: String,
    /// Its code as the books keep it (for [`Iif::changes`]).
    pub code: String,
    /// Its parent in the books when the file writes it apart, at the top
    /// level, because QuickBooks would give it its parent's type
    /// (INVARIANT 2): the parent's code and QuickBooks type.
    pub apart_from: Option<(String, QbAccountType)>,
}

/// One class (a project) as QuickBooks will know it, and as the books keep
/// it (INVARIANT 9).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IifClass {
    /// The wanderbooks class.
    pub id: ClassId,
    /// Its NAME in the file, already made safe for IIF ([`iif_classes`]
    /// builds these).
    pub written: String,
    /// Its name as the books keep it (for [`Iif::changes`]).
    pub name: String,
}

/// What [`journal_to_iif`] writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Iif {
    /// The file: ASCII text, every line ending CR LF.
    pub text: String,
    /// Each account name, number or place and each memo the file writes
    /// differently from the books (INVARIANTS 7, 9), in file order: the
    /// accounts as given, then the classes, then the entries as given.
    pub changes: Vec<IifChange>,
    /// The entries with a line on an accounts receivable or accounts
    /// payable account, in file order: the file names no customer or
    /// vendor on those lines (INVARIANT 8).
    pub receivable_payable: Vec<EntryId>,
}

/// One text the file writes differently from the books.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IifChange {
    /// Which text.
    pub field: IifField,
    /// The books' text.
    pub books: String,
    /// What the file says (empty for a number left out).
    pub written: String,
}

/// Which text an [`IifChange`] is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IifField {
    /// An account's own name: its level of the NAME.
    AccountName {
        /// The account's code in the books.
        code: String,
    },
    /// An account's number (ACCNUM): its code in the books, written
    /// differently.
    AccountNumber,
    /// An account's number left out of the file: ACCNUM is blank
    /// (INVARIANT 6).
    AccountNumberLeftOut {
        /// Why.
        why: LeftOut,
    },
    /// A subaccount written at the top level, apart from its parent (the
    /// change's `books`: the parent's code), because QuickBooks gives a
    /// subaccount its parent's type and the two differ (INVARIANT 2).
    AtTopLevel {
        /// The account's code in the books.
        code: String,
        /// Its QuickBooks type.
        own: QbAccountType,
        /// Its parent's.
        parent: QbAccountType,
    },
    /// A class's name (a project's): its NAME in the `!CLASS` list and
    /// the CLASS field (INVARIANT 9).
    ClassName {
        /// The class's id in the books.
        class: ClassId,
    },
    /// An entry's memo: its description in the books.
    Memo {
        /// The entry.
        entry: EntryId,
    },
}

/// Why an account number is left out of the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LeftOut {
    /// It is longer than QuickBooks Desktop holds ([`NUMBER_MAX`]).
    TooLong,
    /// Another account's number would be written the same (ignoring
    /// case): this, as written.
    Shared(String),
}

/// One line, both texts quoted with anything unprintable escaped (`\t`,
/// `\n`, `\u{200b}`), so a change never breaks the list it is printed in,
/// and a full TIN in either masked, in any script's digits:
/// `account 6010's name "Café meals" is written "Cafe meals"`.
impl fmt::Display for IifChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (books, written) = (shown(&self.books), shown(&self.written));
        let line = match &self.field {
            IifField::AccountName { code } => format!(
                "account {}'s name {books:?} is written {written:?}",
                shown(code).escape_debug(),
            ),
            IifField::AccountNumber => {
                format!("account number {books:?} is written {written:?}")
            }
            IifField::AccountNumberLeftOut {
                why: LeftOut::TooLong,
            } => format!(
                "account number {books:?} is left out: QuickBooks Desktop holds {NUMBER_MAX} characters at most"
            ),
            IifField::AccountNumberLeftOut {
                why: LeftOut::Shared(as_written),
            } => format!(
                "account number {books:?} is left out: another account's number would also be written {:?}",
                shown(as_written)
            ),
            IifField::AtTopLevel { code, own, parent } => format!(
                "account {}'s name is written at the top level, not under account {}'s: QuickBooks gives a subaccount its parent's type, and it is {}, its parent {}",
                shown(code).escape_debug(),
                books.escape_debug(),
                own.keyword(),
                parent.keyword()
            ),
            IifField::ClassName { class } => {
                format!("class {class}'s name {books:?} is written {written:?}")
            }
            IifField::Memo { entry } => {
                format!("entry {entry}'s memo {books:?} is written {written:?}")
            }
        };
        f.write_str(&redact_full_tins(&line))
    }
}

/// `text` as a warning may show it (rules.md B-26: no full TIN): each run
/// of nine digits `ledger_core::parties::has_full_tin` finds masked (in
/// any script's digits and separators: "１２３－４５－６７８９"), and if the
/// text still holds one once written as a field, every numeral in it
/// (digits a tab apart, which the file writes a space apart, or letters
/// deunicode writes as digits: "Ƽ" is "5").
fn shown(text: &str) -> String {
    let r = redact_full_tins(text).into_owned();
    if has_full_tin(&field(&r)) {
        masked(&r)
    } else {
        r
    }
}

/// `text` with every numeral masked.
fn masked(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_numeric() { '\u{2022}' } else { c })
        .collect()
}

impl Iif {
    /// What the file writes differently from the books, to tell the
    /// human (INVARIANT 7), one line each: each account name and number
    /// as [`IifChange`] prints it, then each thing written differently in
    /// the memos once, with the entries it is in, in the order first met
    /// (`"—" is written "--" in the memos of entries 3, 5, 9`), then the
    /// memos cut to [`MEMO_MAX`]. A memo's run of changed characters is
    /// one thing: "北京" is written "Bei Jing". Every change is in one
    /// line or more, and no line holds a full TIN: a memo that holds one,
    /// as the books keep it or once written, has every numeral of its runs
    /// masked, and a line with digits masked says so (`"•••" is written
    /// "•••" in the memo of entry 3 (digits masked: may be a TIN)`:
    /// Arabic-Indic digits written in ASCII).
    pub fn warnings(&self) -> Vec<String> {
        let mut lines = Vec::new();
        type Key = (String, String, bool);
        let mut runs: Vec<(Key, Vec<EntryId>)> = Vec::new();
        let mut at: HashMap<Key, usize> = HashMap::new();
        let mut cut: Vec<EntryId> = Vec::new();
        for c in &self.changes {
            let IifField::Memo { entry } = c.field else {
                lines.push(c.to_string());
                continue;
            };
            let tin = has_full_tin(&c.books) || has_full_tin(&field(&c.books));
            for (from, to) in changed_runs(&c.books, MEMO_MAX) {
                let key = match (masked(&from), masked(&to)) {
                    (f, t) if tin && (f != from || t != to) => (f, t, true),
                    _ => (from, to, false),
                };
                let i = *at.entry(key.clone()).or_insert_with(|| {
                    runs.push((key, Vec::new()));
                    runs.len() - 1
                });
                runs[i].1.push(entry);
            }
            if field(&c.books).len() > MEMO_MAX {
                cut.push(entry);
            }
        }
        for ((from, to, tin), entries) in runs {
            lines.push(format!(
                "{from:?} is written {to:?} in {}{}",
                memos_of(entries),
                if tin {
                    " (digits masked: may be a TIN)"
                } else {
                    ""
                }
            ));
        }
        if !cut.is_empty() {
            let n = cut.len();
            lines.push(format!(
                "{} {} cut to {MEMO_MAX} characters, the most QuickBooks Desktop holds",
                memos_of(cut),
                if n == 1 { "is" } else { "are" }
            ));
        }
        lines
            .into_iter()
            .map(|l| redact_full_tins(&l).into_owned())
            .collect()
    }
}

/// "the memo of entry 3" or "the memos of entries 3, 5, 9", the ids in
/// order, each once.
fn memos_of(mut entries: Vec<EntryId>) -> String {
    entries.sort_unstable();
    entries.dedup();
    let ids: Vec<String> = entries.iter().map(ToString::to_string).collect();
    match ids.as_slice() {
        [one] => format!("the memo of entry {one}"),
        _ => format!("the memos of entries {}", ids.join(", ")),
    }
}

/// Errors.
#[derive(Debug, Error, PartialEq)]
pub enum IifError {
    /// An entry uses an account that wasn't declared.
    #[error("entry {entry} uses account {account}, which isn't in the account list")]
    UndeclaredAccount {
        /// The entry.
        entry: u64,
        /// The account.
        account: AccountId,
    },
    /// Two or more accounts would have one name in QuickBooks
    /// (INVARIANT 6).
    #[error(
        "accounts {} would have the same name in QuickBooks, {name:?}: rename one",
        .codes.join(", ")
    )]
    NameCollision {
        /// The name, as the file would write it.
        name: String,
        /// The accounts' codes in the books.
        codes: Vec<String>,
    },
    /// An entry's line has a class that wasn't declared (INVARIANT 9).
    #[error("entry {entry} uses class {class}, which isn't in the class list")]
    UndeclaredClass {
        /// The entry.
        entry: u64,
        /// The class.
        class: ClassId,
    },
    /// Two or more classes would have one name in QuickBooks
    /// (INVARIANT 9; [`iif_classes`] never builds them).
    #[error(
        "classes {} would have the same name in QuickBooks, {name:?}: rename one",
        .classes.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    )]
    ClassNameCollision {
        /// The name, as the file would write it.
        name: String,
        /// The classes' ids in the books.
        classes: Vec<ClassId>,
    },
    /// A class's NAME is empty, holds a colon, or is longer than
    /// QuickBooks Desktop holds (INVARIANT 9; [`iif_classes`] never builds
    /// one).
    #[error(
        "class {class}'s name in QuickBooks, {name:?}, isn't one level of 1 to {NAME_MAX} characters"
    )]
    ClassNameInvalid {
        /// The class's id in the books.
        class: ClassId,
        /// The NAME, as the file would write it.
        name: String,
    },
    /// An account's NAME has a level longer than QuickBooks Desktop holds
    /// (INVARIANT 2; [`iif_accounts`] never builds one).
    #[error(
        "account {code}'s name in QuickBooks, {name:?}, has a level longer than {NAME_MAX} characters"
    )]
    NameTooLong {
        /// The account's code in the books.
        code: String,
        /// The NAME, as the file would write it.
        name: String,
    },
}

/// Every account in the chart as QuickBooks will know it: its colon path
/// (parents first), number and QuickBooks type, `qb_type` deciding each
/// account's (INVARIANTS 1, 2, 4, 6), in the order of the books' codes,
/// each after its parent.
///
/// A subaccount whose type differs from its parent's is written at the
/// top level, apart from it (`apart_from`), its own subaccounts with it:
/// QuickBooks gives a subaccount its parent's type, and the type decides
/// where its balance shows. Each level is the account's own name made safe
/// for IIF, any colon in it a '-', trimmed ("?" if nothing is left) and
/// cut to [`NAME_MAX`]. Level by level from the top, the accounts whose
/// paths would match ignoring ASCII case have " (code)" added to their own
/// names (cut to make room), the code made safe the same way ("no number"
/// for a blank one), until no two match: two "Software" accounts are
/// "Software (6110)" and "Software (6120)", their subaccounts following
/// them, and a third already named "Software (6110)" is then "Software
/// (6110) (6130)". A match that remains (two accounts whose codes are
/// written alike ignoring case, or alike in their first 30 characters) is
/// left for [`journal_to_iif`] to refuse.
pub fn iif_accounts(
    ledger: &Ledger,
    qb_type: impl Fn(&ledger_core::Account) -> QbAccountType,
) -> Vec<IifAccount> {
    let accounts: Vec<&ledger_core::Account> = ledger.chart().all().collect();
    let types: HashMap<AccountId, QbAccountType> =
        accounts.iter().map(|a| (a.id, qb_type(a))).collect();
    // Its parent in QuickBooks: its parent in the books, of its type.
    let parent_of = |a: &ledger_core::Account| {
        a.parent
            .filter(|p| types.get(p) == types.get(&a.id))
            .and_then(|p| ledger.chart().get(p))
    };
    // Each account's chain of ids, highest first. A chart's parents form a
    // tree, so no chain is longer than the chart; the bound guards a
    // malformed one.
    let chain = |a: &ledger_core::Account| {
        let mut ids = Vec::new();
        let mut at = Some(a);
        for _ in 0..=accounts.len() {
            let Some(a) = at else {
                break;
            };
            ids.push(a.id);
            at = parent_of(a);
        }
        ids.reverse();
        ids
    };
    let chains: HashMap<AccountId, Vec<AccountId>> =
        accounts.iter().map(|a| (a.id, chain(a))).collect();
    let own: HashMap<AccountId, String> =
        accounts.iter().map(|a| (a.id, segment(&a.name))).collect();
    let tags: HashMap<AccountId, String> = accounts.iter().map(|a| (a.id, tag(&a.code))).collect();
    let mut level = own.clone();
    let path_of = |id: AccountId, level: &HashMap<AccountId, String>| {
        chains[&id]
            .iter()
            .map(|p| level.get(p).map_or("?", String::as_str))
            .collect::<Vec<_>>()
            .join(":")
    };
    // Level by level: a level's paths rest only on the levels above it,
    // already settled. Within a level, each round tags every untagged
    // account of each match, until a round tags none: each account is
    // tagged once at most, so it ends.
    let deepest = chains.values().map(Vec::len).max().unwrap_or(0);
    let mut tagged: HashSet<AccountId> = HashSet::new();
    for depth in 1..=deepest {
        loop {
            let mut by_name: BTreeMap<String, Vec<AccountId>> = BTreeMap::new();
            for a in accounts.iter().filter(|a| chains[&a.id].len() == depth) {
                by_name
                    .entry(path_of(a.id, &level).to_ascii_lowercase())
                    .or_default()
                    .push(a.id);
            }
            let mut more = false;
            for id in by_name.into_values().filter(|ids| ids.len() > 1).flatten() {
                if tagged.insert(id) {
                    level.insert(id, with_tag(&own[&id], &tags[&id]));
                    more = true;
                }
            }
            if !more {
                break;
            }
        }
    }
    let mut out: Vec<IifAccount> = accounts
        .iter()
        .map(|a| IifAccount {
            id: a.id,
            path: path_of(a.id, &level),
            number: field(&a.code).trim().to_string(),
            qb_type: types[&a.id],
            name: a.name.clone(),
            code: a.code.clone(),
            apart_from: a
                .parent
                .and_then(|p| ledger.chart().get(p))
                .filter(|p| parent_of(a).is_none() && types.contains_key(&p.id))
                .map(|p| (p.code.clone(), types[&p.id])),
        })
        .collect();
    out.sort_by(|a, b| a.code.cmp(&b.code));
    let parents: HashMap<AccountId, AccountId> = chains
        .iter()
        .filter_map(|(id, c)| Some((*id, *c.iter().rev().nth(1)?)))
        .collect();
    parents_first(out, &parents)
}

/// Every class (project) in the books as QuickBooks will know it, in id
/// order (INVARIANT 9): its name made safe as an account's own level is
/// (plain ASCII, a colon a '-', trimmed, "?" if nothing is left, cut to
/// [`NAME_MAX`]), and the classes whose NAMEs would match ignoring ASCII
/// case given " (id)" after their names (cut to make room), round after
/// round until none matches. Where they meet, the names already written
/// differently from the books (made safe, or tagged) yield first, so a
/// name written exactly as the books have it keeps it unless another
/// class's own name is the same ignoring case: "Café" beside "Cafe" is
/// "Cafe (1)", "Cafe" stays, and if a class is named "Cafe (1)" too, the
/// changed one is tagged again, "Cafe (1) (1)"; two "Rebrand" classes
/// are "Rebrand (2)" and "Rebrand (5)", and a third named "Rebrand (2)"
/// keeps its name while the tagged one becomes "Rebrand (2) (2)"; a
/// changed name meeting own names again and again is tagged again and
/// again ("Café" beside "Cafe", "Cafe (1)" and "Cafe (1) (1)" is "Cafe
/// (1) (1) (1)"). A tag that changes nothing (a name already cut to
/// [`NAME_MAX`] and ending in its own " (id)") is not made: that name
/// stands and the others meeting it are tagged. Each tag lengthens a
/// name or leaves it at [`NAME_MAX`] ending in its own id, after which it
/// is never tagged again; two names so ended differ by their ids. So
/// every round that finds two alike tags one, and the rounds end with no
/// two NAMEs alike.
pub fn iif_classes(ledger: &Ledger) -> Vec<IifClass> {
    let classes = ledger.list_classes();
    let own: Vec<String> = classes.iter().map(|c| segment(&c.name)).collect();
    let mut written = own.clone();
    loop {
        let mut by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, w) in written.iter().enumerate() {
            by_name.entry(w.to_ascii_lowercase()).or_default().push(i);
        }
        let mut more = false;
        for alike in by_name.into_values().filter(|ix| ix.len() > 1) {
            let can: Vec<usize> = alike
                .into_iter()
                .filter(|&i| with_tag(&written[i], &classes[i].id.to_string()) != written[i])
                .collect();
            let yielding: Vec<usize> = can
                .iter()
                .copied()
                .filter(|&i| written[i] != classes[i].name)
                .collect();
            for i in if yielding.is_empty() { can } else { yielding } {
                written[i] = with_tag(&written[i], &classes[i].id.to_string());
                more = true;
            }
        }
        if !more {
            break;
        }
    }
    classes
        .iter()
        .zip(written)
        .map(|(c, written)| IifClass {
            id: c.id,
            written,
            name: c.name.clone(),
        })
        .collect()
}

/// `accounts`, in the order given but each after its parent (INVARIANT 1:
/// a subaccount's NAME names its parent, which QuickBooks must know
/// first): the first account whose parent is already placed (or that has
/// none) goes next. An account caught in a cycle (a malformed chart) goes
/// last, in the order given.
fn parents_first(
    accounts: Vec<IifAccount>,
    parents: &HashMap<AccountId, AccountId>,
) -> Vec<IifAccount> {
    let at: HashMap<AccountId, usize> = accounts
        .iter()
        .enumerate()
        .map(|(i, a)| (a.id, i))
        .collect();
    let mut kids: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut ready: BTreeSet<usize> = BTreeSet::new();
    for (i, a) in accounts.iter().enumerate() {
        match parents.get(&a.id).and_then(|p| at.get(p)) {
            Some(&p) if p != i => kids.entry(p).or_default().push(i),
            _ => {
                ready.insert(i);
            }
        }
    }
    let mut order = Vec::with_capacity(accounts.len());
    let mut placed = vec![false; accounts.len()];
    while let Some(i) = ready.pop_first() {
        order.push(i);
        placed[i] = true;
        ready.extend(kids.remove(&i).unwrap_or_default());
    }
    order.extend((0..accounts.len()).filter(|&i| !placed[i]));
    let mut slots: Vec<Option<IifAccount>> = accounts.into_iter().map(Some).collect();
    order.into_iter().filter_map(|i| slots[i].take()).collect()
}

/// Render posted entries as IIF: the accounts declared first, then the
/// classes (none: no class list), then one transaction per entry in the
/// order given, each numbered (DOCNUM) with its entry id and each line's
/// class in CLASS; every text the file writes differently from the books,
/// and the entries with a line on accounts receivable or payable
/// (INVARIANTS 1-9). Refused when an entry uses an account `accounts`
/// lacks or a class `classes` lacks, two accounts or two classes would
/// have one name, or a name has a level too long for QuickBooks.
pub fn journal_to_iif(
    accounts: &[IifAccount],
    classes: &[IifClass],
    entries: &[&JournalEntry],
) -> Result<Iif, IifError> {
    // Made safe again: it changes nothing iif_accounts built, and an
    // account built some other way can't break the file.
    let paths: Vec<String> = accounts.iter().map(|a| field(&a.path)).collect();
    if let Some((a, path)) = accounts
        .iter()
        .zip(&paths)
        .find(|(_, p)| p.split(':').any(|l| l.len() > NAME_MAX))
    {
        return Err(IifError::NameTooLong {
            code: a.code.clone(),
            name: path.clone(),
        });
    }
    let mut by_name: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for (a, path) in accounts.iter().zip(&paths) {
        by_name
            .entry(path.to_ascii_lowercase())
            .or_default()
            .push(&a.code);
    }
    if let Some((lower, codes)) = by_name.iter().find(|(_, c)| c.len() > 1) {
        let name = paths
            .iter()
            .find(|p| p.to_ascii_lowercase() == *lower)
            .cloned()
            .unwrap_or_default();
        return Err(IifError::NameCollision {
            name,
            codes: codes.iter().map(|c| c.to_string()).collect(),
        });
    }
    // INVARIANT 6's numbers: made safe and trimmed; one too long, or
    // written like another's ignoring case, is left out.
    let numbers: Vec<String> = accounts
        .iter()
        .map(|a| field(&a.number).trim().to_string())
        .collect();
    let mut uses: HashMap<String, usize> = HashMap::new();
    for n in numbers
        .iter()
        .filter(|n| !n.is_empty() && n.len() <= NUMBER_MAX)
    {
        *uses.entry(n.to_ascii_lowercase()).or_default() += 1;
    }
    // INVARIANT 9's names: made safe again (it changes nothing
    // iif_classes built), one level each, no two alike.
    let class_names: Vec<String> = classes.iter().map(|c| field(&c.written)).collect();
    if let Some((c, name)) = classes.iter().zip(&class_names).find(|(_, n)| {
        n.is_empty() || n.len() > NAME_MAX || n.contains(':') || n.trim() != n.as_str()
    }) {
        return Err(IifError::ClassNameInvalid {
            class: c.id,
            name: name.clone(),
        });
    }
    let mut by_class_name: BTreeMap<String, Vec<ClassId>> = BTreeMap::new();
    for (c, name) in classes.iter().zip(&class_names) {
        by_class_name
            .entry(name.to_ascii_lowercase())
            .or_default()
            .push(c.id);
    }
    if let Some((lower, ids)) = by_class_name.iter().find(|(_, ids)| ids.len() > 1) {
        let name = class_names
            .iter()
            .find(|n| n.to_ascii_lowercase() == *lower)
            .cloned()
            .unwrap_or_default();
        return Err(IifError::ClassNameCollision {
            name,
            classes: ids.clone(),
        });
    }
    let class_of: HashMap<ClassId, &str> = classes
        .iter()
        .zip(&class_names)
        .map(|(c, n)| (c.id, n.as_str()))
        .collect();
    let by_id: HashMap<AccountId, (&str, QbAccountType)> = accounts
        .iter()
        .zip(&paths)
        .map(|(a, path)| (a.id, (path.as_str(), a.qb_type)))
        .collect();
    let mut changes = Vec::new();
    let mut out = format!("!ACCNT\tNAME\tACCNTTYPE\tACCNUM{EOL}");
    for ((a, path), number) in accounts.iter().zip(&paths).zip(&numbers) {
        let left_out = if number.len() > NUMBER_MAX {
            Some(LeftOut::TooLong)
        } else if uses
            .get(&number.to_ascii_lowercase())
            .is_some_and(|&n| n > 1)
        {
            Some(LeftOut::Shared(number.clone()))
        } else {
            None
        };
        let accnum = if left_out.is_some() { "" } else { number };
        out.push_str(&format!(
            "ACCNT\t{path}\t{}\t{accnum}{EOL}",
            a.qb_type.keyword()
        ));
        // The account's own level: what follows its parents' path.
        let own = path.rsplit(':').next().unwrap_or_default();
        if own != a.name {
            changes.push(IifChange {
                field: IifField::AccountName {
                    code: a.code.clone(),
                },
                books: a.name.clone(),
                written: own.to_string(),
            });
        }
        if let Some((parent, parent_type)) = &a.apart_from {
            changes.push(IifChange {
                field: IifField::AtTopLevel {
                    code: a.code.clone(),
                    own: a.qb_type,
                    parent: *parent_type,
                },
                books: parent.clone(),
                written: String::new(),
            });
        }
        if let Some(why) = left_out {
            changes.push(IifChange {
                field: IifField::AccountNumberLeftOut { why },
                books: a.code.clone(),
                written: String::new(),
            });
        } else if accnum != a.code {
            changes.push(IifChange {
                field: IifField::AccountNumber,
                books: a.code.clone(),
                written: accnum.to_string(),
            });
        }
    }
    if !classes.is_empty() {
        out.push_str(&format!("!CLASS\tNAME{EOL}"));
        for (c, name) in classes.iter().zip(&class_names) {
            out.push_str(&format!("CLASS\t{name}{EOL}"));
            if *name != c.name {
                changes.push(IifChange {
                    field: IifField::ClassName { class: c.id },
                    books: c.name.clone(),
                    written: name.clone(),
                });
            }
        }
    }
    out.push_str(&format!(
        "!TRNS\tTRNSID\tTRNSTYPE\tDATE\tACCNT\tCLASS\tAMOUNT\tDOCNUM\tMEMO{EOL}"
    ));
    out.push_str(&format!(
        "!SPL\tSPLID\tTRNSTYPE\tDATE\tACCNT\tCLASS\tAMOUNT\tDOCNUM\tMEMO{EOL}"
    ));
    out.push_str(&format!("!ENDTRNS{EOL}"));
    let mut receivable_payable = Vec::new();
    for entry in entries {
        let docnum = entry.id();
        let d = entry.date();
        let date = format!("{}/{}/{}", d.month(), d.day(), d.year());
        let mut memo = field(entry.description());
        // ASCII, so every byte is a character.
        memo.truncate(MEMO_MAX);
        if memo != entry.description() {
            changes.push(IifChange {
                field: IifField::Memo { entry: docnum },
                books: entry.description().to_string(),
                written: memo.clone(),
            });
        }
        // TRNS carries the first debit; the rest are SPLs.
        let mut lines: Vec<_> = entry.lines().iter().collect();
        lines.sort_by_key(|l| match l.side {
            Side::Debit => 0,
            Side::Credit => 1,
        });
        let mut named = false;
        for (n, line) in lines.iter().enumerate() {
            let (account, qb_type) =
                by_id
                    .get(&line.account)
                    .ok_or(IifError::UndeclaredAccount {
                        entry: entry.id(),
                        account: line.account,
                    })?;
            named |= matches!(
                qb_type,
                QbAccountType::AccountsReceivable | QbAccountType::AccountsPayable
            );
            let class = match line.class_id {
                None => "",
                Some(c) => class_of.get(&c).ok_or(IifError::UndeclaredClass {
                    entry: entry.id(),
                    class: c,
                })?,
            };
            let keyword = if n == 0 { "TRNS" } else { "SPL" };
            out.push_str(&format!(
                "{keyword}\t\tGENERAL JOURNAL\t{date}\t{account}\t{class}\t{}\t{docnum}\t{memo}{EOL}",
                format_amount(signed(line.side, line.amount.amount)),
            ));
        }
        out.push_str(&format!("ENDTRNS{EOL}"));
        if named {
            receivable_payable.push(docnum);
        }
    }
    Ok(Iif {
        text: out,
        changes,
        receivable_payable,
    })
}

/// Debit positive, credit negative (Intuit: "Debit amounts are always
/// positive, credit amounts are always negative").
fn signed(side: Side, amount: Decimal) -> Decimal {
    match side {
        Side::Debit => amount,
        Side::Credit => -amount,
    }
}

/// Two decimal places, no thousands separators.
fn format_amount(d: Decimal) -> String {
    format!("{d:.2}")
}

/// Whether `c` is a tab or ends a line (Unicode Standard Annex #14's
/// mandatory breaks: LF, VT, FF, CR, NEL, LINE SEPARATOR, PARAGRAPH
/// SEPARATOR): either would end an IIF field or record.
fn breaks(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{B}' | '\u{C}' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// Call `each` with every character of `s` and what a field writes for it
/// (INVARIANT 4), in order: printable ASCII is itself but for a double
/// quote, which is a single one; a tab or a line break is a space, any
/// other control character '?', and a character past ASCII its deunicode
/// transliteration ("é" is "e", "—" "--", "北" "Bei ", whose trailing
/// space deunicode drops at the end or before a space), or '?' when it
/// has none.
fn each_piece(s: &str, mut each: impl FnMut(char, &str)) {
    let flat: String = s
        .chars()
        .map(|c| {
            if breaks(c) {
                ' '
            } else if c.is_control() {
                '?'
            } else {
                c
            }
        })
        .collect();
    let mut piece = String::new();
    for (c, t) in s.chars().zip(flat.ascii_chars()) {
        piece.clear();
        match t {
            // deunicode writes ASCII, a few transliterations holding a line
            // break; nothing else it writes is a control.
            Some(t) => piece.extend(t.chars().map(|c| match c {
                '\n' => ' ',
                '"' => '\'',
                c if c.is_ascii_control() || !c.is_ascii() => '?',
                c => c,
            })),
            None => piece.push('?'),
        }
        each(c, &piece);
    }
}

/// A field IIF can carry (INVARIANT 4): printable ASCII with no double
/// quote, each character of `s` written as [`each_piece`] says.
fn field(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    each_piece(s, |_, piece| out.push_str(piece));
    out
}

/// Each run of consecutive characters of `s` that [`field`] writes
/// differently, and what it writes for them, within the field's first
/// `limit` characters (a memo cut there keeps no more): "Café — 北京"
/// gives ("é", "e"), ("—", "--") and ("北京", "Bei Jing").
fn changed_runs(s: &str, limit: usize) -> Vec<(String, String)> {
    let mut runs = Vec::new();
    let mut run: Option<(String, String)> = None;
    let mut len = 0;
    each_piece(s, |c, piece| {
        if len >= limit {
            return;
        }
        // ASCII, so every byte is a character.
        let piece = &piece[..piece.len().min(limit - len)];
        len += piece.len();
        let mut buf = [0u8; 4];
        if piece == c.encode_utf8(&mut buf) {
            runs.extend(run.take());
        } else {
            let (from, to) = run.get_or_insert_with(Default::default);
            from.push(c);
            to.push_str(piece);
        }
    });
    runs.extend(run);
    runs
}

/// `s`'s first `max` characters, any space they end with dropped. `s` is a
/// field (ASCII), so every byte is a character.
fn cut(s: &str, max: usize) -> &str {
    s.get(..max).unwrap_or(s).trim_end()
}

/// One account's own level in its path (INVARIANT 2): the name made safe,
/// any colon a '-' (a fullwidth colon transliterates to one), trimmed,
/// "?" if nothing is left, and cut to [`NAME_MAX`].
fn segment(name: &str) -> String {
    let s = field(name).replace(':', "-");
    match cut(s.trim(), NAME_MAX) {
        "" => "?".to_string(),
        t => t.to_string(),
    }
}

/// What tells an account's level from another's alike (INVARIANT 6): its
/// code, made safe like a level, or "no number" for a blank one.
fn tag(code: &str) -> String {
    match field(code).replace(':', "-").trim() {
        "" => "no number".to_string(),
        t => t.to_string(),
    }
}

/// `own` followed by " (tag)", `own` cut to leave it room within
/// [`NAME_MAX`]; a tag with no room left beside it stands alone, cut.
fn with_tag(own: &str, tag: &str) -> String {
    let suffix = format!(" ({tag})");
    match cut(own, NAME_MAX.saturating_sub(suffix.len())) {
        "" => cut(suffix.trim_start(), NAME_MAX).to_string(),
        base => format!("{base}{suffix}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledger_core::{AccountType, Currency, Money};

    #[test]
    fn iif_declares_accounts_then_journals() {
        let mut ledger = Ledger::new(Currency::USD);
        let cash = ledger
            .add_account("1000", "Cash", AccountType::Asset, None, false)
            .unwrap();
        let rev = ledger
            .add_account("4000", "Revenue", AccountType::Revenue, None, false)
            .unwrap();
        let id = ledger.create_entry(chrono::NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(), "Sale");
        let m = Money::new(Decimal::new(100000, 2), Currency::USD);
        ledger.add_line(id, cash, Side::Debit, m).unwrap();
        ledger.add_line(id, rev, Side::Credit, m).unwrap();
        ledger.post_entry(id).unwrap();
        let accounts = iif_accounts(&ledger, |a| match a.account_type {
            AccountType::Asset => QbAccountType::Bank,
            _ => QbAccountType::Income,
        });
        let iif = journal_to_iif(&accounts, &[], &[ledger.get_entry(id).unwrap()]).unwrap();
        assert_eq!(
            iif.text,
            "!ACCNT\tNAME\tACCNTTYPE\tACCNUM\r\n\
             ACCNT\tCash\tBANK\t1000\r\n\
             ACCNT\tRevenue\tINC\t4000\r\n\
             !TRNS\tTRNSID\tTRNSTYPE\tDATE\tACCNT\tCLASS\tAMOUNT\tDOCNUM\tMEMO\r\n\
             !SPL\tSPLID\tTRNSTYPE\tDATE\tACCNT\tCLASS\tAMOUNT\tDOCNUM\tMEMO\r\n\
             !ENDTRNS\r\n\
             TRNS\t\tGENERAL JOURNAL\t1/5/2026\tCash\t\t1000.00\t1\tSale\r\n\
             SPL\t\tGENERAL JOURNAL\t1/5/2026\tRevenue\t\t-1000.00\t1\tSale\r\n\
             ENDTRNS\r\n"
        );
        assert!(iif.changes.is_empty());
    }

    /// INVARIANTS 2 and 4 over every character there is (all 1,112,064
    /// Unicode scalar values, alone, after a letter and before one): a
    /// field is printable ASCII with no double quote, so never a tab or a
    /// line break, and making it safe again changes nothing; it lists a
    /// changed run exactly when it writes the text differently; a level is
    /// also never empty, never holds a colon, has no space at either end,
    /// and is at most NAME_MAX characters.
    #[test]
    fn every_character_is_written_as_printable_ascii() {
        let printable = |s: &str| s.bytes().all(|b| (0x20..0x7F).contains(&b) && b != b'"');
        let mut n = 0u32;
        for c in (0..=0x10FFFFu32).filter_map(char::from_u32) {
            n += 1;
            for s in [c.to_string(), format!("a{c}"), format!("{c}b")] {
                let w = field(&s);
                assert!(printable(&w), "{c:?} is written {w:?}");
                assert_eq!(field(&w), w, "{c:?}");
                assert_eq!(changed_runs(&s, MEMO_MAX).is_empty(), w == s, "{c:?}");
                let own = segment(&s);
                assert!(printable(&own) && !own.contains(':'), "{c:?}: {own:?}");
                assert!(!own.is_empty() && own == own.trim(), "{c:?}: {own:?}");
                assert!(own.len() <= NAME_MAX, "{c:?}: {own:?}");
                assert_eq!(segment(&own), own, "{c:?}");
            }
        }
        assert_eq!(n, 1_112_064);
        // Printable ASCII is itself, every character of it but the double
        // quote.
        let ascii: String = (0x20u8..0x7F).map(char::from).collect();
        assert_eq!(field(&ascii), ascii.replace('"', "'"));
        assert_eq!(
            changed_runs(&ascii, MEMO_MAX),
            [("\"".to_string(), "'".to_string())]
        );
    }

    /// deunicode 1.6.0's table, sampled (Cargo.toml pins it exactly): a
    /// version that writes one of these differently fails here, never
    /// quietly in a CPA's file.
    #[test]
    fn the_transliteration_table_is_pinned() {
        for (from, to) in [
            ("René", "Rene"),
            ("Café", "Cafe"),
            ("北京", "Bei Jing"),
            ("東京", "Dong Jing"),
            ("서울", "seoul"),
            ("ありがとう", "arigatou"),
            ("Łódź", "Lodz"),
            ("Straße", "Strasse"),
            ("Æneid", "AEneid"),
            ("€5", "EUR5"),
            ("a\u{2013}b\u{2014}c\u{2212}d", "a-b--c-d"),
            ("\u{201C}q\u{201D} \u{2018}s\u{2019}", "'q' 's'"),
            ("say \"hi\"", "say 'hi'"),
            ("\u{201E}q\u{AB}", ",,q<<"),
            ("x\u{A0}y", "x y"),
            ("\u{2192}", "-"),
            ("Ratio\u{FF1A}Ops", "Ratio:Ops"),
            ("\u{1F600}", "grinning"),
            ("\u{2615} break", "coffee break"),
            ("e\u{301}", "e"),
            ("\u{FEFF}x\u{200B}y", "x y"),
            ("\u{E000}", "?"),
        ] {
            assert_eq!(field(from), to, "{from:?}");
        }
    }

    /// INVARIANT 4's controls: a tab, LF, CR, VT, FF, NEL and the two
    /// Unicode separators are spaces; any other control is '?'.
    #[test]
    fn breaks_are_spaces_and_other_controls_question_marks() {
        assert_eq!(
            field("a\tb\nc\r\nd\u{B}e\u{C}f\u{85}g\u{2028}h\u{2029}i"),
            "a b c  d e f g h i"
        );
        assert_eq!(field("a\u{0}b\u{1}c\u{1B}d\u{7F}e\u{9F}f"), "a?b?c?d?e?f");
    }

    /// INVARIANT 2: a level is trimmed, never empty, never a colon, and
    /// cut to NAME_MAX characters, any space it then ends with dropped.
    #[test]
    fn a_level_is_trimmed_never_empty_and_cut_to_31() {
        assert_eq!(segment(" Rent \t"), "Rent");
        assert_eq!(segment("北京"), "Bei Jing");
        assert_eq!(segment("\u{FEFF}"), "?");
        assert_eq!(segment("a:b\u{FF1A}c"), "a-b-c");
        assert_eq!(NAME_MAX, 31);
        let long = "Professional services and consulting fees";
        assert_eq!(segment(long), "Professional services and consu");
        assert_eq!(segment(long).len(), 31);
        // Cut just after a space: the space goes.
        assert_eq!(
            segment("abcdefghij abcdefghij abcdefgh xyz"),
            "abcdefghij abcdefghij abcdefgh"
        );
        assert_eq!(segment(&"x".repeat(31)), "x".repeat(31));
        // Transliterated first, then cut.
        assert_eq!(
            segment("北京北京北京北京北京"),
            "Bei Jing Bei Jing Bei Jing Bei"
        );
    }

    /// INVARIANT 6's tag: " (code)" fits in NAME_MAX, the name cut to make
    /// room; a code with a colon can't add a level; a blank code is "no
    /// number"; a tag too long to fit beside the name stands alone, cut.
    #[test]
    fn a_tag_fits_and_never_adds_a_level() {
        assert_eq!(with_tag("Software", &tag("6100")), "Software (6100)");
        assert_eq!(
            with_tag(
                &segment("Professional services and consulting fees"),
                &tag("6100")
            ),
            "Professional services an (6100)"
        );
        assert_eq!(with_tag("Software", &tag("61:00")), "Software (61-00)");
        assert_eq!(with_tag("Software", &tag("  ")), "Software (no number)");
        assert_eq!(
            with_tag("Software", &tag(" 6\u{FF11}00 ")),
            "Software (6100)"
        );
        let t = tag(&"9".repeat(40));
        assert_eq!(with_tag("Software", &t), format!("({}", "9".repeat(30)));
        let (long, code) = ("x".repeat(31), "7".repeat(28));
        for (own, code) in [
            ("x", "1"),
            (long.as_str(), "1234567"),
            ("ab", code.as_str()),
        ] {
            let w = with_tag(own, &tag(code));
            assert!(
                w.len() <= NAME_MAX && !w.contains(':') && w == w.trim(),
                "{w:?}"
            );
        }
    }

    /// Iif::warnings' runs: each run of changed characters once, with what
    /// it is written as, within the first `limit` characters written.
    #[test]
    fn changed_runs_are_the_characters_written_differently() {
        let runs = |s: &str, limit: usize| changed_runs(s, limit);
        let pairs = |v: &[(&str, &str)]| -> Vec<(String, String)> {
            v.iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect()
        };
        assert_eq!(
            runs("Café — 北京", MEMO_MAX),
            pairs(&[("é", "e"), ("—", "--"), ("北京", "Bei Jing")])
        );
        assert_eq!(
            runs("a\t☕\nb \"c\"", MEMO_MAX),
            pairs(&[("\t☕\n", " coffee "), ("\"", "'"), ("\"", "'")])
        );
        assert_eq!(runs("plain", MEMO_MAX), pairs(&[]));
        // Within the limit: "北" is written "Bei ", cut to "Be".
        assert_eq!(runs("ab北京", 4), pairs(&[("北", "Be")]));
        assert_eq!(runs("abc—", 3), pairs(&[]));
    }
}
