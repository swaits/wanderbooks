//! Who is calling, what their token allows, and the record of each call
//! (plan 3.3, I-9), for books served from the data root.
//!
//! ```text
//! GOAL: a call reaches a book only with a token for that book and for
//!       what the call does, and every call that reaches it is recorded
//!       before it is answered.
//! SOURCE: plan 3.3 (scopes), I-9 ("every access through the server is
//!         logged, and logging fails closed"), section 7's record.
//! INVARIANTS:
//!   1. The HTTP door (local.rs) puts a Caller on every request it lets
//!      through; a call that reaches a book with an access log and has no
//!      Caller is refused.
//!   2. A call runs only if its Caller's token has the scope it needs
//!      (scopes.rs); otherwise it is refused, unrun.
//!   3. Every call, run or refused, gets one record, written before its
//!      answer goes (a call that ran, on its own blocking thread while it
//!      still counts as running, so a close waits for its record too);
//!      if the record can't be written, the book stops taking requests
//!      (SharedStore's stop) and the answer says so, a call refused as not
//!      run keeping its "not run" code.
//!   4. A record's head is the journal's head after the call's own last
//!      write (taken under the store's lock as it lets go), whatever the
//!      call answered; none for a call that wrote nothing.
//! ```

use ledger_store::access::{Access, AccessLog, Client, Outcome, Principal, digest};
use ledger_store::auth::{Scope, Token};
use ledger_store::chain::Head;
use rmcp::service::{RequestContext, RoleServer};
use std::sync::Arc;

/// Who made a request: the token the door matched (INVARIANT 1).
#[derive(Clone, Debug)]
pub struct Caller {
    /// The token.
    pub token: Token,
}

impl Caller {
    /// As the access log names them.
    pub fn principal(&self) -> Principal {
        Principal {
            id: self.token.id.clone(),
            kind: "agent".into(),
            label: self.token.name.clone(),
        }
    }

    /// Whether the token has `scope`.
    pub fn may(&self, scope: Scope) -> bool {
        self.token.scopes.contains(&scope)
    }
}

/// The caller of a request, as the door left it.
pub fn caller(context: &RequestContext<RoleServer>) -> Option<Caller> {
    context
        .extensions
        .get::<axum::http::request::Parts>()
        .and_then(|parts| parts.extensions.get::<Caller>())
        .cloned()
}

/// The client program, as it named itself at `initialize`.
fn client(context: &RequestContext<RoleServer>) -> Option<Client> {
    context.peer.peer_info().map(|info| Client {
        name: info.client_info.name.clone(),
        version: info.client_info.version.clone(),
    })
}

fn request_id(context: &RequestContext<RoleServer>) -> String {
    match &context.id {
        rmcp::model::NumberOrString::Number(n) => n.to_string(),
        rmcp::model::NumberOrString::String(s) => s.to_string(),
    }
}

/// One call on its way through: who, what, and the log it goes into.
#[derive(Clone)]
pub(crate) struct Call {
    log: Arc<AccessLog>,
    caller: Option<Caller>,
    client: Option<Client>,
    request_id: String,
    action: &'static str,
    subject: String,
    args_digest: Option<String>,
}

/// Why a call wasn't run, for its record and its answer.
pub(crate) const UNAUTHORIZED: &str = "unauthorized";
/// The token lacks the scope.
pub(crate) const INSUFFICIENT_SCOPE: &str = "insufficient_scope";
/// The books were closing.
pub(crate) const CLOSING: &str = "closing";
/// The server had stopped.
pub(crate) const STOPPED: &str = "stopped";

/// What a refused call is told when its record couldn't be kept, or a
/// call that ran is told instead of its answer (INVARIANT 3).
pub(crate) const NOT_RECORDED: &str = "this call's access record couldn't be written, so the books server stopped taking requests, to keep every access on record: ask the human to restart it";
/// The same, for a call that ran.
pub(crate) const RAN_NOT_RECORDED: &str = "this call ran, but its access record couldn't be written, so the books server stopped taking requests, to keep every access on record: ask the human to restart it, then check whether the call's change is in the books before trying again";

impl Call {
    /// A call to `subject` (a tool's name, a resource's address). Its
    /// arguments' digest is added later ([`Call::digested`]): hashing a
    /// large call's arguments waits until it is counted as running, so a
    /// close beginning meanwhile waits for it.
    pub(crate) fn new(
        log: &Arc<AccessLog>,
        context: &RequestContext<RoleServer>,
        action: &'static str,
        subject: &str,
    ) -> Self {
        Self {
            log: log.clone(),
            caller: caller(context),
            client: client(context),
            request_id: request_id(context),
            action,
            subject: subject.to_string(),
            args_digest: None,
        }
    }

    /// The digest of a call's arguments (blocking work: a call's
    /// arguments may be megabytes).
    pub(crate) fn digest_of(
        args: Option<&serde_json::Map<String, serde_json::Value>>,
    ) -> Option<String> {
        args.map(|a| digest(&serde_json::Value::Object(a.clone())))
    }

    /// This call, with its arguments' digest.
    pub(crate) fn digested(&mut self, args_digest: Option<String>) {
        self.args_digest = args_digest;
    }

    /// The call's caller, if the door named one.
    pub(crate) fn caller(&self) -> Option<&Caller> {
        self.caller.as_ref()
    }

    /// Record the call's outcome, on a blocking thread; Err only if it
    /// couldn't be written (INVARIANT 3).
    pub(crate) async fn record(&self, result: Outcome, head_after: Option<Head>) -> Result<(), ()> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || me.record_blocking(result, head_after))
            .await
            .unwrap_or(Err(()))
    }

    /// [`Call::record`], on this thread (one that may block).
    pub(crate) fn record_blocking(
        &self,
        result: Outcome,
        head_after: Option<Head>,
    ) -> Result<(), ()> {
        let access = Access {
            principal: self
                .caller
                .as_ref()
                .map_or_else(Principal::unknown, Caller::principal),
            client: self.client.clone(),
            request_id: Some(self.request_id.clone()),
            action: self.action.to_string(),
            subject: self.subject.clone(),
            args_digest: self.args_digest.clone(),
            result,
            head_after,
        };
        match self.log.record(&access) {
            Ok(_) => Ok(()),
            Err(e) => {
                eprintln!("wanderbooks server: an access record couldn't be written: {e}");
                Err(())
            }
        }
    }
}
