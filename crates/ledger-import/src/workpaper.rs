//! The audit workpaper as a PDF (plan P8; rules.md B-45).
//!
//! ```text
//! GOAL: one document a CPA or the owner can read that says what was
//!       checked, through which date and state of the books, and every
//!       finding; the same audit gives the same bytes.
//! SOURCE: IRS Pub. 583 (records); Rev. Proc. 98-25 §5.01 (the audit
//!         trail); CONVENTION (the statements' type and page).
//! INVARIANTS:
//!   1. Deterministic bytes: no clock, no random id; every text is data,
//!      never markup.
//!   2. The findings appear in the order given, each with severity, code,
//!      subject and message; the checks table lists every check the
//!      programme runs, found or not.
//! ```

use crate::pdf::PdfError;
use serde::Serialize;
use std::sync::OnceLock;
use typst::foundations::{Dict, IntoValue};
use typst_as_lib::{TypstEngine, TypstTemplateMainFile};

/// One finding as the page shows it.
#[derive(Clone, Debug, Serialize)]
pub struct WorkpaperFinding {
    /// failure, warning or info.
    pub severity: String,
    /// The finding code.
    pub code: String,
    /// What it is about, in words.
    pub subject: String,
    /// What was found.
    pub message: String,
}

/// One check the programme runs.
#[derive(Clone, Debug, Serialize)]
pub struct WorkpaperCheck {
    /// The finding code.
    pub code: String,
    /// failure, warning or info.
    pub severity: String,
    /// When it is reported.
    pub predicate: String,
}

/// Everything the page shows.
#[derive(Clone, Debug, Serialize)]
pub struct Workpaper {
    /// The company's name.
    pub company: String,
    /// The page's title.
    pub title: String,
    /// The audit's scope.
    pub scope: String,
    /// The date it covers.
    pub through: String,
    /// The books' figure head.
    pub through_seq: u64,
    /// The programme's version.
    pub program_version: String,
    /// The trial balance's hash.
    pub tb_hash: String,
    /// The findings' hash.
    pub findings_sha256: String,
    /// One line: how many of each severity.
    pub summary: String,
    /// The findings.
    pub findings: Vec<WorkpaperFinding>,
    /// The checks.
    pub checks: Vec<WorkpaperCheck>,
}

static TEMPLATE: &str = include_str!("../templates/workpaper.typ");

fn engine() -> &'static TypstEngine<TypstTemplateMainFile> {
    static ENGINE: OnceLock<TypstEngine<TypstTemplateMainFile>> = OnceLock::new();
    ENGINE.get_or_init(|| {
        TypstEngine::builder()
            .main_file(TEMPLATE)
            .fonts(crate::fonts::typst_fonts())
            .build()
    })
}

/// Render the workpaper as PDF bytes.
pub fn workpaper_pdf(w: &Workpaper) -> Result<Vec<u8>, PdfError> {
    let data = serde_json::to_string(w).map_err(|e| PdfError::Compile(e.to_string()))?;
    let mut inputs = Dict::new();
    inputs.insert("data".into(), data.into_value());
    let doc = engine()
        .compile_with_input(inputs)
        .output
        .map_err(|e| PdfError::Compile(format!("{e:?}")))?;
    typst_pdf::pdf(&doc, &typst_pdf::PdfOptions::default())
        .map_err(|e| PdfError::Save(format!("{e:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Workpaper {
        Workpaper {
            company: "Wanderland Labs LLC".into(),
            title: "Audit workpaper".into(),
            scope: "month".into(),
            through: "2026-10-31".into(),
            through_seq: 412,
            program_version: "audit-0.1.0-1".into(),
            tb_hash: "ab".repeat(32),
            findings_sha256: "cd".repeat(32),
            summary: "1 failure, 0 warnings".into(),
            findings: vec![WorkpaperFinding {
                severity: "failure".into(),
                code: "evidence_missing".into(),
                subject: "entry 7".into(),
                message: "entry 7 (2026-10-02) has no receipt: *not markup* #strong[x]".into(),
            }],
            checks: vec![WorkpaperCheck {
                code: "evidence_missing".into(),
                severity: "failure".into(),
                predicate: "a posted entry the policy covers has no receipt".into(),
            }],
        }
    }

    /// The same workpaper renders to the same bytes, a PDF.
    #[test]
    fn workpaper_pdf_is_deterministic() {
        let a = workpaper_pdf(&sample()).unwrap();
        let b = workpaper_pdf(&sample()).unwrap();
        assert!(a.starts_with(b"%PDF"));
        assert_eq!(a, b);
    }

    /// A different finding is a different document.
    #[test]
    fn a_different_finding_is_a_different_document() {
        let mut w = sample();
        let a = workpaper_pdf(&w).unwrap();
        w.findings.clear();
        assert_ne!(a, workpaper_pdf(&w).unwrap());
    }
}
