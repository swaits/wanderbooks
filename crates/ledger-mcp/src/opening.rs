//! What the agents' bridge answers `initialize` with while the books are
//! still opening (plan 3.9, "The bridge and slow starts").
//!
//! ```text
//! GOAL: an agent whose session starts while the books are closed gets
//!       its `initialize` answered within its client's startup timeout,
//!       with capabilities that are the server's own.
//! SOURCE: plan/served.md 3.9 (the bridge answers initialize itself after
//!         5 s, "from the tool list built into the same executable and a
//!         short generic text that points at books.start").
//! INVARIANTS:
//!   1. The capabilities are the server's (one function builds both).
//!   2. The protocol version is the one asked for when this build knows
//!      it and it has an initialize handshake, else the newest that does.
//!   3. The text names no book and no folder: it points at books.start.
//! ```

use rmcp::model::{Implementation, ProtocolVersion, ServerCapabilities, ServerConfig};

/// The text an agent gets while the books open (INVARIANT 3).
pub const OPENING_TEXT: &str = "wanderbooks: double-entry books, opening now. Call books.start first: it returns these books' rules, conventions and open items, and every procedure is in books.handbook. Calls wait while the books open.";

/// The server's capabilities (INVARIANT 1).
pub(crate) fn capabilities() -> ServerCapabilities {
    ServerCapabilities::builder()
        .enable_tools()
        .enable_resources()
        .enable_prompts()
        .build()
}

/// The `initialize` result the bridge gives while the books open, for a
/// client that asked for protocol version `requested`.
pub fn opening_initialize(requested: Option<&str>) -> serde_json::Value {
    let version = requested
        .and_then(|asked| {
            ProtocolVersion::KNOWN_VERSIONS
                .iter()
                .find(|v| v.as_str() == asked && v.has_initialize())
        })
        .cloned()
        .unwrap_or(ProtocolVersion::LATEST_WITH_INITIALIZE);
    let mut info = ServerConfig::new(capabilities())
        .with_instructions(OPENING_TEXT)
        .with_server_info(Implementation::new(
            "wanderbooks",
            env!("CARGO_PKG_VERSION"),
        ));
    info.protocol_version = version;
    serde_json::to_value(info).expect("an initialize result is plain data")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_asked_for_is_kept_when_known() {
        for v in ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"] {
            assert_eq!(opening_initialize(Some(v))["protocolVersion"], v);
        }
        let newest = ProtocolVersion::LATEST_WITH_INITIALIZE.as_str().to_string();
        for v in [
            Some("1999-01-01"),
            Some("2026-07-28"),
            Some("nonsense"),
            None,
        ] {
            assert_eq!(
                opening_initialize(v)["protocolVersion"],
                newest.as_str(),
                "{v:?}"
            );
        }
    }

    #[test]
    fn it_names_the_server_and_points_at_books_start() {
        let r = opening_initialize(None);
        assert_eq!(r["serverInfo"]["name"], "wanderbooks");
        assert_eq!(r["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
        assert!(r["instructions"].as_str().unwrap().contains("books.start"));
        assert_eq!(
            r["capabilities"],
            serde_json::to_value(capabilities()).unwrap()
        );
        assert!(r["capabilities"]["tools"].is_object());
        assert!(r["capabilities"]["resources"].is_object());
        assert!(r["capabilities"]["prompts"].is_object());
    }
}
