//! The link: the app's one durable address, its line to the relay, its
//! own TLS, and what it serves to the remote door.
//!
//! ```text
//! GOAL: a person turns web assistants on and the app holds a durable
//!       public address, one line to the relay, its own certificate and
//!       key, and hands each web caller, decrypted here and nowhere else,
//!       to the remote door on this computer; turned off, nothing is
//!       listening and nothing is dialled; and the line is never alive
//!       longer than this process.
//! SOURCE: WB-link-1 to WB-link-5; plan p15-app-and-relay.md 3 (the shape),
//!         5 (the relay), 6a (unit B3's duties); D-80, D-84, D-87.
//! INVARIANTS:
//!   1. The line runs in the app's own process (the window), not in the
//!      books server it started: the server is a child that closes when
//!      the window's input to it closes, and the window's death takes the
//!      line with it (a socket this process holds ends with it), so the
//!      line can never keep a door open for books that closed, and the
//!      key and certificate are in one process only.
//!   2. Everything the relay is sent is the public key, the name, the
//!      proof of the key over this TLS session and the DNS proof value;
//!      the private keys (the line key, the certificate's key, the ACME
//!      account) are never sent anywhere.
//!   3. A caller's TLS ends here: the relay's bytes are TLS, decrypted by
//!      this app with its own certificate, only for the one host name of
//!      its address. What goes to the door is plain HTTP on 127.0.0.1,
//!      after the door's per-start secret and the relay's PROXY header
//!      (the first SECRET + header bytes of the connection), which is how
//!      the door knows the caller's address and that this app sent it.
//!   4. Off means off: turning the link off ends the line and the
//!      certificate stops being served before `turn_off` returns, and the
//!      name is kept so the address is the same when it is turned on again.
//!   5. A new address replaces the secret label and the old address stops
//!      being served at once (no certificate for its host name); if the
//!      relay lost the name, a whole new identity.
//!   6. The line, the certificate and the door each fail on their own:
//!      with no internet the window and the local assistants are untouched,
//!      the state says so in words, and everything resumes by itself.
//!   7. At most `MAX_CALLERS` callers are served at once; a caller whose
//!      TLS does not finish in `handshake` is dropped.
//! SCENARIOS:
//!   1. See `tests/link.rs`: claim, prove, line up, certificate, a caller
//!      through the relay to a stand-in door, off, on again with the same
//!      address, a new address.
//! ```

use crate::certs::{
    CertStore, CertTrust, DnsProof, Issuer, OrderRequest, Resolver, StoredCert, now, server_config,
};
use crate::identity::{Identity, IdentityError};
use crate::line::{
    Backoff, Dial, Failure, MAX_STREAMS, Redial, Timing, WakeDetector, failure_of, jitter,
    trouble_of_refusal,
};
use crate::status::{LinkStatus, Trouble};
use crate::text::{clean, detail};
use futures::future::BoxFuture;
use relay::client::{self, Incoming, Line, LineHandle, LineOptions};
use relay::proto::{Refusal, Request};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::{Notify, Semaphore, watch};
use tokio::time::timeout;
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

/// The most callers served at once (INVARIANT 7).
pub const MAX_CALLERS: usize = 16;

/// How the link is set up.
#[derive(Clone)]
pub struct LinkConfig {
    /// The link folder in the data root (`<root>/link`).
    pub dir: PathBuf,
    /// The relay's domain (`wanderbooks.link`).
    pub domain: String,
    /// The relay's own host name, which its certificate is for.
    pub relay_host: String,
    /// Where to dial.
    pub dial: Dial,
    /// How to check the relay's certificate.
    pub relay_tls: Arc<rustls::ClientConfig>,
    /// The certificate authority client.
    pub issuer: Arc<dyn Issuer>,
    /// Which certificate authorities' certificates are believed.
    pub trust: CertTrust,
    /// Every wait.
    pub timing: Timing,
}

/// The remote door on this computer, as the server gave it.
#[derive(Clone)]
pub struct DoorTarget {
    /// Where it listens (127.0.0.1:port).
    pub addr: SocketAddr,
    secret: Zeroizing<String>,
}

impl DoorTarget {
    /// The door at `addr` with this start's secret (64 lowercase hex
    /// characters, as the server gives it).
    pub fn new(addr: SocketAddr, secret: &str) -> Result<Self, String> {
        let hex = secret.len() == 64
            && secret
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !hex {
            return Err(
                "the books server gave a secret that isn't the expected 64 characters".into(),
            );
        }
        if !addr.ip().is_loopback() {
            return Err("the door is not on this computer".into());
        }
        Ok(DoorTarget {
            addr,
            secret: Zeroizing::new(secret.to_owned()),
        })
    }
}

impl std::fmt::Debug for DoorTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DoorTarget")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

/// Why the link could not do what was asked.
#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    /// The identity files.
    #[error("{0}")]
    Identity(#[from] IdentityError),
    /// The link is already shutting down.
    #[error("the link is shutting down")]
    Gone,
}

struct Run {
    stop: CancellationToken,
    done: tokio::task::JoinHandle<()>,
}

pub(crate) struct Inner {
    cfg: LinkConfig,
    identity: Mutex<Option<Identity>>,
    status: watch::Sender<LinkStatus>,
    resolver: Arc<Resolver>,
    door: RwLock<Option<DoorTarget>>,
    line: watch::Sender<Option<LineHandle>>,
    run: tokio::sync::Mutex<Option<Run>>,
    nudge: Notify,
    served: AtomicU64,
    /// The address kept for this installation, read once and held (the
    /// window asks every frame): `None` until read.
    kept: Mutex<Option<Option<String>>>,
}

struct LineProof(LineHandle);

impl DnsProof for LineProof {
    fn publish<'a>(&'a self, value: &'a str) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            match self.0.publish_txt(value).await {
                Ok(r) if r.ok => Ok(()),
                // The relay's words are a stranger's: short and plain
                // before they go anywhere (text.rs).
                Ok(r) => Err(clean(
                    &r.error.unwrap_or_else(|| "refused".into()),
                    crate::text::DETAIL_CHARS,
                )),
                Err(_) => Err("the line to the relay failed".into()),
            }
        })
    }

    fn clear(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let _ = self.0.request(Request::ClearTxt).await;
        })
    }
}

fn pick_in(start: i64, end: i64) -> i64 {
    if end <= start {
        return start;
    }
    let span = u64::try_from(end - start).unwrap_or(0);
    let off = jitter(Duration::from_secs(span)).as_secs();
    start + i64::try_from(off).unwrap_or(0)
}

impl Inner {
    fn set(&self, f: impl FnOnce(&mut LinkStatus)) {
        self.status.send_modify(f);
    }

    /// The address kept, as it now stands.
    fn keep(&self, address: Option<String>) {
        *self.kept.lock().unwrap_or_else(|e| e.into_inner()) = Some(address);
    }

    fn identity_do<T>(&self, f: impl FnOnce(&mut Identity) -> T) -> Option<T> {
        self.identity
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
            .map(f)
    }

    // ---- lifecycle -------------------------------------------------------

    pub(crate) async fn turn_on(self: &Arc<Self>) -> Result<(), LinkError> {
        let mut run = self.run.lock().await;
        if run.is_some() {
            return Ok(());
        }
        self.begin(&mut run).await
    }

    async fn begin(self: &Arc<Self>, run: &mut Option<Run>) -> Result<(), LinkError> {
        let dir = self.cfg.dir.clone();
        let loaded = tokio::task::spawn_blocking(move || Identity::load_or_create(&dir))
            .await
            .map_err(|_| LinkError::Gone)?;
        let id = match loaded {
            Ok(id) => id,
            Err(e) => {
                self.set(|s| {
                    s.on = true;
                    s.trouble = Some(Trouble::Files(e.to_string()));
                });
                return Err(e.into());
            }
        };
        let host = id.host(&self.cfg.domain);
        let address = id.address(&self.cfg.domain);
        let claimed = id.claimed();
        self.keep(Some(address.clone()));
        *self.identity.lock().unwrap_or_else(|e| e.into_inner()) = Some(id);
        self.resolver.set_host(&host);
        self.set(|s| {
            *s = LinkStatus {
                on: true,
                address: Some(address),
                claimed,
                served: s.served,
                ..LinkStatus::default()
            };
        });
        let stop = CancellationToken::new();
        let me = self.clone();
        let token = stop.clone();
        let done = tokio::spawn(async move {
            tokio::join!(me.line_loop(token.clone()), me.cert_loop(token));
        });
        *run = Some(Run { stop, done });
        Ok(())
    }

    pub(crate) async fn turn_off(self: &Arc<Self>) {
        let mut run = self.run.lock().await;
        self.end(&mut run).await;
    }

    async fn end(self: &Arc<Self>, run: &mut Option<Run>) {
        if let Some(r) = run.take() {
            r.stop.cancel();
            // The tasks end at their next await; give them a moment, then
            // let go of them (the line closes with its task).
            let abort = r.done.abort_handle();
            if timeout(Duration::from_secs(5), r.done).await.is_err() {
                abort.abort();
            }
        }
        self.line.send_replace(None);
        self.resolver.set_cert(None);
        let served = self.served.load(Ordering::Relaxed);
        self.set(|s| {
            *s = LinkStatus {
                served,
                ..LinkStatus::default()
            }
        });
    }

    pub(crate) async fn new_address(self: &Arc<Self>) -> Result<String, LinkError> {
        let mut run = self.run.lock().await;
        let was_on = run.is_some();
        // The relay no longer knows the name, or turned it off: a new
        // label would not help; a new identity does.
        let lost = self
            .status
            .borrow()
            .trouble
            .as_ref()
            .is_some_and(|t| *t == Trouble::NameLost || *t == Trouble::Blocked);
        self.end(&mut run).await;
        let dir = self.cfg.dir.clone();
        let domain = self.cfg.domain.clone();
        let made = tokio::task::spawn_blocking(move || -> Result<String, IdentityError> {
            if lost {
                let id = Identity::reset(&dir)?;
                // The name is being abandoned: nothing for it is kept.
                CertStore::new(&dir).forget();
                Ok(id.address(&domain))
            } else {
                let mut id = Identity::load_or_create(&dir)?;
                id.new_label()?;
                Ok(id.address(&domain))
            }
        })
        .await
        .map_err(|_| LinkError::Gone)??;
        self.keep(Some(made.clone()));
        if was_on {
            self.begin(&mut run).await?;
        } else {
            *self.identity.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        Ok(made)
    }

    // ---- the line --------------------------------------------------------

    async fn addresses(&self) -> Result<Vec<SocketAddr>, Trouble> {
        match &self.cfg.dial {
            Dial::Addr(a) => Ok(vec![*a]),
            Dial::Host(host, port) => {
                // Looked up at every dial: a laptop on a new network finds
                // the relay again.
                match timeout(
                    self.cfg.timing.connect_timeout,
                    tokio::net::lookup_host((host.as_str(), *port)),
                )
                .await
                {
                    Ok(Ok(found)) => {
                        let found: Vec<SocketAddr> = found.collect();
                        if found.is_empty() {
                            Err(Trouble::NoInternet)
                        } else {
                            Ok(found)
                        }
                    }
                    // A name that does not resolve is, nearly always, no
                    // internet.
                    _ => Err(Trouble::NoInternet),
                }
            }
        }
    }

    /// One try at a line. `Ok(Err(refusal))` is the relay saying no.
    async fn dial(&self) -> Result<Result<Line, Refusal>, Trouble> {
        let (name, key, claim) = self
            .identity_do(|id| (id.name(), id.key_copy(), !id.claimed()))
            .ok_or_else(|| Trouble::Files("no identity".into()))?;
        let opts = LineOptions {
            max_streams: MAX_STREAMS,
            ..LineOptions::default()
        };
        let mut last = Trouble::RelayDown;
        for addr in self.addresses().await? {
            let tried = timeout(
                self.cfg.timing.connect_timeout,
                client::connect(
                    addr,
                    self.cfg.relay_tls.clone(),
                    &self.cfg.relay_host,
                    name,
                    &key,
                    claim,
                    opts,
                ),
            )
            .await;
            match tried {
                Ok(Ok(line)) => return Ok(Ok(line)),
                Ok(Err(e)) => match failure_of(&e) {
                    Failure::Refused(r) => return Ok(Err(r)),
                    Failure::Trouble(t) => last = t,
                },
                Err(_) => last = Trouble::RelayDown,
            }
        }
        Err(last)
    }

    async fn line_loop(self: &Arc<Self>, stop: CancellationToken) {
        let t = self.cfg.timing;
        let mut redial = Redial::new(&t);
        let mut name_tries = 0u32;
        loop {
            if stop.is_cancelled() {
                return;
            }
            let dialled = tokio::select! {
                () = stop.cancelled() => return,
                d = self.dial() => d,
            };
            match dialled {
                Ok(Ok(line)) => {
                    name_tries = 0;
                    self.line_is_up();
                    let up = tokio::time::Instant::now();
                    let ended = self.hold(line, &stop).await;
                    self.line_is_down();
                    if stop.is_cancelled() {
                        return;
                    }
                    // A line that ended on its own (the relay restarted, the
                    // network went) is dialled again after a wait spread
                    // over `redial_spread`, so a relay coming back is not
                    // hit by every app in the same second; one that held
                    // less than `line_stable` waits as a failed dial does,
                    // so a relay that accepts and drops is not redialled
                    // at a steady rate for ever (INVARIANT 1). A sleep or a
                    // network change dials at once: that is one machine
                    // that moved, not a crowd.
                    let wait = match ended {
                        Ended::Woke | Ended::Nudged => {
                            redial.reset();
                            Duration::ZERO
                        }
                        _ => redial.ended(up.elapsed()),
                    };
                    tokio::select! {
                        () = stop.cancelled() => return,
                        () = tokio::time::sleep(wait) => {}
                    }
                    continue;
                }
                Ok(Err(Refusal::Taken)) if !self.identity_do(|i| i.claimed()).unwrap_or(true) => {
                    // Somebody else has this name: another, at once. Nothing
                    // else has used the name yet: the certificate waits for
                    // the line, and the door is not started until the name
                    // is claimed (`Link::wait_settled`).
                    name_tries += 1;
                    if name_tries <= 8 {
                        let _ = self.identity_do(|i| i.retry_name());
                        if let Some((host, address)) = self.identity_do(|i| {
                            (i.host(&self.cfg.domain), i.address(&self.cfg.domain))
                        }) {
                            self.resolver.set_host(&host);
                            self.keep(Some(address.clone()));
                            self.set(|s| s.address = Some(address));
                        }
                        continue;
                    }
                    self.set(|s| s.trouble = Some(Trouble::RelayBusy));
                }
                Ok(Err(refusal)) => {
                    let claimed = self.identity_do(|i| i.claimed()).unwrap_or(false);
                    let trouble = trouble_of_refusal(refusal, claimed);
                    self.set(|s| s.trouble = Some(trouble));
                }
                Err(trouble) => self.set(|s| s.trouble = Some(trouble)),
            }
            let mut wait = redial.failed();
            // A relay that said "wait" is not asked again at once.
            if matches!(
                self.status.borrow().trouble,
                Some(Trouble::RateLimited | Trouble::RelayBusy)
            ) {
                wait = wait.max(t.backoff_min * 4);
            }
            tokio::select! {
                () = stop.cancelled() => return,
                () = tokio::time::sleep(wait) => {}
                () = self.nudge.notified() => redial.reset(),
            }
        }
    }

    fn line_is_up(&self) {
        let _ = self.identity_do(|i| i.set_claimed());
        self.set(|s| {
            s.claimed = true;
            s.line_up = true;
            s.trouble = None;
        });
    }

    fn line_is_down(&self) {
        self.line.send_replace(None);
        self.set(|s| s.line_up = false);
    }

    /// Hold a line: serve its callers, keep it alive, end it when told
    /// or when it is no good.
    async fn hold(self: &Arc<Self>, mut line: Line, stop: &CancellationToken) -> Ended {
        let t = self.cfg.timing;
        let handle = line.handle();
        self.line.send_replace(Some(handle.clone()));
        let acceptor = TlsAcceptor::from(server_config(self.resolver.clone()));
        let callers = Arc::new(Semaphore::new(MAX_CALLERS));
        // The pings are their own task: a slow answer must not stop
        // callers being taken.
        let (failed_tx, mut failed) = tokio::sync::oneshot::channel::<()>();
        let pinger = {
            let handle = handle.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(t.ping_every).await;
                    let answered = timeout(t.ping_timeout, handle.request(Request::Ping)).await;
                    if !matches!(answered, Ok(Ok(_))) {
                        let _ = failed_tx.send(());
                        return;
                    }
                }
            })
        };
        let mut tick = tokio::time::interval(t.tick);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut wake = WakeDetector::new(t.wake_gap);
        let ended = loop {
            tokio::select! {
                () = stop.cancelled() => break Ended::Stopped,
                () = self.nudge.notified() => break Ended::Nudged,
                _ = &mut failed => break Ended::PingFailed,
                _ = tick.tick() => {
                    self.expire_if_due();
                    if wake.woke() {
                        break Ended::Woke;
                    }
                    if handle.is_closed() {
                        break Ended::Closed;
                    }
                }
                inc = line.accept() => match inc {
                    None => break Ended::Closed,
                    Some(inc) => {
                        let Ok(permit) = callers.clone().try_acquire_owned() else {
                            continue; // too many at once: this one is dropped
                        };
                        let me = self.clone();
                        let acceptor = acceptor.clone();
                        tokio::spawn(async move {
                            me.serve_caller(inc, acceptor).await;
                            drop(permit);
                        });
                    }
                },
            }
        };
        pinger.abort();
        handle.close();
        ended
    }

    /// One caller: end its TLS here, then give the plain bytes to the
    /// door after the secret and the PROXY header (INVARIANT 3).
    async fn serve_caller(&self, mut inc: Incoming, acceptor: TlsAcceptor) {
        let Some(door) = self.door.read().unwrap_or_else(|e| e.into_inner()).clone() else {
            // No door to give the caller to: end the stream, so the caller
            // is turned away now and not left waiting.
            let _ = inc.stream.shutdown().await;
            return;
        };
        let Ok(Ok(mut tls)) = timeout(self.cfg.timing.handshake, acceptor.accept(inc.stream)).await
        else {
            return;
        };
        let Ok(Ok(mut up)) = timeout(Duration::from_secs(5), TcpStream::connect(door.addr)).await
        else {
            return;
        };
        let _ = up.set_nodelay(true);
        let mut front = Vec::with_capacity(64 + 52);
        front.extend_from_slice(door.secret.as_bytes());
        front.extend(inc.caller.encode());
        if up.write_all(&front).await.is_err() {
            return;
        }
        self.served.fetch_add(1, Ordering::Relaxed);
        let served = self.served.load(Ordering::Relaxed);
        self.set(|s| s.served = served);
        let _ = tokio::io::copy_bidirectional(&mut tls, &mut up).await;
        let _ = tls.shutdown().await;
    }

    // ---- the certificate -------------------------------------------------

    fn install(&self, c: &StoredCert) {
        self.resolver
            .set_cert(c.certified().map(|k| (k, c.not_after)));
        let end = c.not_after;
        self.set(|s| {
            s.cert_expires = Some(end);
        });
    }

    fn uninstall(&self) {
        self.resolver.set_cert(None);
        self.set(|s| s.cert_expires = None);
    }

    /// A certificate that has ended is not in place, whatever is waiting
    /// to renew it (INVARIANT 9 of certs.rs): the resolver already refuses
    /// it; this makes the state say so.
    fn expire_if_due(&self) {
        let ended = self
            .status
            .borrow()
            .cert_expires
            .is_some_and(|end| now() >= end);
        if ended {
            self.uninstall();
        }
    }

    async fn wait_for_line(&self, stop: &CancellationToken) -> Option<LineHandle> {
        let mut rx = self.line.subscribe();
        loop {
            if let Some(h) = rx.borrow_and_update().clone() {
                return Some(h);
            }
            tokio::select! {
                () = stop.cancelled() => return None,
                r = rx.changed() => if r.is_err() { return None },
            }
        }
    }

    /// `wait`, cut short at `ends` (unix seconds) if that comes first: a
    /// sleep never runs past the end of the certificate in use.
    fn until(wait: Duration, ends: Option<i64>) -> Duration {
        match ends {
            Some(end) => wait.min(Duration::from_secs(
                u64::try_from(end.saturating_sub(now())).unwrap_or(0),
            )),
            None => wait,
        }
    }

    async fn cert_loop(self: &Arc<Self>, stop: CancellationToken) {
        let t = self.cfg.timing;
        let store = CertStore::new(&self.cfg.dir);
        let mut backoff = Backoff::new(t.cert_retry_min, t.cert_retry_max);
        // Nothing about a certificate is done until the line is up: the
        // relay has then accepted the name, so the name is settled and the
        // certificate is for it, never for one the first claim gave up
        // (a "taken" answer picks another).
        let Some(_line) = self.wait_for_line(&stop).await else {
            return;
        };
        loop {
            if stop.is_cancelled() {
                return;
            }
            let Some(name) = self.identity_do(|i| i.name().to_string()) else {
                return;
            };
            let identifier = format!("*.{name}.{}", self.cfg.domain);
            let stored = store.load(&name, &identifier, &self.cfg.trust);
            let tnow = now();
            match &stored {
                Some(c) if c.valid_at(tnow) => self.install(c),
                _ => self.uninstall(),
            }
            if let Some(c) = &stored
                && c.valid_at(tnow)
                && tnow < c.renew_at
            {
                // Wait for the renewal time; now and then ask the authority
                // whether it wants the certificate renewed sooner.
                let secs = u64::try_from(c.renew_at - tnow).unwrap_or(0);
                let wait = Self::until(
                    Duration::from_secs(secs).min(t.ari_recheck),
                    Some(c.not_after),
                );
                tokio::select! {
                    () = stop.cancelled() => return,
                    () = tokio::time::sleep(wait) => {}
                }
                if now() < c.renew_at {
                    self.recheck_window(&store, c).await;
                    continue;
                }
            }
            // Time to order. A line is needed for the proof.
            let Some(_line) = self.wait_for_line(&stop).await else {
                return;
            };
            let ordered = tokio::select! {
                () = stop.cancelled() => return,
                o = self.order(&name, &identifier, stored.as_ref()) => o,
            };
            match ordered {
                Ok(new) => {
                    if store.save(&new).is_err() {
                        self.set(|s| {
                            s.cert_trouble = Some(
                                "Wanderbooks got a certificate but can't keep it on this computer."
                                    .into(),
                            );
                        });
                    } else {
                        self.set(|s| s.cert_trouble = None);
                    }
                    self.install(&new);
                    backoff.reset();
                }
                Err(why) => {
                    let ends = stored
                        .as_ref()
                        .filter(|c| c.valid_at(now()))
                        .map(|c| c.not_after);
                    let words = match ends {
                        Some(_) => format!(
                            "The certificate could not be renewed yet. It keeps trying; the one in use still works. {why}"
                        ),
                        None => format!(
                            "Wanderbooks could not get a certificate yet. It keeps trying. {why}"
                        ),
                    };
                    self.set(|s| s.cert_trouble = Some(words));
                    // Never asleep past the end of the certificate in use:
                    // at its end it stops being served and the state says
                    // so.
                    let wait = Self::until(backoff.wait(), ends);
                    tokio::select! {
                        () = stop.cancelled() => return,
                        () = tokio::time::sleep(wait) => {}
                    }
                }
            }
        }
    }

    /// Ask the authority again when it wants this certificate renewed
    /// (RFC 9773): a window that opens sooner than the time kept brings
    /// the renewal forward.
    async fn recheck_window(&self, store: &CertStore, c: &StoredCert) {
        let Some(line) = self.line.borrow().clone() else {
            return;
        };
        let Ok(list) = line.authorities().await else {
            return;
        };
        let Some(a) = list.iter().find(|a| a.id == c.authority) else {
            return;
        };
        let Some((start, end)) = self.cfg.issuer.renewal_window(a, &c.chain_pem).await else {
            return;
        };
        let at = pick_in(start, end);
        if at < c.renew_at {
            let mut sooner = c.clone();
            sooner.renew_at = at;
            let _ = store.save(&sooner);
        }
    }

    async fn order(
        &self,
        name: &str,
        identifier: &str,
        previous: Option<&StoredCert>,
    ) -> Result<StoredCert, String> {
        let line = self
            .line
            .borrow()
            .clone()
            .ok_or("The line to the relay is down.")?;
        let list = line
            .authorities()
            .await
            .map_err(|_| "The relay would not say which certificate authorities to use.")?;
        if list.is_empty() {
            return Err("The relay named no certificate authority.".into());
        }
        let usable: Vec<_> = list.iter().filter(|a| a.usable).collect();
        if usable.is_empty() {
            return Err("No certificate authority is ready for this installation yet.".into());
        }
        let proof = LineProof(line);
        let mut problems: Vec<String> = Vec::new();
        for a in usable {
            // What the relay and the authority say is a stranger's: the
            // words here are the app's, and theirs is a short quoted detail
            // (text.rs).
            let which = match detail(&a.id) {
                d if d.is_empty() => "an authority".to_string(),
                d => format!("authority {d}"),
            };
            let replaces = previous
                .filter(|p| p.authority == a.id && p.valid_at(now()))
                .and_then(StoredCert::der);
            let order = self.cfg.issuer.issue(OrderRequest {
                authority: a,
                identifier: identifier.to_owned(),
                replaces,
                dns: &proof,
            });
            let result = match timeout(self.cfg.timing.order_timeout, order).await {
                Ok(r) => r,
                Err(_) => Err("took too long".to_string()),
            };
            proof.clear().await;
            match result {
                Ok(issued) => {
                    let window = self.cfg.issuer.renewal_window(a, &issued.chain_pem).await;
                    let renew_at = window.map(|(s, e)| pick_in(s, e));
                    match StoredCert::new(
                        name,
                        &a.id,
                        &issued,
                        identifier,
                        renew_at,
                        &self.cfg.trust,
                    ) {
                        Ok(c) => return Ok(c),
                        Err(why) => problems.push(format!("{which}: {why}")),
                    }
                }
                // An issuer's own words are kept short whatever they are.
                Err(why) => problems.push(format!("{which}: {}", clean(&why, PROBLEM_CHARS))),
            }
        }
        Err(problems_in_words(&problems))
    }
}

/// The most characters of one authority's failure kept in the window's
/// words.
const PROBLEM_CHARS: usize = 300;

/// The most authorities' failures spelled out; the rest are counted.
const PROBLEMS_SHOWN: usize = 3;

/// The failures of the authorities tried, as one bounded text.
fn problems_in_words(problems: &[String]) -> String {
    let mut out: Vec<String> = problems
        .iter()
        .take(PROBLEMS_SHOWN)
        .map(|p| {
            let p = clean(p, PROBLEM_CHARS);
            if p.ends_with(['.', '?', '!']) {
                p
            } else {
                format!("{p}.")
            }
        })
        .collect();
    if problems.len() > PROBLEMS_SHOWN {
        out.push(format!("And {} more.", problems.len() - PROBLEMS_SHOWN));
    }
    out.join(" ")
}

/// Why a held line ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ended {
    Stopped,
    Nudged,
    PingFailed,
    Woke,
    Closed,
}

/// The link, for the app: a handle with plain (blocking) methods over its
/// own small runtime, so the window and the books server's glue need no
/// async of their own.
pub struct Link {
    inner: Arc<Inner>,
    handle: tokio::runtime::Handle,
    owned: Mutex<Option<tokio::runtime::Runtime>>,
}

impl Link {
    fn make(cfg: LinkConfig) -> Arc<Inner> {
        let resolver = Resolver::new();
        let (status, _) = watch::channel(LinkStatus::default());
        let (line, _) = watch::channel(None);
        Arc::new(Inner {
            cfg,
            identity: Mutex::new(None),
            status,
            resolver,
            door: RwLock::new(None),
            line,
            run: tokio::sync::Mutex::new(None),
            nudge: Notify::new(),
            served: AtomicU64::new(0),
            kept: Mutex::new(None),
        })
    }

    /// A link with a runtime of its own: two threads, ended with the link.
    pub fn start(cfg: LinkConfig) -> Result<Link, String> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("wanderbooks-link")
            .enable_all()
            .build()
            .map_err(|e| format!("can't start the web link ({})", e.kind()))?;
        let handle = rt.handle().clone();
        Ok(Link {
            inner: Self::make(cfg),
            handle,
            owned: Mutex::new(Some(rt)),
        })
    }

    /// A link on a runtime that is already running (tests). Its blocking
    /// methods need a multi-threaded runtime.
    pub fn start_on(handle: tokio::runtime::Handle, cfg: LinkConfig) -> Link {
        Link {
            inner: Self::make(cfg),
            handle,
            owned: Mutex::new(None),
        }
    }

    fn block<T: Send + 'static>(
        &self,
        fut: impl std::future::Future<Output = T> + Send + 'static,
    ) -> Option<T> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.handle.spawn(async move {
            let _ = tx.send(fut.await);
        });
        rx.recv().ok()
    }

    /// The link as it stands: a read of memory, never a wait.
    pub fn status(&self) -> LinkStatus {
        self.inner.status.borrow().clone()
    }

    /// Changes of status, for a caller that wants to wait.
    pub fn watch(&self) -> watch::Receiver<LinkStatus> {
        self.inner.status.subscribe()
    }

    /// The address kept for this installation, whether or not the link is
    /// on (it is the same when turned on again). `None` before the first
    /// time it was turned on. Read from the folder once and held: the
    /// window asks every frame, and the key beside it is not read again
    /// for that (it is read where it signs, in `dial`).
    pub fn kept_address(&self) -> Option<String> {
        let mut kept = self.inner.kept.lock().unwrap_or_else(|e| e.into_inner());
        kept.get_or_insert_with(|| {
            Identity::load(&self.inner.cfg.dir)
                .ok()
                .flatten()
                .map(|i| i.address(&self.inner.cfg.domain))
        })
        .clone()
    }

    /// Wait until the relay has accepted this installation's name, so the
    /// address is settled, or say why not: the door is started as this
    /// address, and a first claim that finds the name taken picks another
    /// (INVARIANT 5 of web.rs).
    pub fn wait_settled(&self, within: Duration) -> Result<(), String> {
        let mut rx = self.inner.status.subscribe();
        self.block(async move {
            // Copied out at once: the borrow of the watch is not held.
            let seen = match timeout(
                within,
                rx.wait_for(|s| s.claimed || s.trouble.as_ref().is_some_and(|t| !t.mends_itself())),
            )
            .await
            {
                Ok(Ok(r)) => Some(r.clone()),
                Ok(Err(_)) => return Err("The link is shutting down.".to_string()),
                Err(_) => None,
            };
            let s = match seen {
                Some(s) => s,
                None => rx.borrow().clone(),
            };
            if s.claimed {
                Ok(())
            } else {
                Err(s.problem().unwrap_or_else(|| {
                    "Wanderbooks has not been given its address by the relay yet.".into()
                }))
            }
        })
        .unwrap_or_else(|| Err("The link is shutting down.".into()))
    }

    /// Where the books server's remote door listens, or none.
    pub fn set_door(&self, door: Option<DoorTarget>) {
        *self.inner.door.write().unwrap_or_else(|e| e.into_inner()) = door;
    }

    /// Turn the link on: make or read the identity, and start the line
    /// and the certificate. Returns when they are started, not when they
    /// are done: [`Link::status`] says how far they are.
    pub fn turn_on(&self) -> Result<(), LinkError> {
        let me = self.inner.clone();
        self.block(async move { me.turn_on().await })
            .unwrap_or(Err(LinkError::Gone))
    }

    /// Turn the link off: the line ends and the certificate is no longer
    /// served before this returns. The name is kept.
    pub fn turn_off(&self) {
        let me = self.inner.clone();
        let _ = self.block(async move { me.turn_off().await });
    }

    /// A new address (INVARIANT 5): the address it will have.
    pub fn new_address(&self) -> Result<String, LinkError> {
        let me = self.inner.clone();
        self.block(async move { me.new_address().await })
            .unwrap_or(Err(LinkError::Gone))
    }

    /// The network may have changed: dial again now.
    pub fn nudge(&self) {
        self.inner.nudge.notify_waiters();
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        // Nothing outlives the link (INVARIANT 1): the line is closed, then
        // the runtime, if it is ours, is let go without waiting.
        let me = self.inner.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.handle.spawn(async move {
            me.turn_off().await;
            let _ = tx.send(());
        });
        let _ = rx.recv_timeout(Duration::from_secs(6));
        if let Some(rt) = self.owned.lock().unwrap_or_else(|e| e.into_inner()).take() {
            rt.shutdown_background();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wait_never_runs_past_the_end_of_the_certificate_in_use() {
        let hours = Duration::from_secs(6 * 3600);
        let t = now();
        // Nothing in use: the wait is as long as asked.
        assert_eq!(Inner::until(hours, None), hours);
        // One that ends in a minute: the wait ends with it.
        let w = Inner::until(hours, Some(t + 60));
        assert!(
            w <= Duration::from_secs(60) && w >= Duration::from_secs(58),
            "{w:?}"
        );
        // One that has ended already: no wait at all.
        assert_eq!(Inner::until(hours, Some(t - 5)), Duration::ZERO);
        // A short wait stays short.
        assert_eq!(
            Inner::until(Duration::from_secs(5), Some(t + 3600)),
            Duration::from_secs(5)
        );
    }

    #[test]
    fn authorities_that_all_fail_are_told_in_bounded_words() {
        let many: Vec<String> = (0..20)
            .map(|i| format!("authority {i}: {}", "x".repeat(2000)))
            .collect();
        let said = problems_in_words(&many);
        assert!(said.chars().count() < 1000, "{}", said.chars().count());
        assert!(said.ends_with("And 17 more."), "{said}");
    }
}
