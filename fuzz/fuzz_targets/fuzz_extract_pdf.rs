//! The PDF extractor on arbitrary bytes (plan P6): reading text, a page's
//! pixels and the page count is a refusal or an answer, never a panic or a
//! hang (D-32: it runs in the server's process).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = ledger_import::extract::pdf_text(data);
    let _ = ledger_import::extract::pdf_lines(data);
    if let Ok(pages) = ledger_import::extract::page_count(data) {
        for page in [0, pages.saturating_sub(1), pages] {
            let _ = ledger_import::extract::page_png(data, page, 64);
        }
    }
    let _ = ledger_import::extract::text_of(data, "application/pdf", "receipt.pdf");
});
