// Fuzz target: the books server's HTTP door takes arbitrary requests.
// Method, path, headers and body are all untrusted; the door is the very
// router the server mounts (ledger-mcp local.rs `router`), in-process over a
// real book, with no socket (`fuzz_door`). Whatever arrives: the door
// answers (no panic, no hang), and a request that carries no token live
// for the book is refused 4xx and never run (INVARIANT 8, local.rs); one
// that carries it never makes the door fail (no 5xx on the MCP route).
#![no_main]

use arbitrary::Arbitrary;
use ledger_mcp::local::fuzz_door::FuzzDoor;
use libfuzzer_sys::fuzz_target;
use std::cell::RefCell;

#[derive(Arbitrary, Debug)]
enum Auth {
    None,
    Valid,
    Other,
    /// `Authorization` with whatever text.
    Raw(Vec<u8>),
}

#[derive(Arbitrary, Debug)]
enum Body {
    Raw(Vec<u8>),
    /// A JSON-RPC message the server could understand.
    Rpc {
        method: u8,
        id: u8,
        params: String,
    },
    /// `tools/call` of a name picked from a few real ones, arguments as text.
    Call {
        tool: u8,
        arguments: String,
    },
}

#[derive(Arbitrary, Debug)]
struct Request {
    method: Vec<u8>,
    verb: u8,
    route: u8,
    book: u8,
    suffix: String,
    auth: Auth,
    session: Option<String>,
    content_type: u8,
    accept: bool,
    extra: Vec<(Vec<u8>, Vec<u8>)>,
    body: Body,
}

const METHODS: [&str; 6] = ["POST", "GET", "DELETE", "PUT", "OPTIONS", "PATCH"];
const RPC: [&str; 8] = [
    "initialize",
    "notifications/initialized",
    "tools/list",
    "ping",
    "resources/list",
    "prompts/list",
    "tools/call",
    "logging/setLevel",
];
const TOOLS: [&str; 6] = [
    "books.start",
    "books.trial_balance",
    "books.draft_entry",
    "books.begin_upload",
    "books.settings",
    "books.handbook",
];

thread_local! {
    /// The door, and how many requests it has had: a fresh book every so
    /// often, because sessions and the access log only grow.
    static DOOR: RefCell<Option<(FuzzDoor, u32)>> = const { RefCell::new(None) };
}

fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || "-_.".contains(*c))
        .take(40)
        .collect()
}

fuzz_target!(|r: Request| {
    DOOR.with(|cell| {
        let mut cell = cell.borrow_mut();
        if cell.as_ref().is_none_or(|(d, n)| *n > 3000 || d.sessions() > 500) {
            *cell = Some((FuzzDoor::new(), 0));
        }
        let (door, n) = cell.as_mut().unwrap();
        *n += 1;

        let method: Vec<u8> = if r.verb % 4 == 0 {
            r.method.clone()
        } else {
            METHODS[r.verb as usize % METHODS.len()].as_bytes().to_vec()
        };
        let book = match r.book % 4 {
            0 | 1 => "acme".to_string(),
            2 => "nobody".to_string(),
            _ => clean(&r.suffix),
        };
        let path = match r.route % 6 {
            0..=2 => format!("/b/{book}/mcp"),
            3 => format!("/b/{book}/u/{}", clean(&r.suffix)),
            4 => format!("/b/{book}/a/{}", clean(&r.suffix)),
            _ => format!("/b/{book}/x/{}", clean(&r.suffix)),
        };
        let mut headers: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        match &r.auth {
            Auth::None => {}
            Auth::Valid => {
                headers.push((b"authorization".to_vec(), format!("Bearer {}", door.bearer()).into_bytes()));
            }
            Auth::Other => {
                headers.push((b"authorization".to_vec(), format!("Bearer {}x", door.other_bearer()).into_bytes()));
            }
            Auth::Raw(v) => {
                headers.push((b"authorization".to_vec(), v.clone()));
            }
        }
        if let Some(s) = &r.session {
            headers.push((b"mcp-session-id".to_vec(), clean(s).into_bytes()));
        }
        headers.push((
            b"content-type".to_vec(),
            ["application/json", "text/plain", "application/octet-stream", ""][r.content_type as usize % 4]
                .as_bytes()
                .to_vec(),
        ));
        if r.accept {
            headers.push((b"accept".to_vec(), b"application/json, text/event-stream".to_vec()));
        }
        headers.extend(r.extra.iter().take(4).cloned());
        // The door's own rule, asked directly: any header (the authorization
        // one, or a smuggled extra) can carry a token live for the book.
        let live = door.carries_a_token(&headers);
        let body = match &r.body {
            Body::Raw(b) => b.clone(),
            Body::Rpc { method, id, params } => {
                let params = serde_json::from_str::<serde_json::Value>(params).unwrap_or_default();
                serde_json::json!({"jsonrpc": "2.0", "id": id, "method": RPC[*method as usize % RPC.len()], "params": params})
                    .to_string()
                    .into_bytes()
            }
            Body::Call { tool, arguments } => {
                let args = serde_json::from_str::<serde_json::Value>(arguments).unwrap_or_default();
                serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                    "params": {"name": TOOLS[*tool as usize % TOOLS.len()], "arguments": args}})
                .to_string()
                .into_bytes()
            }
        };

        let Some((status, _)) = door.send(&method, &path, &headers, body) else {
            return; // not an HTTP request at all
        };
        if !live && book == "acme" && path.ends_with("/mcp") {
            assert!((400..500).contains(&status), "no token, yet {status} for {path}");
        }
        if live && path.ends_with("/mcp") && book == "acme" {
            assert!(status < 500, "a valid token made the door fail: {status}");
        }
    });
});
