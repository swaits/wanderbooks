//! Tax categories at account creation, and `policy.require_tax_category`
//! (P3, defect 8; rules.md B-32), through a real books server as an
//! agent reaches it (common.rs).
//!
//! ```text
//! GOAL: every revenue and expense account maps to a line of the return,
//!       or the human hears that it doesn't; a company that refuses
//!       uncategorized accounts gets exactly that, never a refused
//!       invoice.
//! SOURCE: rules.md B-32; plan/served.md P3 defect 8 and "Settled in
//!         design" (warn by default, headers exempt).
//! ```
//!
//! Fixtures are fictional (Fernhill Labs LLC).

use common::{Books, Wire};
use serde_json::{Value, json};

mod common;

struct Mcp {
    wire: Wire,
    next_id: u64,
}

impl Mcp {
    /// A session with books that keep their own settings, initialized.
    fn spawn(books: &Books) -> Self {
        let mut m = Self {
            wire: books.wire(),
            next_id: 1,
        };
        m.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "spec-tax-policy", "version": "0"}}),
        );
        m.wire
            .send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}}));
        m
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.wire.send(&msg);
        let resp = self
            .wire
            .recv(common::TIMEOUT)
            .unwrap_or_else(|| panic!("no answer to {method}: {:?}", self.wire.refused));
        assert_eq!(resp["id"], id, "id mismatch in {resp}");
        assert!(resp.get("error").is_none(), "{method}: {}", resp["error"]);
        resp
    }

    fn result(&mut self, tool: &str, args: Value) -> Value {
        let resp = self.request("tools/call", json!({"name": tool, "arguments": args}));
        resp["result"].clone()
    }

    /// Call a tool; panic on a refusal. Returns structuredContent.
    fn call(&mut self, tool: &str, args: Value) -> Value {
        let result = self.result(tool, args);
        assert_ne!(
            result.get("isError"),
            Some(&json!(true)),
            "{tool}: {}",
            result["content"]
        );
        result.get("structuredContent").cloned().unwrap_or_else(|| {
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
        })
    }

    /// Call a tool expecting a refusal; returns its text.
    fn call_err(&mut self, tool: &str, args: Value) -> String {
        let result = self.result(tool, args);
        assert_eq!(
            result.get("isError"),
            Some(&json!(true)),
            "{tool}: {result}"
        );
        result["content"][0]["text"].as_str().unwrap().to_string()
    }

    fn events(&mut self) -> u64 {
        self.call("books.verify", json!({"blobs": false}))["events"]
            .as_u64()
            .unwrap()
    }

    fn set_policy(&mut self, value: Value) {
        self.call(
            "books.set_setting",
            json!({"changes": [{"key": "policy.require_tax_category", "value": value}],
                   "decided_by": "Tobin Harlow", "decided_on": "2026-10-20"}),
        );
    }
}

fn add(m: &mut Mcp, args: Value) -> Value {
    m.call("books.add_account", args)
}

fn chart_codes(m: &mut Mcp) -> Vec<String> {
    m.call("books.chart", json!({}))["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["code"].as_str().unwrap().to_string())
        .collect()
}

/// Defect 8, repro 1.4.1: books.add_account of a revenue or expense
/// account without a tax category creates it and warns, naming it and
/// the tools to call; an asset, liability, equity or header account, or
/// one given its category, has no warning. A category that doesn't fit
/// the account or the return is refused, and a deductible percent needs
/// a category.
#[test]
fn add_account_warns_without_tax_category_defect_8() {
    let books = Books::new("warn");
    let mut m = Mcp::spawn(&books);
    let out = add(
        &mut m,
        json!({"code": "6150", "name": "Software", "account_type": "expense"}),
    );
    assert!(out["account_id"].as_u64().is_some());
    assert_eq!(
        out["warnings"],
        json!([
            "6150 Software has no tax category: the tax summary will list it as uncategorized. Propose one from books.tax_categories and set it with books.set_tax_category once the human agrees."
        ])
    );
    let out = add(
        &mut m,
        json!({"code": "4000", "name": "Sales", "account_type": "revenue"}),
    );
    assert_eq!(out["warnings"].as_array().unwrap().len(), 1, "{out}");
    for args in [
        json!({"code": "1000", "name": "Business Checking", "account_type": "asset"}),
        json!({"code": "2000", "name": "Card", "account_type": "liability"}),
        json!({"code": "3000", "name": "Capital", "account_type": "equity"}),
        json!({"code": "6000", "name": "Operating", "account_type": "expense", "placeholder": true}),
        json!({"code": "6160", "name": "Hosting", "account_type": "expense", "tax_category": "other_deductions"}),
        json!({"code": "6170", "name": "Meals", "account_type": "expense", "tax_category": "meals", "deductible_percent": "50"}),
    ] {
        let out = add(&mut m, args.clone());
        assert!(out.get("warnings").is_none(), "{args}: {out}");
    }
    let chart = m.call("books.chart", json!({}));
    let meals = chart["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["code"] == "6170")
        .unwrap()
        .clone();
    assert_eq!(meals["tax_category"], "meals");
    assert_eq!(meals["deductible_percent"], "50");
    let before = m.events();
    for (args, says) in [
        (
            json!({"code": "4100", "name": "Fees", "account_type": "revenue", "tax_category": "meals"}),
            "expense",
        ),
        (
            json!({"code": "6180", "name": "X", "account_type": "expense", "tax_category": "no_such"}),
            "unknown tax category",
        ),
        (
            json!({"code": "6190", "name": "Y", "account_type": "expense", "deductible_percent": "50"}),
            "needs a tax_category",
        ),
        (
            json!({"code": "6195", "name": "Z", "account_type": "expense", "tax_category": "other_deductions", "deductible_percent": "150"}),
            "between 0 and 100",
        ),
    ] {
        let err = m.call_err("books.add_account", args.clone());
        assert!(err.contains(says), "{args}: {err}");
    }
    assert_eq!(m.events(), before, "refusals write nothing");
    // Needs attention's list, for the agent too.
    let digest = m.call("books.digest", json!({"since": "2026-01-01"}));
    assert_eq!(
        digest["uncategorized_accounts"],
        json!(["4000 Sales", "6150 Software"])
    );
}

/// Defect 8 under `policy.require_tax_category = refuse`: books.add_account
/// without a category is refused and the chart is unchanged; with one it
/// is created (account.added and account.tax_set); import_chart_csv with
/// one uncategorized expense row creates nothing and says why; clearing a
/// needed category is refused; a bill on books without 6999 still posts,
/// its result naming the account it created, which Needs attention (the
/// digest) lists. Back to warn, the same account is created with a
/// warning.
#[test]
fn require_tax_category_refuses_defect_8() {
    let books = Books::new("refuse");
    let mut m = Mcp::spawn(&books);
    add(
        &mut m,
        json!({"code": "1000", "name": "Business Checking", "account_type": "asset"}),
    );
    add(
        &mut m,
        json!({"code": "6150", "name": "Software", "account_type": "expense", "tax_category": "other_deductions"}),
    );
    m.set_policy(json!("refuse"));
    let settings = m.call("books.settings", json!({}));
    assert!(
        settings.to_string().contains("policy.require_tax_category"),
        "{settings}"
    );
    let before = m.events();
    let codes = chart_codes(&mut m);
    let err = m.call_err(
        "books.add_account",
        json!({"code": "6160", "name": "Hosting", "account_type": "expense"}),
    );
    assert!(
        err.contains("policy.require_tax_category is refuse")
            && err.contains("6160 Hosting")
            && err.contains("books.tax_categories"),
        "{err}"
    );
    assert_eq!(chart_codes(&mut m), codes);
    assert_eq!(m.events(), before);
    let out = add(
        &mut m,
        json!({"code": "6160", "name": "Hosting", "account_type": "expense", "tax_category": "other_deductions"}),
    );
    assert!(out.get("warnings").is_none(), "{out}");
    assert_eq!(m.events(), before + 2, "account.added and account.tax_set");
    // A header is exempt.
    add(
        &mut m,
        json!({"code": "6000", "name": "Operating", "account_type": "expense", "placeholder": true}),
    );
    // The import: refused whole, saying why.
    let before = m.events();
    let err = m.call_err(
        "books.import_chart_csv",
        json!({"csv_text": "code,name,type,parent_code,tax_category\n6200,Rent,expense,,rent_property\n6210,Misc,expense,,\n"}),
    );
    assert!(
        err.contains("6210") && err.contains("fifth column") && err.contains("never a header"),
        "{err}"
    );
    assert!(!chart_codes(&mut m).contains(&"6200".to_string()));
    assert_eq!(m.events(), before);
    // Clearing a needed category.
    let err = m.call_err(
        "books.set_tax_category",
        json!({"account": "6150", "category": null}),
    );
    assert!(err.contains("keeps a tax category"), "{err}");
    m.call(
        "books.set_tax_category",
        json!({"account": "6150", "category": "supplies"}),
    );
    // A bill creates General Expenses 6999 without a category: never
    // refused, named in the result's warnings and the digest.
    let vendor = m.call("books.add_vendor", json!({"name": "Coastline Hosting"}));
    let bill = m.call(
        "books.create_bill",
        json!({"party_id": vendor["id"], "date": "2026-02-01", "due_date": "2026-03-01",
               "lines": [{"description": "hosting", "amount": "42.00"}]}),
    );
    assert_eq!(
        bill["warnings"],
        json!([
            "6999 General Expenses has no tax category: the tax summary will list it as uncategorized. Propose one from books.tax_categories and set it with books.set_tax_category once the human agrees."
        ])
    );
    let digest = m.call("books.digest", json!({"since": "2026-01-01"}));
    assert_eq!(
        digest["uncategorized_accounts"],
        json!(["6999 General Expenses"])
    );
    // A second bill creates nothing, and warns of nothing.
    let again = m.call(
        "books.create_bill",
        json!({"party_id": vendor["id"], "date": "2026-02-02", "due_date": "2026-03-02",
               "lines": [{"description": "hosting", "amount": "1.00"}]}),
    );
    assert!(again.get("warnings").is_none(), "{again}");
    // Back to warn (cleared): created, with the warning.
    m.set_policy(Value::Null);
    let out = add(
        &mut m,
        json!({"code": "6210", "name": "Misc", "account_type": "expense"}),
    );
    assert_eq!(out["warnings"].as_array().unwrap().len(), 1, "{out}");
    let out = m.call(
        "books.import_chart_csv",
        json!({"csv_text": "code,name,type,parent_code\n6220,Postage,expense,\n6230,Fees,revenue,\n1100,Petty Cash,asset,\n"}),
    );
    assert_eq!(out["count"], 3);
    assert_eq!(out["warnings"].as_array().unwrap().len(), 2, "{out}");
}

/// rules.md B-31 for new accounts: a name another account under the same
/// parent has, ignoring capitals (top-level accounts are siblings too),
/// or one holding nine digits in a row, is refused, for books.add_account
/// and books.import_chart_csv alike; under another parent the name is
/// free.
#[test]
fn a_new_account_named_like_a_sibling_is_refused() {
    let books = Books::new("names");
    let mut m = Mcp::spawn(&books);
    add(
        &mut m,
        json!({"code": "6000", "name": "Operating", "account_type": "expense", "placeholder": true}),
    );
    let ops = m.call("books.chart", json!({}))["accounts"][0]["id"].clone();
    add(
        &mut m,
        json!({"code": "6150", "name": "Software", "account_type": "expense", "parent": ops, "tax_category": "other_deductions"}),
    );
    let before = m.events();
    for (args, says) in [
        (
            json!({"code": "6151", "name": "SOFTWARE", "account_type": "expense", "parent": ops}),
            "6150 Software already has the name",
        ),
        (
            json!({"code": "6001", "name": " operating ", "account_type": "expense"}),
            "6000 Operating already has the name",
        ),
        (
            json!({"code": "1000", "name": "Checking 123456789", "account_type": "asset"}),
            "nine digits",
        ),
    ] {
        let err = m.call_err("books.add_account", args.clone());
        assert!(err.contains(says), "{args}: {err}");
    }
    let err = m.call_err(
        "books.import_chart_csv",
        json!({"csv_text": "code,name,type,parent_code\n1000,Cash,asset,\n1010,cash,asset,\n"}),
    );
    assert!(err.contains("1010") && err.contains("1000 Cash"), "{err}");
    assert_eq!(m.events(), before);
    // Top level: no sibling of the child.
    add(
        &mut m,
        json!({"code": "6200", "name": "Software", "account_type": "expense", "tax_category": "other_deductions"}),
    );
}

/// rules.md B-32: a gain on business property (`form_4797_gain_loss`, the
/// disposal account a refusing company must categorize) is exported apart
/// in the tax summary, JSON and CSV alike: at its book amount, with no tax
/// amount, and outside income and both nets (Form 4797 figures it from
/// the tax basis; never Schedule C income, never ordinary income the
/// summary could place).
#[test]
fn a_gain_on_business_property_is_exported_apart() {
    let books = Books::new("form4797");
    let mut m = Mcp::spawn(&books);
    for args in [
        json!({"code": "1000", "name": "Business Checking", "account_type": "asset"}),
        json!({"code": "4000", "name": "Sales", "account_type": "revenue", "tax_category": "gross_receipts"}),
        json!({"code": "4700", "name": "Gain on disposal", "account_type": "revenue", "tax_category": "form_4797_gain_loss"}),
    ] {
        add(&mut m, args);
    }
    for (cr, amount) in [("4000", "10.00"), ("4700", "7.50")] {
        let e = m.call(
            "books.draft_entry",
            json!({"date": "2026-03-02", "description": "x", "no_party_reason": "test"}),
        );
        let id = e["entry_id"].clone();
        for (account, side) in [("1000", "debit"), (cr, "credit")] {
            m.call(
                "books.add_line",
                json!({"entry_id": id, "account": account, "side": side, "amount": amount}),
            );
        }
        m.call(
            "books.post_entry",
            json!({"entry_id": id, "confirmed": true}),
        );
    }
    let range = |format: &str| json!({"report": "tax_summary", "format": format, "start": "2026-01-01", "end": "2026-12-31"});
    let out = m.call("books.export", range("json"));
    let j: Value = serde_json::from_str(out["content"].as_str().unwrap()).unwrap();
    assert_eq!(
        j["form_4797"],
        json!([{"category": "form_4797_gain_loss", "label": "Gain or loss on business property (Form 4797)",
                "accounts": ["4700 Gain on disposal"], "book": "7.50"}])
    );
    assert_eq!(j["income"].as_array().unwrap().len(), 1, "{j}");
    assert_eq!(j["income"][0]["category"], "gross_receipts");
    assert_eq!(j["book_net"], "10.00");
    assert_eq!(j["tax_net"], "10.00");
    let out = m.call("books.export", range("csv"));
    let csv = out["content"].as_str().unwrap();
    assert!(
        csv.lines().any(|l| l
            == "form_4797,form_4797_gain_loss,Gain or loss on business property (Form 4797),4700 Gain on disposal,7.50,"),
        "{csv}"
    );
    assert!(
        !csv.lines().any(|l| l.starts_with("income,form_4797")),
        "{csv}"
    );
}
