//! The text a write tool is given never holds a tax id (BS-partner-2), checked
//! once, before the call runs or writes anything, from one table of every
//! string a write tool takes.
//!
//! ```text
//! GOAL: a full tax id enters the books only through the vault
//!       (books.set_tin, books.set_w9). Free text, which most of a write
//!       tool's strings are, is checked here for what looks like one, in
//!       one place, so a field is never forgotten because its tool never
//!       thought of it.
//! SOURCE: BS-partner-2; WB-format-10 (a refused call writes nothing);
//!         wanderbooks' CHOICE on BS-partner-2 (the owner, 2026-11-21:
//!         shapes on free text, strict on party fields, masking on bank
//!         imports), docs/spec/claims/partner.md.
//! INVARIANTS:
//!   1. Every string a write tool's schema has (a field, an array's
//!      items, a map's values and keys, any untyped value) is in TABLE
//!      under one class, and every row of TABLE is a string the schema
//!      has (tests `every_string_a_write_tool_takes_is_in_the_table`,
//!      `the_table_names_only_strings_the_tools_take`): a new field fails
//!      the build until it is classed.
//!   2. FREE text is refused when it looks like a tax id
//!      (`ledger_core::settings::holds_printed_tin`: nine digits standing
//!      alone, an SSN's 3-2-4, an EIN's 2-7, with any dash, dot or space
//!      between). STRICT text (a party's fields, a name that becomes
//!      one, an account name, a recipient) is refused at any run of nine
//!      digits (`ledger_core::parties::has_full_tin`), as the engine
//!      refuses it.
//!   3. EXEMPT strings are exempt for a reason the class names: the vault's
//!      own input, a lookup key for a record the books already hold, an
//!      id or digest, an amount, a date, a word from a fixed vocabulary,
//!      the last four digits, a bank pattern that is never kept, a link
//!      (the engine's link rule reads it), or a structured value the
//!      engine's own validation reads (a setting, a convention).
//!   4. A string whose path the table does not know is checked as FREE:
//!      the check fails toward refusing.
//!   5. The check is made for any call that can write, preview or not,
//!      before the call runs; the refusal says which field, that it looks
//!      like a tax id, and where a tax id goes, never the number, and the
//!      books are as they were.
//! SCENARIOS:
//!   1. IF books.add_class is given the name `Acme 000-00-0000`: SHOULD
//!      refuse in words naming `name`, and write nothing.
//!   2. IF books.create_invoice is given the memo `inv 2026-11-0042`:
//!      SHOULD NOT refuse.
//!   3. IF books.set_tin is given a full tax id in `tin`: SHOULD NOT
//!      refuse here (the vault is where it goes).
//! ```

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::{Map, Value};

/// Why a string needs no check for a tax id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Why {
    /// A full tax id as the vault takes it (`books.set_tin`, `books.set_w9`).
    Vault,
    /// A key that names a record the books hold (an account code, a party,
    /// a vendor, an asset, a project): a key nothing matches is refused,
    /// and what is kept is the record's, never this text.
    Lookup,
    /// An id, an upload, a digest or a preview the books made.
    Id,
    /// A decimal amount, quantity or rate: parsed as a number, never kept
    /// as text.
    Amount,
    /// A date or a month: parsed, never kept as text.
    Date,
    /// A word from a fixed vocabulary the tool parses; a word outside it is
    /// refused.
    Enum,
    /// The last four digits of a card, account or tax id: exactly four
    /// digits are accepted, and nothing else.
    Digits,
    /// A bank pattern matched against descriptions and never kept.
    Pattern,
    /// A web address: the engine reads it with the link rule
    /// (`holds_printed_tin_in_link`), under which an app listing's id in a
    /// path is no tax id.
    Link,
    /// A structured value (a setting's, a convention's) that the engine's
    /// own validation reads, every text of it, with the same shapes
    /// (`ledger_core::settings`, `ledger_core::conventions`).
    Engine,
}

/// What the check does with a string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    /// Free text: refused when it looks like a tax id.
    Free,
    /// A party's own field (or a name that becomes one): refused at any
    /// run of nine digits.
    Strict,
    /// Not checked, for the reason named.
    Exempt(Why),
}

const FREE: Class = Class::Free;
const STRICT: Class = Class::Strict;
const VAULT: Class = Class::Exempt(Why::Vault);
const LOOKUP: Class = Class::Exempt(Why::Lookup);
const ID: Class = Class::Exempt(Why::Id);
const AMOUNT: Class = Class::Exempt(Why::Amount);
const DATE: Class = Class::Exempt(Why::Date);
const ENUM: Class = Class::Exempt(Why::Enum);
const DIGITS: Class = Class::Exempt(Why::Digits);
const PATTERN: Class = Class::Exempt(Why::Pattern);
const LINK: Class = Class::Exempt(Why::Link);
const ENGINE: Class = Class::Exempt(Why::Engine);

/// Every string a write tool takes, by tool and path: `a.b` for a field,
/// `a[]` for an array's items, `a.*` for a map's values and `a.#key` for
/// its keys. A tool with no string at all is not listed.
pub(crate) const TABLE: &[(&str, &[(&str, Class)])] = &[
    (
        "books.add_account",
        &[
            ("account_type", ENUM),
            ("code", FREE),
            ("deductible_percent", AMOUNT),
            ("name", STRICT),
            ("tax_category", ENUM),
        ],
    ),
    ("books.add_class", &[("name", FREE)]),
    ("books.add_customer", &[("name", STRICT)]),
    (
        "books.add_instrument",
        &[
            ("account", LOOKUP),
            ("kind", ENUM),
            ("label", FREE),
            ("last4", DIGITS),
            ("owner_party", LOOKUP),
        ],
    ),
    (
        "books.add_line",
        &[
            ("account", LOOKUP),
            ("amount", AMOUNT),
            ("class", LOOKUP),
            ("foreign_amount", AMOUNT),
            ("foreign_currency", ENUM),
            ("fx_rate", AMOUNT),
            ("side", ENUM),
        ],
    ),
    ("books.add_party", &[("name", STRICT), ("roles[]", ENUM)]),
    (
        "books.add_project",
        &[
            ("description", FREE),
            ("ended_on", DATE),
            ("kind", ENUM),
            ("links[]", LINK),
            ("name", FREE),
            ("notes", FREE),
            ("party", LOOKUP),
            ("started_on", DATE),
            ("status", ENUM),
        ],
    ),
    ("books.add_vendor", &[("name", STRICT)]),
    (
        "books.annotate_attachment",
        &[
            ("fields.amount", AMOUNT),
            ("fields.category", FREE),
            ("fields.date", DATE),
            ("fields.instrument", LOOKUP),
            ("fields.payee", FREE),
            ("fields.reimbursable_to", FREE),
            ("fields.source", FREE),
            ("note", FREE),
        ],
    ),
    (
        "books.approve_entry",
        &[("approved_on", DATE), ("approver", FREE), ("note", FREE)],
    ),
    (
        "books.assign_role",
        &[("account", LOOKUP), ("role", LOOKUP)],
    ),
    (
        "books.attach_receipt",
        &[
            ("description", FREE),
            ("file_name", FREE),
            ("link_to[].id", LOOKUP),
            ("link_to[].kind", ENUM),
            ("mime_type", FREE),
            ("upload_id", ID),
        ],
    ),
    (
        "books.attest_period",
        &[
            ("method.model", FREE),
            ("method.reasoning_effort", FREE),
            ("note", FREE),
            ("period", DATE),
        ],
    ),
    (
        "books.attest_receipt",
        &[
            ("amount", AMOUNT),
            ("date", DATE),
            ("method.model", FREE),
            ("method.reasoning_effort", FREE),
            ("note", FREE),
        ],
    ),
    (
        "books.audit",
        &[("preview", ID), ("scope", ENUM), ("through", DATE)],
    ),
    (
        "books.begin_upload",
        &[
            ("file_name", FREE),
            ("mime_type", FREE),
            ("purpose", ENUM),
            ("sha256", ID),
        ],
    ),
    ("books.close_period", &[("preview", ID)]),
    ("books.consolidate_duplicates", &[("preview", ID)]),
    (
        "books.correct_asset",
        &[
            ("asset", LOOKUP),
            ("cost", AMOUNT),
            ("in_service", DATE),
            ("in_service_date", DATE),
            ("reason", FREE),
            ("salvage_value", AMOUNT),
        ],
    ),
    (
        "books.create_bill",
        &[
            ("date", DATE),
            ("due_date", DATE),
            ("duplicate_reason", FREE),
            ("lines[].account", LOOKUP),
            ("lines[].amount", AMOUNT),
            ("lines[].class", LOOKUP),
            ("lines[].description", FREE),
            ("lines[].quantity", AMOUNT),
            ("lines[].unit_price", AMOUNT),
            ("memo", FREE),
            ("preview", ID),
            ("received_on", DATE),
            ("terms", FREE),
            ("upload_id", ID),
            ("vendor", LOOKUP),
            ("vendor_ref", FREE),
        ],
    ),
    (
        "books.create_invoice",
        &[
            ("bill_to.address", STRICT),
            ("bill_to.billing_address", STRICT),
            ("bill_to.billing_email", STRICT),
            ("bill_to.contact", STRICT),
            ("bill_to.email", STRICT),
            ("bill_to.name", STRICT),
            ("bill_to.phone", STRICT),
            ("bill_to.shipping_address", STRICT),
            ("customer", LOOKUP),
            ("customer_ref", FREE),
            ("date", DATE),
            ("due_date", DATE),
            ("lines[].account", LOOKUP),
            ("lines[].amount", AMOUNT),
            ("lines[].class", LOOKUP),
            ("lines[].description", FREE),
            ("lines[].quantity", AMOUNT),
            ("lines[].unit_price", AMOUNT),
            ("memo", FREE),
            ("preview", ID),
            ("terms", FREE),
        ],
    ),
    (
        "books.create_period",
        &[("end", DATE), ("name", FREE), ("start", DATE)],
    ),
    (
        "books.create_recurring",
        &[
            ("description", FREE),
            ("end", DATE),
            ("frequency", ENUM),
            ("lines[].account", LOOKUP),
            ("lines[].amount", AMOUNT),
            ("lines[].side", ENUM),
            ("start", DATE),
        ],
    ),
    (
        "books.create_schedule",
        &[
            ("balance_account", LOOKUP),
            ("convention", ENUM),
            ("description", FREE),
            ("kind", ENUM),
            ("pl_account", LOOKUP),
            ("start", DATE),
            ("total", AMOUNT),
        ],
    ),
    ("books.discard_draft", &[("preview", ID), ("reason", FREE)]),
    (
        "books.dispose_asset",
        &[
            ("buyer", LOOKUP),
            ("date", DATE),
            ("gain_loss_account", LOOKUP),
            ("preview", ID),
            ("proceeds", AMOUNT),
            ("proceeds_account", LOOKUP),
            ("tag", LOOKUP),
        ],
    ),
    (
        "books.draft_entry",
        &[
            ("date", DATE),
            ("description", FREE),
            ("instrument", LOOKUP),
            ("no_party_reason", FREE),
            ("parties[].amount", AMOUNT),
            ("parties[].party", LOOKUP),
            ("parties[].percent", AMOUNT),
            ("party", LOOKUP),
            ("vendor", LOOKUP),
        ],
    ),
    (
        "books.draft_from_template",
        &[
            ("accounts.#key", ENUM),
            ("accounts.*", LOOKUP),
            ("amount", AMOUNT),
            ("date", DATE),
            ("description", FREE),
            ("no_party_reason", FREE),
            ("party", LOOKUP),
            ("template_id", LOOKUP),
        ],
    ),
    (
        "books.file_document",
        &[
            ("category", ENUM),
            ("description", FREE),
            ("file_name", FREE),
            ("link_to[].id", LOOKUP),
            ("link_to[].kind", ENUM),
            ("mime_type", FREE),
            ("upload_id", ID),
        ],
    ),
    (
        "books.import_bank_csv",
        &[
            ("bank_account_code", LOOKUP),
            ("date_order", ENUM),
            ("default_offset_code", LOOKUP),
            ("rules.#key", PATTERN),
            ("rules.*", LOOKUP),
            ("upload_id", ID),
        ],
    ),
    ("books.import_chart_csv", &[("upload_id", ID)]),
    (
        "books.label_asset",
        &[
            ("asset", LOOKUP),
            ("description", FREE),
            ("serial", FREE),
            ("tag", FREE),
        ],
    ),
    (
        "books.link_attachment",
        &[("subject.id", LOOKUP), ("subject.kind", ENUM)],
    ),
    (
        "books.mark_sent",
        &[
            ("by", FREE),
            ("invoice", LOOKUP),
            ("kind", ENUM),
            ("note", FREE),
            ("on", DATE),
            ("recipients[]", STRICT),
            ("reference", FREE),
            ("via", FREE),
        ],
    ),
    (
        "books.pay_bill",
        &[
            ("amount", AMOUNT),
            ("applications[].amount", AMOUNT),
            ("applications[].discount", AMOUNT),
            ("applications[].discount_reason", FREE),
            ("date", DATE),
            ("deposit_to", LOOKUP),
            ("fee.account", LOOKUP),
            ("fee.amount", AMOUNT),
            ("fee.vendor", LOOKUP),
            ("instrument", LOOKUP),
            ("paid_from", LOOKUP),
            ("preview", ID),
            ("reference", FREE),
        ],
    ),
    (
        "books.post_entry",
        &[("expected_total", AMOUNT), ("preview", ID)],
    ),
    (
        "books.purge_tin",
        &[("party", LOOKUP), ("preview", ID), ("reason", FREE)],
    ),
    (
        "books.raise_item",
        &[
            ("citation", FREE),
            ("decided_by", FREE),
            ("decided_on", DATE),
            ("detail_md", FREE),
            ("due", DATE),
            ("note", FREE),
            ("reason", FREE),
            ("title", FREE),
        ],
    ),
    (
        "books.receive_payment",
        &[
            ("amount", AMOUNT),
            ("applications[].amount", AMOUNT),
            ("applications[].discount", AMOUNT),
            ("applications[].discount_reason", FREE),
            ("date", DATE),
            ("deposit_to", LOOKUP),
            ("fee.account", LOOKUP),
            ("fee.amount", AMOUNT),
            ("fee.vendor", LOOKUP),
            ("instrument", LOOKUP),
            ("paid_from", LOOKUP),
            ("preview", ID),
            ("reference", FREE),
        ],
    ),
    (
        "books.reconcile",
        &[
            ("account", LOOKUP),
            ("preview", ID),
            ("statement_balance", AMOUNT),
            ("statement_date", DATE),
        ],
    ),
    (
        "books.record_decision",
        &[
            ("citation", FREE),
            ("decided_by[]", FREE),
            ("decided_on", DATE),
            ("decision_md", FREE),
            ("projects[]", LOOKUP),
            ("reason", FREE),
            ("subject", FREE),
        ],
    ),
    (
        "books.record_payout",
        &[
            ("adjustments[].account", LOOKUP),
            ("adjustments[].amount", AMOUNT),
            ("adjustments[].class", LOOKUP),
            ("adjustments[].side", ENUM),
            ("date", DATE),
            ("deposit", AMOUNT),
            ("deposit_account", LOOKUP),
            ("description", FREE),
            ("fees[].account", LOOKUP),
            ("fees[].amount", AMOUNT),
            ("fees[].class", LOOKUP),
            ("revenue[].account", LOOKUP),
            ("revenue[].amount", AMOUNT),
            ("revenue[].class", LOOKUP),
        ],
    ),
    (
        "books.record_statement",
        &[
            ("account", LOOKUP),
            ("closing_balance", AMOUNT),
            ("opening_balance", AMOUNT),
            ("period_end", DATE),
            ("period_start", DATE),
        ],
    ),
    (
        "books.register_asset",
        &[
            ("accum_depr_account", LOOKUP),
            ("basis_rule", ENUM),
            ("basis_source.citation", FREE),
            ("basis_source.decided_by", FREE),
            ("basis_source.decided_on", DATE),
            ("basis_source.note", FREE),
            ("contributor_adjusted_basis", AMOUNT),
            ("contributor_basis", AMOUNT),
            ("cost", AMOUNT),
            ("cost_account", LOOKUP),
            ("description", FREE),
            ("expense_account", LOOKUP),
            ("fmv_at_contribution", AMOUNT),
            ("in_service_date", DATE),
            ("purchase_date", DATE),
            ("salvage_value", AMOUNT),
            ("sec704c_method", ENUM),
            ("serial", FREE),
            ("tag", FREE),
            ("tax_basis_rule", ENUM),
        ],
    ),
    (
        "books.remit_payroll_taxes",
        &[("agency", LOOKUP), ("cash_account", LOOKUP), ("date", DATE)],
    ),
    (
        "books.rename_account",
        &[("account", LOOKUP), ("name", STRICT), ("reason", FREE)],
    ),
    (
        "books.renumber_account",
        &[("account", LOOKUP), ("code", FREE)],
    ),
    (
        "books.resolve_item",
        &[
            ("citation", FREE),
            ("decided_by", FREE),
            ("decided_on", DATE),
            ("note", FREE),
            ("resolution", FREE),
        ],
    ),
    (
        "books.retire_convention",
        &[
            ("citation", FREE),
            ("decided_by", FREE),
            ("decided_on", DATE),
            ("id", FREE),
            ("note", FREE),
            ("reason", FREE),
        ],
    ),
    (
        "books.reverse_entry",
        &[("date", DATE), ("description", FREE), ("preview", ID)],
    ),
    (
        "books.review_entry",
        &[
            ("method.model", FREE),
            ("method.reasoning_effort", FREE),
            ("note", FREE),
        ],
    ),
    ("books.run_depreciation", &[("period", DATE)]),
    (
        "books.run_payroll",
        &[
            ("cash_account", LOOKUP),
            ("date", DATE),
            ("lines[].employee", LOOKUP),
            ("lines[].employee_name", FREE),
            ("lines[].federal_withholding", AMOUNT),
            ("lines[].gross_pay", AMOUNT),
            ("lines[].medicare_employee", AMOUNT),
            ("lines[].medicare_employer", AMOUNT),
            ("lines[].other_deductions", AMOUNT),
            ("lines[].ss_employee", AMOUNT),
            ("lines[].ss_employer", AMOUNT),
            ("lines[].state_withholding", AMOUNT),
        ],
    ),
    ("books.run_recurring", &[("through", DATE)]),
    ("books.run_schedules", &[("preview", ID), ("through", DATE)]),
    (
        "books.set_asset_tax_basis",
        &[
            ("asset", LOOKUP),
            ("basis_source.citation", FREE),
            ("basis_source.decided_by", FREE),
            ("basis_source.decided_on", DATE),
            ("basis_source.note", FREE),
            ("contributor_adjusted_basis", AMOUNT),
            ("contributor_basis", AMOUNT),
            ("fmv_at_contribution", AMOUNT),
            ("rule", ENUM),
            ("sec704c_method", ENUM),
        ],
    ),
    (
        "books.set_convention",
        &[
            ("body_md", FREE),
            ("citation", FREE),
            ("decided_by", FREE),
            ("decided_on", DATE),
            ("id", FREE),
            ("kind", ENUM),
            ("note", FREE),
            ("reason", FREE),
            ("structured", ENGINE),
            ("title", FREE),
        ],
    ),
    ("books.set_draft_state", &[("note", FREE), ("state", ENUM)]),
    ("books.set_entry_instrument", &[("instrument", LOOKUP)]),
    (
        "books.set_entry_party",
        &[
            ("no_party_reason", FREE),
            ("parties[].amount", AMOUNT),
            ("parties[].party", LOOKUP),
            ("parties[].percent", AMOUNT),
            ("party", LOOKUP),
        ],
    ),
    ("books.set_entry_vendor", &[("vendor", LOOKUP)]),
    (
        "books.set_fx_rate",
        &[("currency", ENUM), ("date", DATE), ("rate", AMOUNT)],
    ),
    (
        "books.set_party_roles",
        &[("add[]", ENUM), ("party", LOOKUP), ("remove[]", ENUM)],
    ),
    (
        "books.set_setting",
        &[
            ("changes[].key", ENUM),
            ("changes[].value", ENGINE),
            ("citation", FREE),
            ("decided_by", FREE),
            ("decided_on", DATE),
            ("note", FREE),
        ],
    ),
    (
        "books.set_tax_category",
        &[
            ("account", LOOKUP),
            ("category", ENUM),
            ("deductible_percent", AMOUNT),
        ],
    ),
    (
        "books.set_tin",
        &[("party", LOOKUP), ("tin", VAULT), ("tin_type", ENUM)],
    ),
    (
        "books.set_vendor_1099",
        &[
            ("citation", FREE),
            ("decided_by", FREE),
            ("decided_on", DATE),
            ("effective", DATE),
            ("form_1099", ENUM),
            ("note", FREE),
            ("reason", FREE),
            ("reason_kind", ENUM),
            ("vendor", LOOKUP),
        ],
    ),
    (
        "books.set_w4",
        &[
            ("kept_at", FREE),
            ("party", LOOKUP),
            ("received_on", DATE),
            ("state", FREE),
        ],
    ),
    (
        "books.set_w9",
        &[
            ("entity_type", FREE),
            ("exempt_payee_code", FREE),
            ("fatca_code", FREE),
            ("kept_at", FREE),
            ("party", LOOKUP),
            ("received_on", DATE),
            ("tin", VAULT),
            ("tin_last4", DIGITS),
            ("tin_type", ENUM),
            ("w9_file_name", FREE),
            ("w9_upload_id", ID),
        ],
    ),
    (
        "books.unlink_attachment",
        &[
            ("reason", FREE),
            ("subject.id", LOOKUP),
            ("subject.kind", ENUM),
        ],
    ),
    ("books.unlink_receipt", &[("reason", FREE)]),
    (
        "books.update_party",
        &[
            ("address", STRICT),
            ("billing_address", STRICT),
            ("billing_email", STRICT),
            ("contact", STRICT),
            ("creditor_of[]", LOOKUP),
            ("default_terms_as_customer", STRICT),
            ("default_terms_as_vendor", STRICT),
            ("display_name", STRICT),
            ("email", STRICT),
            ("name", STRICT),
            ("notes", STRICT),
            ("party", LOOKUP),
            ("phone", STRICT),
            ("shipping_address", STRICT),
            ("website", STRICT),
        ],
    ),
    (
        "books.update_project",
        &[
            ("description", FREE),
            ("ended_on", DATE),
            ("kind", ENUM),
            ("links[]", LINK),
            ("name", FREE),
            ("notes", FREE),
            ("party", LOOKUP),
            ("project", LOOKUP),
            ("started_on", DATE),
            ("status", ENUM),
        ],
    ),
    (
        "books.void_document",
        &[("date", DATE), ("kind", ENUM), ("reason", FREE)],
    ),
    ("books.void_reconciliation", &[("reason", FREE)]),
    ("books.void_statement", &[("reason", FREE)]),
    ("books.withdraw_attachment", &[("reason", FREE)]),
    (
        "books.write_off_invoice",
        &[
            ("amount", AMOUNT),
            ("date", DATE),
            ("preview", ID),
            ("reason", FREE),
            ("upload_id", ID),
        ],
    ),
];

/// The table as lookups: tool, then path.
fn classes() -> &'static HashMap<&'static str, HashMap<&'static str, Class>> {
    static MAP: OnceLock<HashMap<&'static str, HashMap<&'static str, Class>>> = OnceLock::new();
    MAP.get_or_init(|| {
        TABLE
            .iter()
            .map(|(tool, rows)| (*tool, rows.iter().copied().collect()))
            .collect()
    })
}

/// The class of the string at `path` of `tool`: its row, or FREE
/// (INVARIANT 4).
fn class_of(tool: &str, path: &str) -> Class {
    classes()
        .get(tool)
        .and_then(|t| t.get(path))
        .copied()
        .unwrap_or(Class::Free)
}

/// Whether `text` is refused as `class`.
fn looks_like_a_tax_id(class: Class, text: &str) -> bool {
    match class {
        Class::Free => ledger_core::settings::holds_printed_tin(text),
        Class::Strict => ledger_core::parties::has_full_tin(text),
        Class::Exempt(_) => false,
    }
}

/// Why `tool` can't take `args`, in words, when a string in them looks like
/// a tax id (INVARIANTS 2 to 5); None when every string will do. Only a
/// tool that can write is asked (`calls::writes`).
pub(crate) fn refusal(tool: &str, args: Option<&Map<String, Value>>) -> Option<String> {
    let mut found = None;
    for (k, v) in args?.iter() {
        let child = Path::root().field(tool, k);
        walk(tool, v, &child, &mut found);
        if found.is_some() {
            break;
        }
    }
    found
}

/// Where a value sits: its path as the table spells it, and as the caller
/// wrote it (with the index, the key).
struct Path {
    table: String,
    shown: String,
}

impl Path {
    fn root() -> Self {
        Path {
            table: String::new(),
            shown: String::new(),
        }
    }

    /// The field `key` of the object here: a map's value when the table
    /// has the object's `*` row, else a field of its own.
    fn field(&self, tool: &str, key: &str) -> Path {
        let dot = |base: &str, add: &str| {
            if base.is_empty() {
                add.to_string()
            } else {
                format!("{base}.{add}")
            }
        };
        let map = dot(&self.table, "*");
        let is_map = classes()
            .get(tool)
            .is_some_and(|r| r.contains_key(map.as_str()));
        Path {
            table: if is_map { map } else { dot(&self.table, key) },
            shown: dot(&self.shown, key),
        }
    }
}

/// Check `v` and everything under it; `found` takes the first refusal.
fn walk(tool: &str, v: &Value, at: &Path, found: &mut Option<String>) {
    if found.is_some() {
        return;
    }
    match v {
        Value::String(s) => {
            if looks_like_a_tax_id(class_of(tool, &at.table), s) {
                *found = Some(ledger_core::settings::tin_refusal(&format!(
                    "the {} of {tool}",
                    at.shown
                )));
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                let child = Path {
                    table: format!("{}[]", at.table),
                    shown: format!("{}[{i}]", at.shown),
                };
                walk(tool, item, &child, found);
            }
        }
        Value::Object(map) => {
            // A structured value the engine reads whole.
            if class_of(tool, &at.table) == ENGINE {
                return;
            }
            let keys = format!("{}.#key", at.table);
            let keyed = classes()
                .get(tool)
                .is_some_and(|r| r.contains_key(keys.as_str()));
            for (k, item) in map {
                // A map's key is text too.
                if keyed && looks_like_a_tax_id(class_of(tool, &keys), k) {
                    *found = Some(ledger_core::settings::tin_refusal(&format!(
                        "a key of the {} of {tool}",
                        at.shown
                    )));
                    return;
                }
                let child = at.field(tool, k);
                walk(tool, item, &child, found);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::BooksServer;
    use serde_json::json;

    /// Every place in a tool's input schema a string can go, as a path:
    /// `a.b` for a field, `a[]` for an array's items, `a.*` for a map's values
    /// and `a.#key` for its keys, resolving `$ref`s and following `anyOf`.
    /// A node that says nothing of its type (any JSON) is a leaf too.
    pub(super) fn schema_leaves(root: &serde_json::Value) -> std::collections::BTreeSet<String> {
        use serde_json::Value;
        fn walk(
            root: &Value,
            v: &Value,
            path: &str,
            out: &mut std::collections::BTreeSet<String>,
            depth: usize,
        ) {
            assert!(depth < 12, "schema nesting too deep at {path}");
            if v.as_bool() == Some(true) {
                out.insert(path.to_string());
                return;
            }
            if let Some(r) = v.get("$ref").and_then(Value::as_str) {
                let key = r.rsplit('/').next().unwrap_or_default();
                let target = ["$defs", "definitions"]
                    .iter()
                    .find_map(|d| root.get(d).and_then(|d| d.get(key)))
                    .unwrap_or_else(|| panic!("schema {r} not found"));
                walk(root, target, path, out, depth + 1);
                return;
            }
            let mut typed = false;
            for k in ["anyOf", "oneOf", "allOf"] {
                if let Some(a) = v.get(k).and_then(Value::as_array) {
                    typed = true;
                    for x in a {
                        walk(root, x, path, out, depth + 1);
                    }
                }
            }
            if v.get("enum").is_some() || v.get("const").is_some() {
                typed = true;
                out.insert(path.to_string());
            }
            let names: Vec<&str> = match v.get("type") {
                Some(Value::String(s)) => vec![s.as_str()],
                Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
                _ => vec![],
            };
            for n in &names {
                typed = true;
                match *n {
                    "string" => {
                        out.insert(path.to_string());
                    }
                    "array" => {
                        if let Some(it) = v.get("items") {
                            walk(root, it, &format!("{path}[]"), out, depth + 1);
                        }
                    }
                    "object" => {
                        let join = |k: &str| {
                            if path.is_empty() {
                                k.to_string()
                            } else {
                                format!("{path}.{k}")
                            }
                        };
                        if let Some(p) = v.get("properties").and_then(Value::as_object) {
                            for (k, x) in p {
                                walk(root, x, &join(k), out, depth + 1);
                            }
                        }
                        match v.get("additionalProperties") {
                            Some(ap) if ap.is_object() || ap.as_bool() == Some(true) => {
                                out.insert(join("#key"));
                                walk(root, ap, &join("*"), out, depth + 1);
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            if !typed && !path.is_empty() {
                // `{}` or only a description: any JSON goes here.
                out.insert(path.to_string());
            }
        }
        let mut out = std::collections::BTreeSet::new();
        walk(root, root, "", &mut out, 0);
        out
    }

    fn write_tools() -> Vec<rmcp::model::Tool> {
        let mut tools: Vec<_> = BooksServer::tool_router()
            .list_all()
            .into_iter()
            .filter(|t| crate::server::calls::writes(&t.name))
            .collect();
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        assert!(tools.len() >= 80, "the write tools: {}", tools.len());
        tools
    }

    fn args(v: Value) -> Map<String, Value> {
        v.as_object().cloned().expect("an object")
    }

    /// INVARIANT 1: a new field is classed or the build fails.
    #[test]
    fn every_string_a_write_tool_takes_is_in_the_table() {
        let mut missing = Vec::new();
        for t in write_tools() {
            let root = Value::Object((*t.input_schema).clone());
            for leaf in schema_leaves(&root) {
                if !classes()
                    .get(&*t.name)
                    .is_some_and(|r| r.contains_key(leaf.as_str()))
                {
                    missing.push(format!("    (\"{}\", &[(\"{leaf}\", ?)]),", t.name));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "a write tool takes strings that text_fields::TABLE does not class (FREE unless an amount, a date, a lookup key ...):\n{}",
            missing.join("\n")
        );
    }

    /// INVARIANT 1, the other way: no row for a string no tool takes, no
    /// row twice, no tool that cannot write.
    #[test]
    fn the_table_names_only_strings_the_tools_take() {
        let mut leaves = std::collections::BTreeMap::new();
        for t in write_tools() {
            let root = Value::Object((*t.input_schema).clone());
            leaves.insert(t.name.to_string(), schema_leaves(&root));
        }
        let mut tools = std::collections::BTreeSet::new();
        for (tool, rows) in TABLE {
            assert!(tools.insert(*tool), "{tool} is in the table twice");
            let have = leaves
                .get(*tool)
                .unwrap_or_else(|| panic!("{tool} is no write tool"));
            let mut paths = std::collections::BTreeSet::new();
            for (path, _) in *rows {
                assert!(paths.insert(*path), "{tool} {path} is in the table twice");
                assert!(have.contains(*path), "{tool} takes no string at {path}");
            }
        }
        for (tool, have) in &leaves {
            assert_eq!(
                tools.contains(tool.as_str()),
                !have.is_empty(),
                "{tool}: listed exactly when it takes a string"
            );
        }
    }

    /// Every class is used, and the fields a tax id must never reach are
    /// FREE or STRICT: the table is no list of exemptions in disguise.
    #[test]
    fn the_free_text_is_checked_and_the_vault_is_the_only_input_for_a_tin() {
        let class = |tool: &str, path: &str| class_of(tool, path);
        for (tool, path) in [
            ("books.add_class", "name"),
            ("books.add_project", "name"),
            ("books.add_project", "notes"),
            ("books.draft_entry", "description"),
            ("books.reverse_entry", "description"),
            ("books.create_invoice", "memo"),
            ("books.create_invoice", "customer_ref"),
            ("books.create_invoice", "lines[].description"),
            ("books.create_bill", "vendor_ref"),
            ("books.attach_receipt", "description"),
            ("books.record_decision", "decision_md"),
            ("books.set_convention", "body_md"),
            ("books.raise_item", "title"),
            ("books.attest_period", "note"),
            ("books.rename_account", "reason"),
            ("books.void_document", "reason"),
        ] {
            assert_eq!(class(tool, path), Class::Free, "{tool} {path}");
        }
        for (tool, path) in [
            ("books.add_customer", "name"),
            ("books.add_vendor", "name"),
            ("books.add_party", "name"),
            ("books.update_party", "notes"),
            ("books.create_invoice", "bill_to.address"),
            ("books.add_account", "name"),
            ("books.mark_sent", "recipients[]"),
        ] {
            assert_eq!(class(tool, path), Class::Strict, "{tool} {path}");
        }
        let vault: Vec<_> = TABLE
            .iter()
            .flat_map(|(t, rows)| rows.iter().map(move |(p, c)| (*t, *p, *c)))
            .filter(|(_, _, c)| *c == Class::Exempt(Why::Vault))
            .map(|(t, p, _)| (t, p))
            .collect();
        assert_eq!(vault, [("books.set_tin", "tin"), ("books.set_w9", "tin")]);
        for why in [
            Why::Vault,
            Why::Lookup,
            Why::Id,
            Why::Amount,
            Why::Date,
            Why::Enum,
            Why::Digits,
            Why::Pattern,
            Why::Link,
            Why::Engine,
        ] {
            assert!(
                TABLE
                    .iter()
                    .any(|(_, rows)| rows.iter().any(|(_, c)| *c == Class::Exempt(why))),
                "{why:?} is used by no row"
            );
        }
    }

    /// SCENARIOS 1 to 3, with the shape of the words.
    #[test]
    fn a_tax_id_in_free_text_is_refused_and_a_reference_is_not() {
        let why = refusal(
            "books.add_class",
            Some(&args(json!({"name": "Acme 000-00-0000"}))),
        )
        .expect("refused");
        assert!(
            why.starts_with("the name of books.add_class looks like it holds a tax id"),
            "{why}"
        );
        assert!(
            why.contains("books.set_tin") && why.contains("W-9"),
            "{why}"
        );
        assert!(why.contains("nothing was changed"), "{why}");
        assert!(
            !why.contains("0000"),
            "the words never repeat the number: {why}"
        );
        // The allow-list: an invoice number, a phone, a long reference.
        for fine in [
            "inv 2026-11-0042",
            "call 801-555-0142",
            "ref 202611210042",
            "$1,234,567.89 paid 2026-11-21",
        ] {
            assert_eq!(
                refusal("books.create_invoice", Some(&args(json!({"memo": fine})))),
                None,
                "{fine}"
            );
        }
        // The vault's own input is no concern of this check.
        assert_eq!(
            refusal(
                "books.set_tin",
                Some(&args(
                    json!({"party": "Acme", "tin": "000-00-0000", "tin_type": "ssn"})
                ))
            ),
            None
        );
        assert_eq!(
            refusal(
                "books.set_w9",
                Some(&args(json!({"party": "Acme", "tin": "00-0000000"})))
            ),
            None
        );
        // ... but the same number beside it is refused.
        assert!(
            refusal(
                "books.set_tin",
                Some(&args(json!({"party": "000-00-0000", "tin": "x"})))
            )
            .is_none(),
            "a party is a lookup key: nothing is kept"
        );
        assert!(
            refusal(
                "books.set_w9",
                Some(&args(json!({"kept_at": "folder 000-00-0000"})))
            )
            .is_some()
        );
    }

    /// A nested string is found with its index, whichever depth.
    #[test]
    fn a_nested_string_is_found_where_it_is() {
        let why = refusal(
            "books.create_invoice",
            Some(&args(json!({
                "customer": "Acme",
                "lines": [
                    {"description": "ok", "amount": "1.00"},
                    {"description": "tax 00-0000000", "amount": "1.00"},
                ],
            }))),
        )
        .expect("refused");
        assert!(
            why.starts_with("the lines[1].description of books.create_invoice"),
            "{why}"
        );
        // A party's field is strict: ten digits are refused there ...
        assert!(
            refusal(
                "books.update_party",
                Some(&args(json!({"notes": "1234567890"})))
            )
            .is_some()
        );
        // ... and let through in a memo.
        assert!(
            refusal(
                "books.create_invoice",
                Some(&args(json!({"memo": "1234567890"})))
            )
            .is_none()
        );
        // A bill-to name is a party field.
        assert!(
            refusal(
                "books.create_invoice",
                Some(&args(json!({"bill_to": {"name": "Acme 123 45 6789"}})))
            )
            .is_some()
        );
    }

    /// The untyped value of a setting, and a structured convention, are
    /// the engine's to read; the maps' keys are text, a pattern is not.
    #[test]
    fn a_structured_value_is_the_engines_and_a_map_key_is_text() {
        let setting = json!({"changes": [{"key": "company.ein", "value": "00-0000000"}]});
        assert_eq!(refusal("books.set_setting", Some(&args(setting))), None);
        let note = json!({"changes": [{"key": "x", "value": 1}], "note": "ein 00-0000000"});
        assert!(refusal("books.set_setting", Some(&args(note))).is_some());
        let conv = json!({"structured": {"lines": [{"account": "6000", "text": "00-0000000"}]}});
        assert_eq!(refusal("books.set_convention", Some(&args(conv))), None);
        // A bank pattern is never kept; its account is a code.
        let rules = json!({"rules": {"payroll 00-0000000": "6000"}});
        assert_eq!(refusal("books.import_bank_csv", Some(&args(rules))), None);
        // A template's slot names are a vocabulary the tool parses: no
        // check, and a slot nothing matches is refused by the tool.
        let slots = json!({"accounts": {"00-0000000": "6000"}});
        assert!(refusal("books.draft_from_template", Some(&args(slots))).is_none());
    }

    /// INVARIANT 4: an unknown tool or field is FREE text.
    #[test]
    fn what_the_table_does_not_know_is_checked_as_free_text() {
        assert!(refusal("books.no_such", Some(&args(json!({"x": "000-00-0000"})))).is_some());
        assert!(
            refusal(
                "books.add_class",
                Some(&args(json!({"later": {"deep": ["00-0000000"]}})))
            )
            .is_some()
        );
        assert_eq!(refusal("books.add_class", None), None);
        assert_eq!(
            refusal(
                "books.add_class",
                Some(&args(json!({"name": 5, "x": null})))
            ),
            None
        );
    }
}
