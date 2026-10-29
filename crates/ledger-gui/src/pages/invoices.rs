//! Invoices and bills: what customers owe the company (money in) and what
//! it owes vendors (money out), each figure derived from the document's
//! ledger lines that stand (rules.md B-38), the list tied to the
//! receivables or payables account (B-23); and one document's page, with
//! its lines and its timeline. Read only (D-28): the agent issues, pays
//! and voids them.

use super::assets::row;
use super::page;
use crate::app::Shell;
use crate::books::Books;
use crate::fmt;
use crate::nav::Route;
use crate::theme::pal;
use crate::widgets;
use chrono::NaiveDate;
use egui::Ui;
use egui_phosphor::regular as icon;
use ledger_core::{DocRef, DocState, DocStatus, DocumentLine};
use rust_decimal::Decimal;

/// Which documents the list shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    /// Anything still owed (partly paid and overdue included).
    #[default]
    Open,
    Overdue,
    Paid,
    Void,
    All,
}

impl Filter {
    fn keeps(self, st: &DocState) -> bool {
        match self {
            Filter::Open => !st.outstanding.is_zero() && st.status != DocStatus::Void,
            Filter::Overdue => st.overdue_days > 0,
            Filter::Paid => st.status == DocStatus::Paid,
            Filter::Void => st.status == DocStatus::Void,
            Filter::All => true,
        }
    }
}

#[derive(Default)]
pub struct State {
    pub filter: Filter,
}

/// One document as the list shows it.
struct Item {
    doc: DocRef,
    name: String,
    party: String,
    date: NaiveDate,
    due: NaiveDate,
    terms: Option<ledger_core::Terms>,
    state: Result<Option<DocState>, ledger_core::LedgerError>,
}

/// Every invoice (or bill) in the books, read as of today (one dated
/// ahead, as of its own date: Books::doc_today).
fn items(b: &Books, bills: bool) -> Vec<Item> {
    let l = &b.ledger;
    if bills {
        l.bills()
            .into_iter()
            .map(|d| Item {
                doc: DocRef::Bill(d.id),
                name: format!("Bill {}", d.id),
                party: l
                    .get_vendor(d.vendor_id)
                    .map(|v| v.name.clone())
                    .unwrap_or_default(),
                date: d.date,
                due: d.due_date,
                terms: d.terms.clone(),
                state: b.doc_today(DocRef::Bill(d.id)),
            })
            .collect()
    } else {
        l.invoices()
            .into_iter()
            .map(|d| Item {
                doc: DocRef::Invoice(d.id),
                name: format!("Invoice {}", d.id),
                party: l
                    .get_customer(d.customer_id)
                    .map(|c| c.name.clone())
                    .unwrap_or_default(),
                date: d.date,
                due: d.due_date,
                terms: d.terms.clone(),
                state: b.doc_today(DocRef::Invoice(d.id)),
            })
            .collect()
    }
}

/// The overview's figure for invoices (or bills): what the open ones
/// still owe, how many are overdue, and how many fall due within a week;
/// None when the books have none.
pub fn summary(b: &Books, bills: bool) -> Option<(Decimal, usize, usize)> {
    let all = items(b, bills);
    if all.is_empty() {
        return None;
    }
    let week = b.today + chrono::Duration::days(7);
    let (mut owed, mut late, mut soon) = (Decimal::ZERO, 0, 0);
    for it in &all {
        if let Ok(Some(st)) = &it.state
            && Filter::Open.keeps(st)
        {
            owed += st.outstanding;
            if st.overdue_days > 0 {
                late += 1;
            } else if it.due <= week {
                soon += 1;
            }
        }
    }
    Some((owed, late, soon))
}

fn route_of(doc: DocRef) -> Route {
    match doc {
        DocRef::Invoice(id) => Route::Invoice(id),
        DocRef::Bill(id) => Route::Bill(id),
    }
}

/// A status for people: "12 days late" while overdue, else its label;
/// "Dated ahead" for one dated after today.
fn status_text(st: &DocState, date: NaiveDate, today: NaiveDate) -> String {
    if date > today {
        "Dated ahead".into()
    } else if st.overdue_days > 0 {
        format!(
            "{} late",
            fmt::count(st.overdue_days as usize, "day", "days")
        )
    } else {
        st.status.label().into()
    }
}

/// Money in (invoices) or money out (bills).
pub fn show_list(ui: &mut Ui, b: &Books, bills: bool, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    page(ui, "invoices", |ui| {
        widgets::page_header(
            ui,
            "Invoices & Bills",
            "What customers owe the company and what it owes vendors, each figure read from the ledger, tied to its account. The bookkeeper issues, records and voids them.",
        );
        let mut side = bills;
        if widgets::segmented(
            ui,
            &mut side,
            &[(false, "Money in (invoices)"), (true, "Money out (bills)")],
        ) {
            shell.go(if side {
                Route::Payables
            } else {
                Route::Receivables
            });
        }
        ui.add_space(10.0);
        let all = items(b, bills);
        // The tiles count what is owed now, whatever the filter.
        let mut owed = Decimal::ZERO;
        let mut late = Decimal::ZERO;
        let mut late_n = 0usize;
        let mut open_n = 0usize;
        let mut unknown = false;
        for it in &all {
            match &it.state {
                Ok(Some(st)) if Filter::Open.keeps(st) => {
                    owed += st.outstanding;
                    open_n += 1;
                    if st.overdue_days > 0 {
                        late += st.outstanding;
                        late_n += 1;
                    }
                }
                Ok(_) => {}
                Err(_) => unknown = true,
            }
        }
        let (what, who) = if bills {
            ("You owe", "bills")
        } else {
            ("Owed to you", "invoices")
        };
        let tie = tie_out(b, bills);
        super::grid(ui, 3, 200.0, 3, |i, ui| {
            let (label, value, sub, how) = match i {
                0 => (
                    what,
                    if unknown {
                        "Can't compute".to_string()
                    } else {
                        fmt::dollars(owed)
                    },
                    Some((
                        fmt::count(
                            open_n,
                            &format!("open {}", &who[..who.len() - 1]),
                            &format!("open {who}"),
                        ),
                        p.muted,
                    )),
                    "What the open documents still owe, together, as of today.",
                ),
                1 => (
                    "Overdue",
                    fmt::dollars(late),
                    Some((
                        fmt::count(late_n, "past its due date", "past their due date"),
                        if late_n > 0 { p.warning } else { p.muted },
                    )),
                    "Of that, what is past its due date.",
                ),
                _ => (
                    "Ledger",
                    tie.0.clone(),
                    Some((tie.1.clone(), if tie.2 { p.muted } else { p.warning })),
                    "The receivables (or payables) account against its documents: they agree unless a line on it is no document's.",
                ),
            };
            widgets::kpi(
                ui,
                widgets::Kpi {
                    label,
                    value,
                    sub,
                    spark: None,
                    how,
                },
            );
        });
        ui.add_space(8.0);
        let st = &mut shell.state.invoices;
        ui.horizontal_wrapped(|ui| {
            for (f, label) in [
                (Filter::Open, "Open"),
                (Filter::Overdue, "Overdue"),
                (Filter::Paid, "Paid"),
                (Filter::Void, "Void"),
                (Filter::All, "All"),
            ] {
                if ui.selectable_label(st.filter == f, label).clicked() {
                    st.filter = f;
                }
            }
        });
        ui.add_space(8.0);
        let filter = st.filter;
        let mut rows: Vec<&Item> = all
            .iter()
            .filter(|it| match &it.state {
                Ok(Some(s)) => filter.keeps(s),
                Ok(None) => filter == Filter::All,
                Err(_) => true,
            })
            .collect();
        rows.sort_by_key(|it| (it.due, it.date, it.name.clone()));
        let mut go = None;
        widgets::card(ui, |ui| {
            if rows.is_empty() {
                let (title, text) = if all.is_empty() {
                    (
                        if bills { "No bills" } else { "No invoices" },
                        if bills {
                            "A vendor's bill appears here once the bookkeeper records it (books.create_bill)."
                        } else {
                            "An invoice appears here once the bookkeeper issues it (books.create_invoice)."
                        },
                    )
                } else {
                    ("Nothing here", "No document matches this filter.")
                };
                widgets::empty(ui, icon::RECEIPT, title, text);
                return;
            }
            let amt = widgets::AMOUNT_MIN + 16.0;
            let fixed = 110.0 + 104.0 * 2.0 + 120.0 + amt * 2.0 + if bills { 104.0 } else { 0.0 };
            let w = ui.available_width().max(fixed + 160.0);
            widgets::min_width(ui, "invoice_list", w, |ui| {
                let head = |t: &str, cw: f32, right: bool| (t.to_string(), cw, right, p.muted);
                let mut header = vec![
                    head("Document", 110.0, false),
                    head(if bills { "Vendor" } else { "Customer" }, 0.0, false),
                    head("Date", 104.0, false),
                    head("Due", 104.0, false),
                    head("Status", 120.0, false),
                    head("Total", amt, true),
                    head("Still owed", amt, true),
                ];
                if bills {
                    header.push(head("Discount by", 104.0, false));
                }
                row(ui, w, &header, None, true, false);
                widgets::rule(ui);
                for it in rows {
                    let (status, total, owed, late) = match &it.state {
                        Ok(Some(s)) => (
                            status_text(s, it.date, b.today),
                            fmt::money(s.total),
                            fmt::money(s.outstanding),
                            s.overdue_days > 0,
                        ),
                        Ok(None) => ("Unknown".into(), String::new(), String::new(), false),
                        Err(_) => ("Can't compute".into(), String::new(), String::new(), false),
                    };
                    let discount_by = it.terms.as_ref().and_then(|t| t.discount_by(it.date));
                    let discount_open = bills && discount_by.is_some_and(|d| d >= b.today);
                    let discount_soon = discount_open
                        && discount_by.is_some_and(|d| d <= b.today + chrono::Days::new(3));
                    let due_soon = bills
                        && matches!(&it.state, Ok(Some(s))
                            if !s.outstanding.is_zero() && s.status != DocStatus::Void)
                        && it.due <= b.today + chrono::Days::new(7);
                    let name = format!("{}, {}, {}, owes {}", it.name, it.party, status, owed);
                    let mut cells = vec![
                        (it.name.clone(), 110.0, false, p.text),
                        (it.party.clone(), 0.0, false, p.text),
                        (fmt::date(it.date), 104.0, false, p.text),
                        (
                            fmt::date(it.due),
                            104.0,
                            false,
                            if late || due_soon { p.warning } else { p.text },
                        ),
                        (
                            status.clone(),
                            120.0,
                            false,
                            if late { p.warning } else { p.muted },
                        ),
                        (total, amt, true, p.text),
                        (owed, amt, true, p.text),
                    ];
                    if bills {
                        cells.push((
                            discount_by.map(fmt::date).unwrap_or_default(),
                            104.0,
                            false,
                            if discount_soon {
                                p.warning
                            } else if discount_open {
                                p.text
                            } else {
                                p.muted
                            },
                        ));
                    }
                    let resp = row(ui, w, &cells, None, false, true);
                    // Named in full, for assistive tech (and the tests).
                    resp.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name)
                    });
                    if resp.clicked() {
                        go = Some(route_of(it.doc));
                    }
                }
            });
        });
        if let Some(r) = go {
            shell.go(r);
        }
    });
}

/// The tie-out of receivables (or payables) as of today: its value, a
/// line under it, and whether it agrees.
fn tie_out(b: &Books, bills: bool) -> (String, String, bool) {
    let role = if bills { "payables" } else { "receivables" };
    match b.ledger.control_check(b.today) {
        Err(e) => ("Can't compute".into(), e.to_string(), false),
        Ok(checks) => {
            let mine: Vec<_> = checks.iter().filter(|c| c.role == role).collect();
            if mine.is_empty() {
                return ("Agrees".into(), "No account holds them yet".into(), true);
            }
            let names: Vec<String> = mine.iter().map(|c| b.account_label(c.account)).collect();
            let loose: usize = mine.iter().map(|c| c.unattached.len()).sum();
            let odd: usize = mine.iter().map(|c| c.attention.len()).sum();
            let off = mine.iter().any(|c| !c.difference.is_zero());
            if off {
                ("Differs".into(), "An engine error: report it".into(), false)
            } else if loose > 0 || odd > 0 {
                let mut why = Vec::new();
                if loose > 0 {
                    why.push(fmt::count(
                        loose,
                        "line no document's",
                        "lines no document's",
                    ));
                }
                if odd > 0 {
                    why.push(fmt::count(
                        odd,
                        "document to look at",
                        "documents to look at",
                    ));
                }
                ("Look at".into(), why.join(", "), false)
            } else {
                ("Agrees".into(), format!("with {}", names.join(", ")), true)
            }
        }
    }
}

/// One invoice or bill: its figures, lines and timeline.
pub fn show_doc(ui: &mut Ui, b: &Books, doc: DocRef, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let l = &b.ledger;
    let (name, party, date, due, entry, lines): (
        String,
        String,
        NaiveDate,
        NaiveDate,
        u64,
        Vec<DocumentLine>,
    ) = match doc {
        DocRef::Invoice(id) => match l.get_invoice(id) {
            Some(d) => (
                format!("Invoice {id}"),
                l.get_customer(d.customer_id)
                    .map(|c| c.name.clone())
                    .unwrap_or_default(),
                d.date,
                d.due_date,
                d.entry_id,
                d.lines.clone(),
            ),
            None => return missing(ui, "No such invoice"),
        },
        DocRef::Bill(id) => match l.get_bill(id) {
            Some(d) => (
                format!("Bill {id}"),
                l.get_vendor(d.vendor_id)
                    .map(|v| v.name.clone())
                    .unwrap_or_default(),
                d.date,
                d.due_date,
                d.entry_id,
                d.lines.clone(),
            ),
            None => return missing(ui, "No such bill"),
        },
    };
    let mut go = None;
    page(ui, "invoice", |ui| {
        widgets::page_header(
            ui,
            &name,
            &format!(
                "{party} · dated {} · due {}",
                fmt::date(date),
                fmt::date(due)
            ),
        );
        let st = match b.doc_today(doc) {
            Ok(Some(st)) => st,
            Ok(None) => return missing(ui, "Not in the books as of today"),
            Err(e) => return missing(ui, &format!("Its figures can't be computed: {e}")),
        };
        let status = status_text(&st, date, b.today);
        super::grid(ui, 4, 180.0, 4, |i, ui| {
            let (label, value, how) = match i {
                0 => ("Total", fmt::dollars(st.total), "The sum of its lines."),
                1 => (
                    if matches!(doc, DocRef::Invoice(_)) {
                        "Received"
                    } else {
                        "Paid"
                    },
                    fmt::dollars(st.paid),
                    "Payments against it that still stand: a reversed payment no longer counts.",
                ),
                2 => (
                    "Still owed",
                    fmt::dollars(st.outstanding),
                    "Its total less what stands against it.",
                ),
                _ => ("Status", status.clone(), "As of today."),
            };
            widgets::kpi(
                ui,
                widgets::Kpi {
                    label,
                    value,
                    sub: None,
                    spark: None,
                    how,
                },
            );
        });
        ui.add_space(8.0);
        let amt = widgets::AMOUNT_MIN + 16.0;
        widgets::titled_card(
            ui,
            "Lines",
            |_| {},
            |ui| {
                let w = ui.available_width().max(160.0 + 200.0 + amt);
                widgets::min_width(ui, "invoice_lines", w, |ui| {
                    let head = |t: &str, cw: f32, right: bool| (t.to_string(), cw, right, p.muted);
                    row(
                        ui,
                        w,
                        &[
                            head("Description", 0.0, false),
                            head("Project", 200.0, false),
                            head("Amount", amt, true),
                        ],
                        None,
                        true,
                        false,
                    );
                    widgets::rule(ui);
                    for line in &lines {
                        let project = line
                            .class_id
                            .and_then(|c| l.get_class(c))
                            .map(|c| c.name.clone())
                            .unwrap_or_default();
                        let resp = row(
                            ui,
                            w,
                            &[
                                (line.description.clone(), 0.0, false, p.text),
                                (project, 200.0, false, p.muted),
                                (fmt::money(line.amount), amt, true, p.text),
                            ],
                            None,
                            false,
                            line.class_id.is_some(),
                        );
                        if let Some(c) = line.class_id
                            && resp.clicked()
                        {
                            go = Some(Route::Project(c));
                        }
                    }
                });
            },
        );
        ui.add_space(8.0);
        widgets::titled_card(
            ui,
            "Timeline",
            |_| {},
            |ui| {
                let w = ui.available_width().max(104.0 + 220.0 + 90.0 + amt);
                widgets::min_width(ui, "invoice_timeline", w, |ui| {
                    let head = |t: &str, cw: f32, right: bool| (t.to_string(), cw, right, p.muted);
                    row(
                        ui,
                        w,
                        &[
                            head("Date", 104.0, false),
                            head("What", 0.0, false),
                            head("Entry", 90.0, false),
                            head("Amount", amt, true),
                        ],
                        None,
                        true,
                        false,
                    );
                    widgets::rule(ui);
                    for t in l.doc_lines(doc) {
                        let what = match (t.movement, t.reversal) {
                            (ledger_core::Movement::Origin, false) => "Issued",
                            (ledger_core::Movement::Origin, true) => "Voided",
                            (ledger_core::Movement::Payment, false) => "Payment",
                            (ledger_core::Movement::Payment, true) => "Payment reversed",
                            (ledger_core::Movement::Discount, _) => "Discount",
                            (ledger_core::Movement::WriteOff, _) => "Written off",
                            (ledger_core::Movement::Adopted, _) => "Adopted",
                            (ledger_core::Movement::RoleMove, _) => "Moved with its account",
                        };
                        // A payment later reversed reads as such.
                        let what = if !t.reversal && !l.stands_on(t.entry, b.today) {
                            format!("{what} (reversed later)")
                        } else {
                            what.to_string()
                        };
                        let name = format!(
                            "{}: {what}, entry #{}, {}",
                            fmt::date(t.date),
                            t.entry,
                            fmt::money(t.amount)
                        );
                        let resp = row(
                            ui,
                            w,
                            &[
                                (fmt::date(t.date), 104.0, false, p.text),
                                (what, 0.0, false, p.text),
                                (format!("#{}", t.entry), 90.0, false, p.muted),
                                (fmt::money(t.amount), amt, true, p.text),
                            ],
                            None,
                            false,
                            true,
                        );
                        resp.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name)
                        });
                        if resp.on_hover_text("Open its entry").clicked() {
                            go = Some(Route::Entry(t.entry));
                        }
                    }
                });
            },
        );
        ui.add_space(8.0);
        if let DocRef::Invoice(id) = doc {
            pdf_actions(ui, b, id);
            ui.add_space(4.0);
        }
        if ui
            .link(format!(
                "{} Open the entry that recorded it (#{entry})",
                icon::ARROW_SQUARE_OUT
            ))
            .clicked()
        {
            go = Some(Route::Entry(entry));
        }
    });
    if let Some(r) = go {
        shell.go(r);
    }
}

/// "Save invoice PDF" and "Open": the invoice as of today, re-created
/// from the books (ledger_package::invoice_document, the same data the
/// server prints), saved in Downloads. Writes nothing to the books.
fn pdf_actions(ui: &mut Ui, b: &Books, id: ledger_core::InvoiceId) {
    let p = pal(ui.ctx());
    let said = ui.id().with(("invoice-pdf-said", id));
    ui.horizontal(|ui| {
        let save = ui
            .button(format!("{} Save invoice PDF", icon::FILE_PDF))
            .on_hover_text("As of today, in your Downloads folder");
        let open = ui
            .button(format!("{} Open", icon::ARROW_SQUARE_OUT))
            .on_hover_text("As of today, in your Downloads folder, opened with its app");
        if save.clicked() || open.clicked() {
            let result = ledger_package::invoice_document(
                &b.ledger,
                &b.settings.company,
                id,
                b.today,
                b.today,
            )
            .and_then(|doc| {
                ledger_import::invoice_pdf::invoice_pdf(&doc).map_err(|e| e.to_string())
            })
            .and_then(|bytes| {
                let stem = format!("invoice-{id}");
                if open.clicked() {
                    crate::export::save_and_open(&stem, "pdf", &bytes)
                } else {
                    crate::export::save(&stem, "pdf", &bytes)
                }
            })
            .map(|path| path.display().to_string());
            ui.data_mut(|d| d.insert_temp(said, result));
        }
    });
    if let Some(result) = ui.data(|d| d.get_temp::<Result<String, String>>(said)) {
        let (text, color) = match result {
            Ok(path) => (format!("Saved {path}"), p.muted),
            Err(why) => (format!("Couldn't save it: {why}"), p.warning),
        };
        ui.add(egui::Label::new(egui::RichText::new(text).size(12.0).color(color)).wrap());
    }
}

fn missing(ui: &mut Ui, title: &str) {
    widgets::card(ui, |ui| widgets::empty(ui, icon::RECEIPT, title, ""));
}
