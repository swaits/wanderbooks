// Fuzz target: a file's bytes come in only by `PUT /b/{book}/u/{upload}`
// (ledger-mcp local.rs INVARIANT 9), through the same in-process router as
// fuzz_http_mcp_request. The body, the declared size and hash, the upload
// id and the token are all arbitrary. Whatever arrives: the door answers,
// and bytes are taken only if they are exactly the declared ones, from the
// token the upload was granted to, once: a 200 for anything else is a bug.
#![no_main]

use arbitrary::Arbitrary;
use ledger_mcp::local::fuzz_door::FuzzDoor;
use libfuzzer_sys::fuzz_target;
use std::cell::RefCell;

#[derive(Arbitrary, Debug)]
enum Declared {
    Honest,
    /// The size off by this much (never 0).
    SizeOff(i8),
    /// The hash of other bytes.
    HashOf(Vec<u8>),
    /// A hash that is not 64 hex digits.
    BadHash(String),
}

#[derive(Arbitrary, Debug)]
enum Who {
    Owner,
    Other,
    Nobody,
}

#[derive(Arbitrary, Debug)]
struct Put {
    body: Vec<u8>,
    declared: Declared,
    who: Who,
    /// Use an id that was never granted (this text, cleaned).
    unknown_id: Option<String>,
    /// Send it a second time too.
    again: bool,
    /// And a body of another length the second time.
    second: Vec<u8>,
}

thread_local! {
    static DOOR: RefCell<Option<(FuzzDoor, u32)>> = const { RefCell::new(None) };
}

fuzz_target!(|p: Put| {
    DOOR.with(|cell| {
        let mut cell = cell.borrow_mut();
        if cell.as_ref().is_none_or(|(_, n)| *n > 1500) {
            *cell = Some((FuzzDoor::new(), 0));
        }
        let (door, n) = cell.as_mut().unwrap();
        *n += 1;

        let sha = ledger_store::journal::sha256_hex(&p.body);
        let (grant_sha, grant_size) = match &p.declared {
            Declared::Honest => (sha.clone(), p.body.len() as u64),
            Declared::SizeOff(d) => {
                let d = if *d == 0 { 1 } else { i64::from(*d) };
                (sha.clone(), (p.body.len() as i64 + d).max(0) as u64)
            }
            Declared::HashOf(other) => (
                ledger_store::journal::sha256_hex(other),
                p.body.len() as u64,
            ),
            Declared::BadHash(s) => (s.clone(), p.body.len() as u64),
        };
        let size_ok = grant_size == p.body.len() as u64 && grant_sha == sha;

        let granted = door.grant(&grant_sha, grant_size);
        let id = match (&p.unknown_id, &granted) {
            (Some(s), _) => {
                let s: String = s
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .take(40)
                    .collect();
                if s.is_empty() { "up_x".to_string() } else { s }
            }
            (None, Ok(id)) => id.clone(),
            (None, Err(_)) => "up_none".to_string(),
        };
        let real = p.unknown_id.is_none() && granted.is_ok();
        let bearer = match p.who {
            Who::Owner => Some(door.bearer().to_string()),
            Who::Other => Some(door.other_bearer().to_string()),
            Who::Nobody => None,
        };
        let mut headers = Vec::new();
        if let Some(b) = &bearer {
            headers.push((
                b"authorization".to_vec(),
                format!("Bearer {b}").into_bytes(),
            ));
        }
        let path = format!("/b/acme/u/{id}");
        let Some((status, text)) = door.send(b"PUT", &path, &headers, p.body.clone()) else {
            return;
        };
        let may_succeed = real && matches!(p.who, Who::Owner) && size_ok;
        if status == 200 {
            assert!(
                may_succeed,
                "bytes taken that should not be: {:?} {:?}",
                p.who, p.declared
            );
            let said = String::from_utf8_lossy(&text);
            assert!(
                said.contains(&sha),
                "the answer doesn't name the bytes' hash: {said}"
            );
        } else if may_succeed {
            // Everything lined up: the only way not to be taken is a
            // failure of the door itself.
            panic!(
                "honest upload by its owner refused: {status} {}",
                String::from_utf8_lossy(&text)
            );
        }
        if status == 200 && p.again {
            let Some((again, _)) = door.send(b"PUT", &path, &headers, p.second.clone()) else {
                return;
            };
            assert_ne!(again, 200, "an upload taken twice");
        }
    });
});
