//! The receipt policy in force when the settings come from files:
//! config.json's when it sets one, else the settings file's threshold.

use ledger_config::{BooksSettings, effective_receipt_policy};

fn dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("ledger-config-policy-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn config_json_wins_else_the_settings_file() {
    let d = dir("files");
    let json = d.join("config.json");
    let settings = BooksSettings {
        receipt_threshold: Some("0.00".into()),
        ..BooksSettings::default()
    };
    // No config.json: the settings file's threshold.
    let p = effective_receipt_policy(&json, &settings).unwrap();
    assert_eq!(p.require_receipts_above, "0.00");
    // config.json's policy, whole.
    std::fs::write(
        &json,
        r#"{"receipt_policy": {"require_receipts_above": "10.00", "receipt_exempt_accounts": ["6999"]}}"#,
    )
    .unwrap();
    let p = effective_receipt_policy(&json, &settings).unwrap();
    assert_eq!(p.require_receipts_above, "10.00");
    assert_eq!(p.receipt_exempt_accounts, ["6999"]);
    // A broken one is refused, by name.
    std::fs::write(&json, "{ not json").unwrap();
    let err = effective_receipt_policy(&json, &settings).unwrap_err();
    assert!(
        err.starts_with(&format!("bad config {}", json.display())),
        "{err}"
    );
    std::fs::remove_dir_all(&d).unwrap();
}
