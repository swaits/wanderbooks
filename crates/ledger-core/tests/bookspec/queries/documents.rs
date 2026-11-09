use crate::*;

/// The buckets of an aging, as the examples key them (BS-arap-1), beside
/// the engine's `AgingRow` fields they read.
const BUCKETS: [&str; 5] = ["current", "1-30", "31-60", "61-90", "90+"];

impl Books<'_> {
    /// The engine's `ar_aging` or `ap_aging` as of the day (BS-arap-1): the
    /// buckets in total, `total`, and each party's buckets, read straight
    /// from the engine's rows, never recomputed here.
    pub(crate) fn answer_aging(&self, q: &Query) -> Result<Value, Refusal> {
        let as_of = self.q_day(q)?;
        let rows = match q.of.as_deref() {
            Some("receivables") => self.l().ar_aging(as_of),
            Some("payables") => self.l().ap_aging(as_of),
            _ => return Err(self.missing("receivables or payables")),
        }
        .map_err(|e| engine_refusal(RuleId::new(Area::Arap, 1), e))?;
        let mut m = JsonMap::new();
        let mut totals = [Decimal::ZERO; 5];
        for row in &rows {
            let buckets = [
                row.current,
                row.days_30,
                row.days_60,
                row.days_90,
                row.days_90_plus,
            ];
            for ((name, owed), total) in BUCKETS.iter().zip(buckets).zip(totals.iter_mut()) {
                m.insert(format!("{}.{name}", row.name), Value::String(figure(owed)));
                *total += owed;
            }
        }
        for (name, total) in BUCKETS.iter().zip(totals) {
            m.insert(name.to_string(), Value::String(figure(total)));
        }
        m.insert(
            "total".to_string(),
            Value::String(figure(totals.iter().copied().sum())),
        );
        Ok(Value::Object(m))
    }

    /// The due date and discount date the document event's terms give
    /// (BS-arap-3), as the engine stored the document and its terms.
    pub(crate) fn answer_document_dates(&self, q: &Query) -> Result<Value, Refusal> {
        let n = q.document.ok_or_else(|| self.missing("document"))?;
        let entry = self.entry_or_refuse(n)?;
        let l = self.l();
        let (date, due, terms) = match (
            l.invoices().into_iter().find(|i| i.entry_id == entry),
            l.bills().into_iter().find(|b| b.entry_id == entry),
        ) {
            (Some(i), _) => (i.date, i.due_date, i.terms.clone()),
            (None, Some(b)) => (b.date, b.due_date, b.terms.clone()),
            (None, None) => return Err(self.missing("invoice or bill")),
        };
        let discount_by = terms
            .and_then(|t| t.discount_by(date))
            .map_or_else(|| "none".to_string(), |d| d.to_string());
        Ok(json_figures([
            ("due".to_string(), due.to_string()),
            ("discount_by".to_string(), discount_by),
        ]))
    }

    /// What the ledger lines tagged with the document event `document` come
    /// to as of the day (BS-arap-2), straight from the engine's `DocState`:
    /// `total`, `paid`, `discounted`, `written_off` and `owes` (its
    /// `outstanding`). The engine keeps no recovery movement (BS-arap-11
    /// reverse-then-plain-entry), so it gives no `recovered`.
    pub(crate) fn answer_document(&self, q: &Query) -> Result<Value, Refusal> {
        let n = q.document.ok_or_else(|| self.missing("document"))?;
        let as_of = self.q_day(q)?;
        let (doc, _) = self.document_of(n, "a document query");
        let state = match doc {
            ledger_core::DocRef::Invoice(id) => self.l().invoice_state(id, as_of),
            ledger_core::DocRef::Bill(id) => self.l().bill_state(id, as_of),
        }
        .map_err(|e| engine_refusal(RuleId::new(Area::Arap, 2), e))?
        .ok_or_else(|| self.missing("document, dated by the day asked"))?;
        Ok(json_figures([
            ("total".to_string(), figure(state.total)),
            ("paid".to_string(), figure(state.paid)),
            ("discounted".to_string(), figure(state.discounted)),
            ("written_off".to_string(), figure(state.written_off)),
            ("owes".to_string(), figure(state.outstanding)),
        ]))
    }

    /// The place of each invoice in the order of issue, and of the next
    /// (BS-arap-4): read from the number the engine stored at issue,
    /// `INV-0003` being the third, a voided invoice keeping its own.
    pub(crate) fn answer_invoice_numbers(&self, _q: &Query) -> Result<Value, Refusal> {
        let place = |number: &str| -> String {
            let digits: String = number.chars().filter(char::is_ascii_digit).collect();
            digits.trim_start_matches('0').to_string()
        };
        let l = self.l();
        let mut m = JsonMap::new();
        for (i, e) in self.ex.events.iter().enumerate() {
            if e.invoice.is_none() || i >= self.posted.len() {
                continue;
            }
            let entry = self.entry_or_refuse(i + 1)?;
            let invoice = l
                .invoices()
                .into_iter()
                .find(|inv| inv.entry_id == entry)
                .ok_or_else(|| self.missing("invoice"))?;
            let number = invoice
                .number
                .as_deref()
                .ok_or_else(|| self.missing("invoice number"))?;
            m.insert((i + 1).to_string(), Value::String(place(number)));
        }
        m.insert(
            "next".to_string(),
            Value::String(place(&l.next_invoice_number("INV-", 1))),
        );
        Ok(Value::Object(m))
    }
}

/// Check an `aging` expectation: every figure the engine gives, and no
/// other (the example lists everything it covers).
pub(crate) fn check_aging(b: &Books<'_>, x: &Expect) {
    check_figures(b, x, b.answer_aging(&x.query()));
}

/// Check a `document` expectation: the figures it names, as the engine's
/// document figures give them.
pub(crate) fn check_document(b: &Books<'_>, x: &Expect) {
    let answer = b.answer_document(&x.query());
    let Value::Object(got) =
        answer.unwrap_or_else(|r| panic!("{}: {}: {}", b.ex.id, x.query, r.reason))
    else {
        panic!("{}: {} answers a table", b.ex.id, x.query)
    };
    for (key, want) in x.figures(&b.ex.id) {
        assert_eq!(
            got.get(key).and_then(Value::as_str),
            Some(want.as_str()),
            "{}: {} {key}",
            b.ex.id,
            x.query
        );
    }
}

/// Check an `invoice_numbers` expectation: every invoice and the next,
/// and no other.
pub(crate) fn check_invoice_numbers(b: &Books<'_>, x: &Expect) {
    check_figures(b, x, b.answer_invoice_numbers(&x.query()));
}

/// Check a `document_dates` expectation: `due` and `discount_by`, both
/// given.
pub(crate) fn check_document_dates(b: &Books<'_>, x: &Expect) {
    check_figures(b, x, b.answer_document_dates(&x.query()));
}

pub(crate) fn check_figures(b: &Books<'_>, x: &Expect, answer: Result<Value, Refusal>) {
    let ex = b.ex;
    let Value::Object(got) =
        answer.unwrap_or_else(|r| panic!("{}: {}: {}", ex.id, x.query, r.reason))
    else {
        panic!("{}: {} answers a table", ex.id, x.query)
    };
    let keys: BTreeSet<String> = got.keys().cloned().collect();
    exactly(ex, x, &keys);
    for (key, want) in x.figures(&ex.id) {
        assert_eq!(
            got[key].as_str(),
            Some(want.as_str()),
            "{}: {} {key}",
            ex.id,
            x.query
        );
    }
}
