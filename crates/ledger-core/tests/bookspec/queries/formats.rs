use crate::*;

impl Books<'_> {
    pub(crate) fn answer_iif(&self, q: &Query) -> Result<Value, Refusal> {
        let as_of = self.q_day(q)?;
        let iif = export_iif(self.l(), &self.ex.setup.cash_accounts, as_of)
            .map_err(|e| engine_refusal(RuleId::new(Area::Format, 2), e))?;
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
pub(crate) const FORMAT_QUERIES: [(&str, &str); 2] = [
    (
        "iif_lines",
        "ledger-package the_bookspec_iif_example_matches_the_engine",
    ),
    (
        "iif_changes",
        "ledger-package the_bookspec_iif_example_matches_the_engine",
    ),
];

/// A format query's shape, checked here; its values are checked by
/// [`FORMAT_QUERIES`]' test.
pub(crate) fn check_format_shape(ex: &Example, x: &Expect) {
    assert_eq!(
        ex.appendix.as_deref(),
        Some("formats"),
        "{}: {} is a format's: the example is in appendix \"formats\"",
        ex.id,
        x.query
    );
    // ledger-package reads the as_of itself: given, never defaulted.
    assert!(x.as_of.is_some(), "{}: {} gives as_of", ex.id, x.query);
    day(ex, x);
    assert!(
        x.after.is_none(),
        "{}: {}: a format query is of the books after every event",
        ex.id,
        x.query
    );
    match (x.query.as_str(), x.values(&ex.id)) {
        ("iif_lines", Values::Lines(_)) => {}
        ("iif_changes", Values::Rows(rows)) => {
            for r in rows {
                assert_eq!(r.len(), 3, "{}: iif_changes row {r:?}", ex.id);
            }
        }
        (q, _) => panic!("{}: {q}: values of the wrong shape", ex.id),
    }
}
