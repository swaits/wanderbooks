use crate::*;

impl Books<'_> {
    pub(crate) fn answer_iif(&self, q: &Query) -> Result<Value, Refusal> {
        let as_of = self.q_day(q)?;
        let iif = export_iif(self.l(), &self.ex.setup.cash_accounts, as_of).map_err(|e| {
            // An entry on more than one A/R or A/P account is BS-format-10's
            // refusal; every other IIF refusal stays BS-format-2's.
            match e {
                ledger_import::IifError::SeveralReceivablePayableAccounts { .. } => {
                    Refusal::of_reason("several_receivable_payable_accounts")
                }
                other => engine_refusal(RuleId::new(Area::Format, 2), other),
            }
        })?;
        match q.name.as_str() {
            "iif_lines" => {
                let mut lines: Vec<&str> = iif.text.split("\r\n").collect();
                if lines.last() == Some(&"") {
                    lines.pop();
                }
                Ok(Value::Array(
                    lines
                        .into_iter()
                        .map(|l| Value::String(l.to_string()))
                        .collect(),
                ))
            }
            "iif_line_ending" => {
                // The file's own bytes: every line break is CR LF
                // (wanderbooks' pick, cr-lf), every one a bare LF, or a mix
                // of the two, which is neither. The engine has no notion of
                // a host, so the example's `setup.host_line_ending` is not
                // read here.
                let text = iif.text.as_str();
                let lf = text.matches('\n').count();
                let crlf = text.matches("\r\n").count();
                let ending = if lf == 0 {
                    "none"
                } else if lf == crlf {
                    "cr-lf"
                } else if crlf == 0 {
                    "lf"
                } else {
                    "mixed"
                };
                Ok(json_figures([(
                    "line_ending".to_string(),
                    ending.to_string(),
                )]))
            }
            _ => Ok(Value::Array(
                iif.changes
                    .iter()
                    .map(|c| {
                        let what = match &c.field {
                            IifField::AccountName { code } => format!("name {code}"),
                            IifField::Memo { entry } => format!("memo {entry}"),
                            IifField::AccountNumber | IifField::AccountNumberLeftOut { .. } => {
                                "number".to_string()
                            }
                            IifField::AtTopLevel { code, .. } => format!("top level {code}"),
                            IifField::ClassName { class } => format!("class {class}"),
                        };
                        Value::Array(vec![
                            Value::String(what),
                            Value::String(c.books.clone()),
                            Value::String(c.written.clone()),
                        ])
                    })
                    .collect(),
            )),
        }
    }
}

/// The formats appendix's queries, and the test (another crate's) that
/// checks them against the engine.
pub(crate) const FORMAT_QUERIES: [(&str, &str); 3] = [
    (
        "iif_lines",
        "ledger-package the_bookspec_iif_example_matches_the_engine",
    ),
    (
        "iif_changes",
        "ledger-package the_bookspec_iif_example_matches_the_engine",
    ),
    (
        "iif_line_ending",
        "ledger-core the_runner_reports_every_claimed_rule_green",
    ),
];

/// A refused format query (BS-format-10): the appendix and the day, no
/// values.
pub(crate) fn check_format_refused(ex: &Example, x: &Expect) {
    assert_eq!(
        ex.appendix.as_deref(),
        Some("formats"),
        "{}: {} is a format's: the example is in appendix \"formats\"",
        ex.id,
        x.query
    );
    assert!(x.as_of.is_some(), "{}: {} gives as_of", ex.id, x.query);
    day(ex, x);
    assert!(
        x.after.is_none(),
        "{}: {}: a format query is of the books after every event",
        ex.id,
        x.query
    );
}

/// A format query's shape, checked here; its values are checked by
/// [`FORMAT_QUERIES`]' test.
pub(crate) fn check_format_shape(ex: &Example, x: &Expect) {
    check_format_refused(ex, x);
    if x.refused.is_some() {
        return;
    }
    match (x.query.as_str(), x.values(&ex.id)) {
        ("iif_lines", Values::Lines(_)) => {}
        ("iif_changes", Values::Rows(rows)) => {
            for r in rows {
                assert_eq!(r.len(), 3, "{}: iif_changes row {r:?}", ex.id);
            }
        }
        ("iif_line_ending", Values::Figures(f)) => {
            assert!(
                matches!(
                    f.get("line_ending").map(String::as_str),
                    Some("cr-lf" | "lf")
                ),
                "{}: iif_line_ending is cr-lf or lf: {f:?}",
                ex.id
            );
        }
        (q, _) => panic!("{}: {q}: values of the wrong shape", ex.id),
    }
}
