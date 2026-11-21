//! Web assistants, from the app's side: the switch per company, the one
//! address, and the line to the relay, joined to the books server's remote
//! door (plan P15 unit B3; WB-link-6).
//!
//! ```text
//! GOAL: a person turns "Allow web assistants" on for a company and the
//!       app offers that company at one durable public address, through
//!       the relay, with the sign-in the books server's remote door gives;
//!       turned off, the company is no longer offered, and when no company
//!       is offered the line and the door are closed; and what was on when
//!       the app closed is on again when it opens, once the books are.
//! SOURCE: plan/p15-app-and-relay.md 3.3 (a switch per company, off by
//!         default; the address with Copy; "Turning it off closes the line
//!         and the door; the name is kept"), 5 (one name per installation,
//!         each company a path under it), D-80, D-84, WB-link-6.
//! INVARIANTS:
//!   1. Off by default, and per company: nothing is dialled, listened on
//!      or offered until a person turns a company on, and only that
//!      company is offered.
//!   2. The order is the door first, then the line when turning on (the
//!      door listens on 127.0.0.1 only and wants the secret), and the line
//!      first, then the door, when turning off: nothing arrives for a door
//!      that is gone, and no door is open to a line that is gone.
//!   3. One address for every company: the address's path names the
//!      company (`/b/<company>/mcp`), and a new address replaces the
//!      secret label, restarts the door at the new issuer (so every token
//!      and sign-in made at the old address is refused) and keeps the name.
//!   4. The switches are kept in a private file in the link folder, so
//!      they are still on at the next start; a company that is not open
//!      yet waits for its tab.
//!   5. The state is read from memory and never waits on the network or
//!      the server: the window asks it every frame.
//!   6. A failure at any step leaves nothing half-on: the company is not
//!      offered, and if it was the only one the line and the door are
//!      closed again; the error is in words.
//!   7. Off is only off when the books server said so: a refusal or any
//!      reply but its success leaves the company shown as on, with the
//!      error in words; a server that cannot be reached at all closes the
//!      whole line and the door (every company), and says so. A company is
//!      never shown off while it can still be reached.
//!   8. The door is started as the address the relay has accepted: the
//!      name is claimed and settled first (a name taken picks another), so
//!      the address given to the door is the one the line and the
//!      certificate carry.
//! SCENARIOS:
//!   1. See `crates/ledger-cli/tests/web_link.rs`: on, a web assistant
//!      signs in through the relay and reads the books, off, on again at
//!      the same address, a new address, a restart; and the tests below
//!      (a books server that refuses or cannot be reached when a company
//!      is turned off).
//! ```

use crate::app_server::AppHost;
use ledger_link::{CertTrust, DoorTarget, Link, LinkConfig, LinkStatus, Step, Trouble};
use ledger_mcp::admin::{Request, Response};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// The relay's domain and its own host name.
pub const DOMAIN: &str = "wanderbooks.link";
/// The relay's host, which its certificate is for.
pub const RELAY_HOST: &str = "relay.wanderbooks.link";

const SWITCHES: &str = "web.json";
const ADMIN_WAIT: Duration = Duration::from_secs(30);
/// How long to wait for the relay to accept the name before the door is
/// started as that address (INVARIANT 8).
const CLAIM_WAIT: Duration = Duration::from_secs(30);

/// What the Connect page shows of the web assistants of one company.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebState {
    /// The switch for this company.
    pub on: bool,
    /// Where it has got to.
    pub step: Step,
    /// The address, while on.
    pub address: Option<String>,
    /// The address an assistant is given for this company.
    pub connector: Option<String>,
    /// A line of progress in words.
    pub words: String,
    /// What is wrong, in words.
    pub problem: Option<String>,
    /// Whether the problem needs the person (and is not just waiting).
    pub needs_person: bool,
    /// An address is kept for when it is turned on again.
    pub kept: bool,
}

impl WebState {
    fn off(kept: bool) -> Self {
        WebState {
            on: false,
            step: Step::Off,
            address: None,
            connector: None,
            words: "Web assistants are off.".into(),
            problem: None,
            needs_person: false,
            kept,
        }
    }
}

/// How the app reaches the books server's admin requests.
pub trait Admin: Send + Sync {
    /// One admin request.
    fn admin(&self, request: Request) -> Result<Response, String>;
}

impl Admin for AppHost {
    fn admin(&self, request: Request) -> Result<Response, String> {
        self.running()?.call(request, Some(ADMIN_WAIT))
    }
}

/// How the link is made (tests give it a relay of their own).
pub type Maker = Box<dyn Fn(PathBuf) -> Result<LinkConfig, String> + Send + Sync>;

/// Web assistants for this window.
pub struct WebAssistants {
    root: Option<PathBuf>,
    make: Maker,
    link: OnceLock<Result<Arc<Link>, String>>,
    on: Mutex<BTreeSet<String>>,
    /// An error from putting the switches back at the start.
    restore_error: Mutex<Option<String>>,
    claim_wait: Duration,
    // One change at a time.
    busy: Mutex<()>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The program's own relay.
pub fn production(dir: PathBuf) -> Result<LinkConfig, String> {
    Ok(LinkConfig {
        issuer: Arc::new(ledger_link::AcmeIssuer::new(&dir)),
        trust: CertTrust::public()?,
        dir,
        domain: DOMAIN.into(),
        relay_host: RELAY_HOST.into(),
        dial: ledger_link::Dial::Host(RELAY_HOST.into(), 443),
        relay_tls: ledger_link::public_relay_tls().map_err(|e| e.to_string())?,
        timing: ledger_link::Timing::default(),
    })
}

impl WebAssistants {
    /// Web assistants for the data root `root` (else this account's).
    pub fn new(root: Option<PathBuf>) -> Self {
        Self::with_maker(root, Box::new(production))
    }

    /// With a link made another way (tests).
    pub fn with_maker(root: Option<PathBuf>, make: Maker) -> Self {
        WebAssistants {
            root,
            make,
            link: OnceLock::new(),
            on: Mutex::new(BTreeSet::new()),
            restore_error: Mutex::new(None),
            claim_wait: CLAIM_WAIT,
            busy: Mutex::new(()),
        }
    }

    /// Wait this long for the relay to accept the name (tests).
    pub fn with_claim_wait(mut self, wait: Duration) -> Self {
        self.claim_wait = wait;
        self
    }

    fn dir(&self) -> Result<PathBuf, String> {
        let root =
            ledger_store::root::Root::find(self.root.as_deref()).map_err(|e| e.to_string())?;
        Ok(root.path().join("link"))
    }

    fn link(&self) -> Result<&Arc<Link>, String> {
        self.link
            .get_or_init(|| {
                let cfg = (self.make)(self.dir()?)?;
                Link::start(cfg).map(Arc::new)
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    fn save(&self, set: &BTreeSet<String>) -> Result<(), String> {
        let dir = self.dir()?;
        let text = serde_json::to_vec(&serde_json::json!({ "books": set }))
            .map_err(|_| "can't keep the switches".to_string())?;
        ledger_link::private::make_dir(&dir)
            .map_err(|e| format!("can't keep the switches ({})", e.kind()))?;
        ledger_link::private::write(&dir.join(SWITCHES), &text)
            .map_err(|e| format!("can't keep the switches ({})", e.kind()))
    }

    fn load(&self) -> BTreeSet<String> {
        let Ok(dir) = self.dir() else {
            return BTreeSet::new();
        };
        std::fs::read(dir.join(SWITCHES))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| {
                v["books"].as_array().map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(str::to_owned))
                        .collect()
                })
            })
            .unwrap_or_default()
    }

    /// The state of one company's web assistants (INVARIANT 5).
    pub fn state(&self, book: &str) -> WebState {
        let on = lock(&self.on).contains(book);
        let Some(Ok(link)) = self.link.get() else {
            // The link was never made: nothing is on, and whether an address
            // is kept is read lazily from the folder.
            let kept = self
                .dir()
                .map(|d| d.join("identity.json").exists())
                .unwrap_or(false);
            let mut s = WebState::off(kept);
            if on {
                s.on = true;
                s.step = Step::GettingAddress;
                s.words = "Getting an address…".into();
            }
            return s;
        };
        let st = link.status();
        let kept = link.kept_address().is_some();
        if !on {
            return WebState::off(kept);
        }
        Self::describe(book, &st, kept, lock(&self.restore_error).clone())
    }

    fn describe(book: &str, st: &LinkStatus, kept: bool, restore: Option<String>) -> WebState {
        let address = st.address.clone();
        let needs_person = st
            .trouble
            .as_ref()
            .is_some_and(|t: &Trouble| !t.mends_itself());
        WebState {
            on: true,
            step: st.step(),
            connector: address.as_ref().map(|a| format!("{a}/b/{book}/mcp")),
            address,
            words: st.words(),
            problem: st.problem().or(restore),
            needs_person,
            kept,
        }
    }

    /// Turn web assistants on for `book` (INVARIANTS 1, 2, 6): its words
    /// say what happens next.
    pub fn turn_on(&self, admin: &dyn Admin, book: &str) -> Result<String, String> {
        let _one = lock(&self.busy);
        let link = self.link()?.clone();
        let was = lock(&self.on).clone();
        let door = self.start_door(admin, &link)?;
        let offered = admin.admin(Request::RemoteAllow {
            book: book.into(),
            allowed: true,
        });
        match offered {
            Ok(Response::Remote(_)) => {}
            Ok(Response::Refused(why)) | Err(why) => {
                self.unwind(admin, &link, &was);
                return Err(why);
            }
            Ok(other) => {
                self.unwind(admin, &link, &was);
                return Err(format!("the books server answered oddly: {other:?}"));
            }
        }
        let mut now = was.clone();
        now.insert(book.to_owned());
        if let Err(why) = self.save(&now) {
            let _ = admin.admin(Request::RemoteAllow {
                book: book.into(),
                allowed: false,
            });
            self.unwind(admin, &link, &was);
            return Err(why);
        }
        *lock(&self.on) = now;
        *lock(&self.restore_error) = None;
        link.set_door(Some(door));
        Ok("Web assistants are on. Getting an address and a certificate takes a moment.".into())
    }

    /// The door first (it listens on this computer only), then the line:
    /// the line is turned on, so the address is known, and the door is
    /// started as that address.
    fn start_door(&self, admin: &dyn Admin, link: &Arc<Link>) -> Result<DoorTarget, String> {
        link.turn_on().map_err(|e| e.to_string())?;
        // The name is settled before the door is started as it (INVARIANT
        // 8): the first claim may find it taken and pick another.
        if let Err(why) = link.wait_settled(self.claim_wait) {
            if lock(&self.on).is_empty() {
                link.turn_off();
            }
            return Err(why);
        }
        let address = link
            .status()
            .address
            .ok_or("Wanderbooks has no address for this yet")?;
        let info = match admin.admin(Request::RemoteStart {
            public_url: address,
            port: None,
        }) {
            Ok(Response::Remote(info)) => info,
            Ok(Response::Refused(why)) | Err(why) => {
                if lock(&self.on).is_empty() {
                    link.turn_off();
                }
                return Err(why);
            }
            Ok(other) => {
                if lock(&self.on).is_empty() {
                    link.turn_off();
                }
                return Err(format!("the books server answered oddly: {other:?}"));
            }
        };
        let listening = info
            .listening
            .ok_or("the books server's door isn't listening")?
            .parse()
            .map_err(|_| "the books server's door has an odd address".to_string())?;
        let secret = info
            .proxy_secret
            .ok_or("the books server gave no secret for its door")?;
        DoorTarget::new(listening, &secret.0)
    }

    /// Put things back as they were `before` a failed change (INVARIANT 6).
    fn unwind(&self, admin: &dyn Admin, link: &Arc<Link>, before: &BTreeSet<String>) {
        if before.is_empty() {
            link.turn_off();
            link.set_door(None);
            let _ = admin.admin(Request::RemoteStop);
        }
    }

    /// Turn web assistants off for `book` (INVARIANTS 2 and 7): with no
    /// company left offered, the line closes first, then the door.
    pub fn turn_off(&self, admin: &dyn Admin, book: &str) -> Result<String, String> {
        let _one = lock(&self.busy);
        let mut now = lock(&self.on).clone();
        now.remove(book);
        // Withdraw the company first: nothing new reaches its books.
        let withdrawn = admin.admin(Request::RemoteAllow {
            book: book.into(),
            allowed: false,
        });
        let mut closed_all = false;
        match withdrawn {
            Ok(Response::Remote(_)) => {}
            // The server answered, and did not withdraw the company: it can
            // still be reached, so it stays shown as on, with the reason.
            Ok(Response::Refused(why)) => {
                return Err(format!(
                    "Web assistants are still on for this company: the books server would not turn them off ({why})."
                ));
            }
            Ok(other) => {
                return Err(format!(
                    "Web assistants are still on for this company: the books server answered in a way Wanderbooks did not expect ({other:?})."
                ));
            }
            // The server cannot be reached at all, so nothing can be told
            // about what it still offers: the safe side is to close the
            // line and the door for every company.
            Err(_) => closed_all = true,
        }
        if (now.is_empty() || closed_all)
            && let Some(Ok(link)) = self.link.get()
        {
            link.turn_off();
            link.set_door(None);
            let _ = admin.admin(Request::RemoteStop);
        }
        // What the person asked to be on stays on disk, so it is on again
        // at the next start; what is open now is what is still offered.
        self.save(&now)?;
        *lock(&self.on) = if closed_all { BTreeSet::new() } else { now };
        Ok(if closed_all {
            "Wanderbooks could not reach its books server to turn this off one company at a time, so it closed the connection to the internet for every company. Turn web assistants on again for the others if you want them.".into()
        } else {
            "Web assistants are off for this company. Its address is kept for when you turn them on again.".into()
        })
    }

    /// A new address (INVARIANT 3): the one the web assistants use for
    /// every company changes; the old one stops working.
    pub fn new_address(&self, admin: &dyn Admin) -> Result<String, String> {
        let _one = lock(&self.busy);
        let link = self.link()?.clone();
        let on = !lock(&self.on).is_empty();
        if on {
            // The old door goes first: nothing signs in at the old address
            // while the new one is made.
            let _ = admin.admin(Request::RemoteStop);
            link.set_door(None);
        }
        let new = link.new_address().map_err(|e| e.to_string())?;
        if on {
            let door = self.start_door(admin, &link)?;
            let allowed: Vec<String> = lock(&self.on).iter().cloned().collect();
            for book in allowed {
                let _ = admin.admin(Request::RemoteAllow {
                    book,
                    allowed: true,
                });
            }
            link.set_door(Some(door));
        }
        Ok(format!(
            "Your new address is ready ({new}). The old one has stopped working: connect your assistants again with the new one."
        ))
    }

    /// Put the switches back as they were when the app closed, for the
    /// companies that are open (INVARIANT 4).
    pub fn restore(&self, admin: &dyn Admin, open: &[String]) {
        let kept = self.load();
        for book in open {
            if kept.contains(book)
                && !lock(&self.on).contains(book)
                && let Err(why) = self.turn_on(admin, book)
            {
                *lock(&self.restore_error) = Some(format!(
                    "Web assistants could not be turned back on for {book}: {why}"
                ));
            }
        }
    }

    /// The window is closing: nothing is dialled or listened on after
    /// this. The switches stay as they are on disk.
    pub fn shutdown(&self) {
        if let Some(Ok(link)) = self.link.get() {
            link.turn_off();
            link.set_door(None);
        }
    }

    /// The network may have changed (a laptop woke): dial again now.
    pub fn nudge(&self) {
        if let Some(Ok(link)) = self.link.get() {
            link.nudge();
        }
    }

    /// The link's own status (tests).
    pub fn link_status(&self) -> Option<LinkStatus> {
        self.link
            .get()
            .and_then(|l| l.as_ref().ok())
            .map(|l| l.status())
    }
}

impl Drop for WebAssistants {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledger_mcp::admin::{RemoteInfo, TokenText};
    use std::sync::atomic::{AtomicU8, Ordering};

    const SECRET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// What the books server does when a company is withdrawn.
    const OK: u8 = 0;
    const REFUSES: u8 = 1;
    const UNREACHABLE: u8 = 2;

    /// A stand-in for the books server's admin socket: it keeps the set of
    /// companies it offers and says what it was asked.
    struct Server {
        withdraw: AtomicU8,
        offered: Mutex<BTreeSet<String>>,
        asked: Mutex<Vec<String>>,
        door: Mutex<bool>,
    }

    impl Server {
        fn new() -> Server {
            Server {
                withdraw: AtomicU8::new(OK),
                offered: Mutex::new(BTreeSet::new()),
                asked: Mutex::new(vec![]),
                door: Mutex::new(false),
            }
        }

        fn info(&self) -> RemoteInfo {
            RemoteInfo {
                listening: Some("127.0.0.1:9".into()),
                public_url: None,
                allowed: lock(&self.offered).iter().cloned().collect(),
                proxy_secret: Some(TokenText(SECRET.into())),
            }
        }

        fn said(&self, what: &str) -> usize {
            lock(&self.asked).iter().filter(|a| *a == what).count()
        }
    }

    impl Admin for Server {
        fn admin(&self, request: Request) -> Result<Response, String> {
            match request {
                Request::RemoteStart { .. } => {
                    *lock(&self.door) = true;
                    Ok(Response::Remote(self.info()))
                }
                Request::RemoteStop => {
                    lock(&self.asked).push("stop".into());
                    *lock(&self.door) = false;
                    lock(&self.offered).clear();
                    Ok(Response::Remote(self.info()))
                }
                Request::RemoteAllow {
                    book,
                    allowed: true,
                } => {
                    lock(&self.offered).insert(book);
                    Ok(Response::Remote(self.info()))
                }
                Request::RemoteAllow {
                    book,
                    allowed: false,
                } => {
                    lock(&self.asked).push(format!("withdraw {book}"));
                    match self.withdraw.load(Ordering::SeqCst) {
                        REFUSES => Ok(Response::Refused("not today".into())),
                        UNREACHABLE => Err("the books server is not there".into()),
                        _ => {
                            lock(&self.offered).remove(&book);
                            Ok(Response::Remote(self.info()))
                        }
                    }
                }
                other => Err(format!("not asked in these tests: {other:?}")),
            }
        }
    }

    /// Web assistants over a data root of their own, with a name the relay
    /// already accepted (so the address is settled) and a relay that is
    /// not there: the line cannot come up, which these tests do not need.
    fn web() -> (tempfile::TempDir, WebAssistants) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let root = dir.path().join("root");
        ledger_store::root::Root::setup(&home, Some(&root), None).unwrap();
        let found = ledger_store::root::Root::find(Some(&root)).unwrap();
        let link = found.path().join("link");
        let mut id = ledger_link::identity::Identity::load_or_create(&link).unwrap();
        id.set_claimed().unwrap();
        let web = WebAssistants::with_maker(
            Some(root),
            Box::new(|dir| {
                Ok(LinkConfig {
                    domain: DOMAIN.into(),
                    relay_host: RELAY_HOST.into(),
                    dial: ledger_link::Dial::Addr("127.0.0.1:1".parse().unwrap()),
                    relay_tls: ledger_link::public_relay_tls().map_err(|e| e.to_string())?,
                    // Never reached: the line cannot come up, so nothing is
                    // ordered.
                    issuer: Arc::new(ledger_link::AcmeIssuer::new(&dir)),
                    trust: CertTrust::public()?,
                    timing: ledger_link::Timing::default(),
                    dir,
                })
            }),
        );
        (dir, web)
    }

    fn link_on(web: &WebAssistants) -> bool {
        web.link_status().is_some_and(|s| s.on)
    }

    /// The plain case: with another company still on, turning one off makes
    /// it unreachable at once and the other keeps working.
    #[test]
    fn one_company_off_is_unreachable_at_once_while_another_still_works() {
        let (_dir, web) = web();
        let server = Server::new();
        web.turn_on(&server, "alpha").unwrap();
        web.turn_on(&server, "beta").unwrap();
        assert_eq!(lock(&server.offered).len(), 2);

        let said = web.turn_off(&server, "alpha").unwrap();
        assert!(said.contains("off for this company"), "{said}");
        assert!(!web.state("alpha").on);
        assert!(web.state("beta").on);
        let offered = lock(&server.offered).clone();
        assert!(
            !offered.contains("alpha"),
            "alpha is not offered by the server"
        );
        assert!(offered.contains("beta"), "beta still is");
        assert!(link_on(&web), "the line stays up for beta");
        assert!(*lock(&server.door), "and so does the door");
        assert_eq!(server.said("stop"), 0);
        // The company left off stays off at the next start.
        assert_eq!(web.load(), BTreeSet::from(["beta".to_string()]));
    }

    /// The server refuses to withdraw the company: it is still reachable,
    /// so it is still shown as on, with the reason in words; nothing else
    /// changes.
    #[test]
    fn a_refusal_leaves_the_company_shown_as_on_with_the_reason() {
        let (_dir, web) = web();
        let server = Server::new();
        web.turn_on(&server, "alpha").unwrap();
        web.turn_on(&server, "beta").unwrap();

        server.withdraw.store(REFUSES, Ordering::SeqCst);
        let err = web.turn_off(&server, "alpha").unwrap_err();
        assert!(err.contains("still on for this company"), "{err}");
        assert!(err.contains("not today"), "{err}");
        assert!(web.state("alpha").on, "alpha is reachable, so shown on");
        assert!(web.state("beta").on);
        assert!(lock(&server.offered).contains("alpha"));
        assert!(link_on(&web));
        assert_eq!(server.said("stop"), 0, "the door was not closed");
        // Nothing was saved off either.
        assert_eq!(
            web.load(),
            BTreeSet::from(["alpha".to_string(), "beta".to_string()])
        );

        // And once the server is willing, it works.
        server.withdraw.store(OK, Ordering::SeqCst);
        web.turn_off(&server, "alpha").unwrap();
        assert!(!web.state("alpha").on);
    }

    /// The server cannot be reached at all: the safe side is the whole line
    /// and door closed, and the person is told.
    #[test]
    fn an_unreachable_server_closes_the_whole_line_and_door_and_says_so() {
        let (_dir, web) = web();
        let server = Server::new();
        web.turn_on(&server, "alpha").unwrap();
        web.turn_on(&server, "beta").unwrap();

        server.withdraw.store(UNREACHABLE, Ordering::SeqCst);
        let said = web.turn_off(&server, "alpha").unwrap();
        assert!(
            said.contains("closed the connection to the internet for every company"),
            "{said}"
        );
        assert!(!link_on(&web), "the line is closed");
        assert!(!web.state("alpha").on);
        assert!(
            !web.state("beta").on,
            "beta is not reachable either, so not shown on"
        );
        assert_eq!(server.said("stop"), 1, "the door was told to close");
        // What the person had on is still wanted at the next start.
        assert_eq!(web.load(), BTreeSet::from(["beta".to_string()]));
    }
}
