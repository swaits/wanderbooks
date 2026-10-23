//! Files in (plan 4.7, P2 slice 5, D-28): `books.begin_upload`, and the
//! bytes a tool is given, as an upload or inline.
//!
//! ```text
//! GOAL: an agent files a document of any size up to the limit without
//!       its bytes passing through its context, and a small one inline,
//!       both exactly as the agent declared them.
//! SOURCE: plan 4.7 (begin_upload; "content_base64 (≤256 KiB + required
//!         sha256)"; guard base64_corrupt), 2.1 ("256 KiB with a mandatory
//!         matching sha256"), D-28 (the agent does it all).
//! INVARIANTS:
//!   1. A tool's bytes are an upload made by the same token, or inline
//!      base64 of at most INLINE_MAX bytes whose SHA-256 is the one given;
//!      never both, never neither. An upload is used up only by the call
//!      that files it (TakenFile::filed); a call that fails leaves it for
//!      the next.
//!   2. begin_upload makes nothing for a receipt or document whose bytes
//!      are stored already: it names the attachment (not withdrawn) that
//!      holds them.
//!   3. An upload is kept for the token that began it, and so is an
//!      export: a call with no token (never one the books server lets
//!      through) begins and takes none.
//!   4. An upload is filed only by a tool of the purpose it was begun for
//!      (a receipt or document, a CSV import, a W-9); another tool's
//!      refusal leaves it for the right one.
//!   5. Inline text longer than INLINE_MAX bytes' base64 is refused
//!      before it is decoded.
//!   6. A file the books make for an agent (D-29) is handed over as an
//!      export the caller's token fetches with `wanderbooks download`;
//!      never inline, never by a path.
//! ```

use super::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The most bytes given inline (plan 2.1: 256 KiB).
pub(crate) const INLINE_MAX: usize = 256 * 1024;

/// Why a call with no token begins, takes or is handed no file
/// (INVARIANT 3).
const NO_CALLER: &str =
    "this call came without a token for these books: a file is kept only for the token that asked";

/// What the bytes of a file a tool files came as (INVARIANT 1), and the
/// upload purposes the tool takes (INVARIANT 4).
pub(crate) struct Given<'a> {
    pub upload_id: Option<&'a str>,
    pub content_base64: Option<&'a str>,
    pub sha256: Option<&'a str>,
    pub purposes: &'a [&'a str],
}

/// What attach_receipt and file_document take.
pub(crate) const FILED: &[&str] = &["receipt", "document"];
/// What the CSV imports take.
pub(crate) const IMPORTED: &[&str] = &["import"];
/// What set_w9 takes.
pub(crate) const W9: &[&str] = &["w9"];

/// The bytes, and the name and type an upload declared.
pub(crate) struct TakenFile {
    pub bytes: Vec<u8>,
    pub file_name: Option<String>,
    pub mime: Option<String>,
    /// The upload's hold, if it came as one (INVARIANT 1).
    pub claim: Option<ledger_store::uploads::Claim>,
}

/// ` --token NAME` for a command given to the agent, when its token isn't
/// the one the commands use by default: the command runs as the caller.
pub(crate) fn token_flag() -> String {
    CALLER.with(|c| match c.borrow().as_ref() {
        Some((_, name)) if name != ledger_store::auth::DEFAULT_NAME => {
            format!(" --token {name}")
        }
        _ => String::new(),
    })
}

/// The bytes are in the books: an upload they came as is used up
/// (INVARIANT 1). Dropped instead, it is given back.
pub(crate) fn filed(claim: Option<ledger_store::uploads::Claim>) {
    if let Some(c) = claim {
        c.spend();
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BeginUploadParams {
    /// The file's SHA-256, hex (`sha256sum FILE`).
    pub sha256: String,
    /// Its size in bytes (at most 25 MiB).
    pub size: u64,
    /// Its MIME type, e.g. "application/pdf".
    pub mime_type: String,
    /// Its name, e.g. "receipt.pdf".
    pub file_name: String,
    /// What it is for: receipt, document, import (a bank or chart CSV), or
    /// w9 (books.set_w9).
    pub purpose: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BeginUploadOut {
    /// The upload's id: name it in the tool that files it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_id: Option<String>,
    /// The command that sends the file (run it with the file's path).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// When the upload lapses (UTC).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// The most bytes taken.
    pub max_bytes: u64,
    /// These bytes are stored already, in this attachment: link it
    /// (books.link_attachment) instead of filing a copy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub existing_attachment_id: Option<u64>,
}

/// `name` made a name the checks take: control and invisible characters
/// and separators dropped or made `_`, cut to fit; `file` if nothing is
/// left.
fn plain_file_name(name: &str) -> String {
    let mut out: String = name
        .chars()
        .filter(|c| !ledger_store::uploads::is_hidden_char(*c))
        .map(|c| {
            if c.is_control() || c == '/' || c == '\\' {
                '_'
            } else {
                c
            }
        })
        .collect();
    while out.len() > ledger_store::uploads::MAX_NAME_BYTES {
        out.pop();
    }
    let out = out.trim().to_string();
    if ledger_store::uploads::file_name_problem(&out).is_none() {
        out
    } else {
        "file".to_string()
    }
}

impl BooksServer {
    /// Hand `bytes` over as `file_name` (INVARIANT 6): kept as an export
    /// for the caller's token.
    pub(crate) fn hand_over(
        &self,
        book: &str,
        file_name: &str,
        mime: &str,
        bytes: Vec<u8>,
    ) -> Result<HandedFile, String> {
        // A name kept before names were checked (a W-9 sealed earlier) is
        // made plain rather than refused: the file is the point.
        let file_name = match ledger_store::uploads::file_name_problem(file_name) {
            None => file_name.to_string(),
            Some(_) => plain_file_name(file_name),
        };
        let token = self.caller_token().ok_or(NO_CALLER)?;
        let kept = self
            .exports
            .put(
                &token,
                &file_name,
                mime,
                &bytes,
                chrono::Utc::now().timestamp(),
            )
            .map_err(|e| e.to_string())?;
        // Always a real time: a kept export expires EXPORT_TTL from now.
        let expires_at = chrono::DateTime::from_timestamp(kept.expires, 0)
            .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
            .ok_or("the file's expiry is past the calendar")?;
        Ok(HandedFile {
            download: format!(
                "wanderbooks download --book {book}{} {} --to <a folder>",
                token_flag(),
                kept.id
            ),
            expires_at,
            size_bytes: bytes.len() as u64,
            sha256: ledger_store::journal::sha256_hex(&bytes),
            file_name,
            mime_type: mime.to_string(),
        })
    }

    /// The token of the call running on this thread (its id), if any.
    fn caller_token(&self) -> Option<String> {
        CALLER.with(|c| c.borrow().as_ref().map(|(id, _)| id.clone()))
    }

    /// The bytes a tool was given (INVARIANT 1).
    pub(crate) fn take_file(&self, given: Given<'_>) -> Result<TakenFile, String> {
        match (given.upload_id, given.content_base64) {
            (Some(_), Some(_)) => Err("give upload_id or content_base64, not both".into()),
            (None, None) => Err(
                "the file's bytes are missing: upload it (books.begin_upload), or give content_base64 with its sha256 (at most 256 KiB)"
                    .into(),
            ),
            (Some(id), None) => {
                // INVARIANT 3.
                let token = self.caller_token().ok_or(NO_CALLER)?;
                let got = self
                    .uploads
                    .take(id.trim(), &token, chrono::Utc::now().timestamp())
                    .map_err(|e| e.to_string())?;
                // INVARIANT 4: dropped, the claim gives it back.
                if !given.purposes.contains(&got.grant.purpose.as_str()) {
                    return Err(format!(
                        "that upload was begun for {}: this tool takes one begun for {}",
                        got.grant.purpose,
                        given.purposes.join(" or ")
                    ));
                }
                Ok(TakenFile {
                    bytes: got.bytes,
                    file_name: Some(got.grant.file_name),
                    mime: Some(got.grant.mime),
                    claim: Some(got.claim),
                })
            }
            (None, Some(b64)) => {
                let Some(sha) = given.sha256 else {
                    return Err("content_base64 comes with its sha256 (the file's SHA-256, hex), so a byte lost on the way is caught".into());
                };
                // INVARIANT 5.
                if b64.len() > INLINE_MAX.div_ceil(3) * 4 {
                    return Err(format!(
                        "inline is at most {INLINE_MAX} bytes: upload it with books.begin_upload"
                    ));
                }
                let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64)
                    .map_err(|_| "content_base64 is not valid base64 (guard base64_corrupt): send the file with books.begin_upload".to_string())?;
                if bytes.len() > INLINE_MAX {
                    return Err(format!(
                        "the file is {} bytes; inline is at most {INLINE_MAX}: upload it with books.begin_upload",
                        bytes.len()
                    ));
                }
                if ledger_store::journal::sha256_hex(&bytes) != sha.trim().to_ascii_lowercase() {
                    return Err("content_base64 doesn't have the sha256 given: some of it was lost on the way (guard base64_corrupt). Send the file with books.begin_upload instead".into());
                }
                Ok(TakenFile {
                    bytes,
                    file_name: None,
                    mime: None,
                    claim: None,
                })
            }
        }
    }
}

#[tool_router(router = files_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.begin_upload",
        title = "Begin an upload",
        description = "Send a file of up to 25 MiB without its bytes passing through you: give its sha256 (sha256sum), size, type, name and purpose; run the command returned with the file's path; then name the upload_id in books.attach_receipt, books.file_document, books.set_w9, books.import_bank_csv or books.import_chart_csv. Purpose: receipt or document (those two tools), import (the CSV imports), w9. If a receipt's or document's bytes are stored already, existing_attachment_id names them: link that instead. An upload lasts 15 minutes and is used once.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn begin_upload(
        &self,
        params: Parameters<BeginUploadParams>,
    ) -> Result<Json<BeginUploadOut>, String> {
        let p = params.0;
        let max = ledger_store::uploads::MAX_UPLOAD;
        let sha = p.sha256.trim().to_ascii_lowercase();
        if !ledger_store::uploads::PURPOSES.contains(&p.purpose.as_str()) {
            return Err("purpose is receipt, document, import (a bank or chart CSV) or w9".into());
        }
        // INVARIANT 2: an attachment that can be linked, for a file that
        // would be one (an import or a W-9 needs its bytes).
        let store = self.lock()?;
        let existing = if FILED.contains(&p.purpose.as_str()) {
            store
                .list_attachments(None)
                .map_err(|e| e.to_string())?
                .into_iter()
                .find(|a| a.blob_sha == sha && a.withdrawn_at.is_none())
                .map(|a| a.id)
        } else {
            None
        };
        let book = store
            .data_dir()
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        drop(store);
        if let Some(id) = existing {
            return Ok(Json(BeginUploadOut {
                upload_id: None,
                command: None,
                expires_at: None,
                max_bytes: max,
                existing_attachment_id: Some(id),
            }));
        }
        let token = self.caller_token().ok_or(NO_CALLER)?;
        let now = chrono::Utc::now();
        let grant = self
            .uploads
            .grant(
                &token,
                &sha,
                p.size,
                &p.mime_type,
                &p.file_name,
                &p.purpose,
                now.timestamp(),
            )
            .map_err(|e| e.to_string())?;
        // Always a real time: a grant lapses UPLOAD_TTL from now.
        let expires = chrono::DateTime::from_timestamp(grant.expires, 0)
            .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
            .ok_or("the upload's expiry is past the calendar")?;
        Ok(Json(BeginUploadOut {
            command: Some(format!(
                "wanderbooks upload --book {book}{} {} -- <the file's path>",
                token_flag(),
                grant.id
            )),
            upload_id: Some(grant.id),
            expires_at: Some(expires),
            max_bytes: max,
            existing_attachment_id: None,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::{CALLER, plain_file_name, token_flag};

    /// A name kept before names were checked is handed over cleaned, never
    /// refused (a W-9 sealed earlier, say).
    #[test]
    fn an_old_name_is_made_plain() {
        assert_eq!(
            plain_file_name("W9 \u{2714}\u{FE0F}.pdf"),
            "W9 \u{2714}.pdf"
        );
        assert_eq!(plain_file_name("W-9\u{200E}.pdf"), "W-9.pdf");
        assert_eq!(plain_file_name("a/b\\c\n.pdf"), "a_b_c_.pdf");
        assert_eq!(plain_file_name("\u{200B}"), "file");
        assert_eq!(plain_file_name(".."), "file");
        let long = "x".repeat(400);
        let cut = plain_file_name(&long);
        assert!(cut.len() <= ledger_store::uploads::MAX_NAME_BYTES);
        // Cut to fit, not dropped: as much of the name as the checks take,
        // and a name that fits already kept whole.
        assert_eq!(cut, "x".repeat(255));
        assert_eq!(plain_file_name(&"y".repeat(255)), "y".repeat(255));
        for n in [
            "W9 \u{2714}\u{FE0F}.pdf",
            "a/b\\c\n.pdf",
            &long,
            "\u{202E}fdp.exe",
        ] {
            assert_eq!(
                ledger_store::uploads::file_name_problem(&plain_file_name(n)),
                None,
                "{n:?}"
            );
        }
    }

    /// A command given to the agent names its token only when it isn't
    /// the one the commands use by default ("agent"): the command runs as
    /// the caller, never as another token.
    #[test]
    fn a_command_names_the_callers_token_unless_it_is_the_default() {
        let flag_for = |caller: Option<(&str, &str)>| {
            CALLER.with(|c| {
                *c.borrow_mut() = caller.map(|(id, name)| (id.to_string(), name.to_string()))
            });
            token_flag()
        };
        assert_eq!(flag_for(None), "");
        assert_eq!(flag_for(Some(("tok_1", "agent"))), "");
        assert_eq!(flag_for(Some(("tok_2", "reader"))), " --token reader");
        assert_eq!(flag_for(None), "");
    }
}
