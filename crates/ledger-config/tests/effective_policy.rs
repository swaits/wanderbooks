//! The receipt policy an older books dir's files set, as adoption moves it
//! in: config.json's when it sets one, else the settings file's threshold.

use ledger_config::{BooksSettings, effective_receipt_policy_text};

#[test]
fn config_json_wins_else_the_settings_file() {
    let settings = BooksSettings {
        receipt_threshold: Some("0.00".into()),
        ..BooksSettings::default()
    };
    // No config.json: the settings file's threshold.
    let p = effective_receipt_policy_text(None, &settings).unwrap();
    assert_eq!(p.require_receipts_above, "0.00");
    // config.json's policy, whole.
    let p = effective_receipt_policy_text(
        Some(
            r#"{"receipt_policy": {"require_receipts_above": "10.00", "receipt_exempt_accounts": ["6999"]}}"#,
        ),
        &settings,
    )
    .unwrap();
    assert_eq!(p.require_receipts_above, "10.00");
    assert_eq!(p.receipt_exempt_accounts, ["6999"]);
    // A broken one is refused.
    let err = effective_receipt_policy_text(Some("{ not json"), &settings).unwrap_err();
    assert!(err.starts_with("bad config"), "{err}");
}
