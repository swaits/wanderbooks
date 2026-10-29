//! The XLSX extractor on arbitrary bytes (plan P6): a zip or not, a sheet
//! or not, it is a value or a refusal, never a panic; a cell's number is
//! read as the decimal string the XML holds.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = ledger_import::extract::xlsx_text(data);
    let _ = ledger_import::extract::text_of(
        data,
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "sheet.xlsx",
    );
});
