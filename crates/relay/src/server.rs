//! The relay itself: the router, the lines, the control streams, DNS.
//!
//! ```text
//! GOAL: route an assistant's TLS connection, by the server name in its
//!       first message, down the line of the app that owns the name, and
//!       forward the encrypted bytes both ways, holding no certificate
//!       for any app and reading nothing; and keep every table, buffer
//!       and queue bounded whoever connects.
//! SOURCE: WB-relay-1 (the router), WB-relay-2 (the line and its proof),
//!         WB-relay-6 (the caller's address), WB-relay-7 (bounds), D-80.
//! INVARIANTS:
//!   1. The router never terminates TLS for an app's name: for a name
//!      under the domain it reads only the ClientHello's server name and
//!      then copies bytes. The only TLS the relay ends is the line's, for
//!      its own host.
//!   2. An unknown name, an app that is not connected, an app over its
//!      stream limit, and every malformed first message are closed in the
//!      same way (a TLS alert, then close): a stranger cannot tell which.
//!   3. The caller's address is written by the relay, as the first bytes
//!      on the stream, before any byte of the caller's; it is the socket's
//!      peer address, never a value the caller sent.
//!   4. A name's line is replaced when its owner reconnects with a good
//!      proof, and removed when it ends; the table holds one entry per
//!      connected app and no more than `max_lines`.
//!   5. Every connection is counted against a global limit and a
//!      per-address and per-network limit (an IPv4 address and /24, an
//!      IPv6 /64 and /48; a mapped IPv4 address is its IPv4 address)
//!      before any work is done for it, and a hello that is not complete
//!      in `hello_timeout` is closed. When the pool of unfinished hellos
//!      is full, an address that held a proven line lately may still use
//!      a small reserve, so lines can reconnect through a flood.
//!   6. Per app: at most `streams_per_app` caller streams and
//!      `control_streams_per_app` control streams, each with its own
//!      timeouts; nothing queues without a bound.
//!   7. A line handshake holds a place for its address and its network
//!      from the moment the router sees the relay's own host name until
//!      the verdict, so one address cannot hold the pool of handshakes.
//!   8. A name that is not the caller's, unclaimed, or blocked, to a
//!      caller who has not shown its key, is refused with one answer.
//!   9. A blocked name's live line is closed: at its next publish, and
//!      within `block_check_ms` in any case.
//!  10. Claiming a name costs the registry's safety ceiling and the
//!      per-prefix claim limits; the certificate authority's weekly
//!      budget is charged when a name first publishes a proof record,
//!      under per-prefix limits of its own.
//! SCENARIOS:
//!   1. An assistant connects with the app's name as server name; the app
//!      sees the caller's address, then the assistant's ClientHello, and
//!      TLS completes between them with the app's own certificate.
//!   2. An unknown name, and an offline app, are closed with the same
//!      alert.
//!   3. A peer that connects and sends nothing, or one byte a second, is
//!      closed at the hello timeout.
//!   4. A client holding the right key but a recorded old proof is
//!      refused.
//!   5. An unclaimed name, a name held by another key and a blocked name
//!      get the same refusal when the key is not the name's.
//!   6. The operator blocks a name whose line is up: the line is closed
//!      and does not come back.
//! ```

use std::collections::HashMap;
use std::hash::{BuildHasher, RandomState};
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_rustls::TlsAcceptor;
use tokio_util::compat::TokioAsyncReadCompatExt;
use tokio_util::sync::CancellationToken;

use crate::authorities::AuthorityList;
use crate::config::Config;
use crate::dns::{DnsService, TcpLimits};
use crate::front::{Activity, Front};
use crate::gate::{IpGate, IpPermit, KnownAddrs, PrefixLimit};
use crate::hello::{self, HelloError};
use crate::mux::{self, Mux, MuxLimits};
use crate::names::{HostKind, Name, Zone};
use crate::proto::{
    self, Caller, Challenge, EXPORTER_LABEL, Hello, Refusal, Request, Response, VERSION, Verdict,
    read_frame, write_frame,
};
use crate::registry::{Charged, Claimed, Registry};
use crate::splice::{self, SpliceLimits};
use crate::txt::{PublishError, TxtStore};

/// What happened, counted. Read by tests and printed by the binary.
#[derive(Default, Debug)]
pub struct Metrics {
    /// Connections accepted by the router.
    pub accepted: AtomicU64,
    /// Connections dropped at once because a limit was reached.
    pub shed: AtomicU64,
    /// Hellos not finished in time.
    pub hello_timeouts: AtomicU64,
    /// First bytes that were not a ClientHello with a name.
    pub bad_hello: AtomicU64,
    /// Names that are not an app's, or apps that are not connected.
    pub unknown_or_offline: AtomicU64,
    /// Callers whose app was over its stream limit.
    pub app_busy: AtomicU64,
    /// Streams the app did not open in time, or refused.
    pub open_failed: AtomicU64,
    /// Callers spliced to an app.
    pub routed: AtomicU64,
    /// Line handshakes begun.
    pub line_attempts: AtomicU64,
    /// Lines refused for a bad or replayed proof.
    pub refused_proof: AtomicU64,
    /// Lines refused for any other reason.
    pub refused_other: AtomicU64,
    /// Names claimed for the first time.
    pub claims: AtomicU64,
    /// Names charged to the certificate budget (first publishes).
    pub cert_charges: AtomicU64,
    /// Lines closed because their name was blocked.
    pub blocked_closed: AtomicU64,
    /// Control requests served.
    pub control: AtomicU64,
    /// Lines up now.
    pub lines_up: AtomicUsize,
}

impl Metrics {
    /// Add one to a counter.
    pub fn bump(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

/// An app's open line, as the router uses it.
#[derive(Clone)]
pub struct LineHandle {
    id: u64,
    mux: Mux,
    streams: Arc<Semaphore>,
}

/// The routing table: name to open line, in memory, rebuilt as apps
/// reconnect. Sharded so no lock is held by many.
pub struct LineMap {
    shards: Vec<Mutex<HashMap<Name, LineHandle>>>,
    state: RandomState,
    count: AtomicUsize,
    next: AtomicU64,
    max: usize,
}

impl LineMap {
    fn new(max: usize) -> Self {
        LineMap {
            shards: (0..256).map(|_| Mutex::new(HashMap::new())).collect(),
            state: RandomState::new(),
            count: AtomicUsize::new(0),
            next: AtomicU64::new(1),
            max,
        }
    }

    fn shard(&self, name: &Name) -> &Mutex<HashMap<Name, LineHandle>> {
        // Truncating the hash to a shard index is the point.
        &self.shards[(self.state.hash_one(name) as usize) % self.shards.len()]
    }

    fn lock<'a>(
        m: &'a Mutex<HashMap<Name, LineHandle>>,
    ) -> std::sync::MutexGuard<'a, HashMap<Name, LineHandle>> {
        m.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The line of `name`, if connected.
    pub fn get(&self, name: &Name) -> Option<LineHandle> {
        Self::lock(self.shard(name)).get(name).cloned()
    }

    /// Lines up.
    pub fn len(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }

    /// True when no line is up.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether a line for `name` could be taken now.
    fn has_room(&self, name: &Name) -> bool {
        self.len() < self.max || Self::lock(self.shard(name)).contains_key(name)
    }

    fn insert(&self, name: Name, handle: LineHandle) -> Result<(), ()> {
        let mut shard = Self::lock(self.shard(&name));
        let replaced = shard.contains_key(&name);
        if !replaced && self.count.load(Ordering::Relaxed) >= self.max {
            return Err(());
        }
        if let Some(old) = shard.insert(name, handle) {
            old.mux.close(); // the owner reconnected: the old line is done
        } else {
            self.count.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    fn remove_if(&self, name: &Name, id: u64) {
        let mut shard = Self::lock(self.shard(name));
        if shard.get(name).is_some_and(|h| h.id == id) {
            shard.remove(name);
            self.count.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// Close the line of `name`, if one is up. True when there was one.
    fn close_name(&self, name: &Name) -> bool {
        match self.get(name) {
            Some(h) => {
                h.mux.close();
                true
            }
            None => false,
        }
    }

    fn close_all(&self) {
        for s in &self.shards {
            for h in Self::lock(s).values() {
                h.mux.close();
            }
        }
    }
}

/// Everything the tasks share.
pub struct Shared {
    /// The configuration.
    pub cfg: Config,
    zone: Zone,
    /// Who is connected.
    pub lines: LineMap,
    /// The durable table.
    pub registry: Registry,
    /// Proof records.
    pub txt: Arc<TxtStore>,
    /// The certificate authorities offered.
    pub authorities: Arc<AuthorityList>,
    acceptor: TlsAcceptor,
    /// What happened.
    pub metrics: Metrics,
    conn_permits: Arc<Semaphore>,
    hello_permits: Arc<Semaphore>,
    hello_reserve: Arc<Semaphore>,
    handshake_permits: Arc<Semaphore>,
    conn_gate: Arc<IpGate>,
    pending_gate: Arc<IpGate>,
    pending_net: Arc<IpGate>,
    handshake_gate: Arc<IpGate>,
    handshake_net: Arc<IpGate>,
    known: KnownAddrs,
    claim_ips: PrefixLimit,
    cert_ips: PrefixLimit,
}

/// Everything a connection holds from accept until its hello is read.
struct Admission {
    conn: OwnedSemaphorePermit,
    conn_ip: IpPermit,
    pending_ip: IpPermit,
    pending_net: IpPermit,
    hello: OwnedSemaphorePermit,
}

impl Shared {
    /// Count a new connection from `ip` against every limit, or say no.
    fn admit(&self, ip: std::net::IpAddr) -> Option<Admission> {
        let conn = Arc::clone(&self.conn_permits).try_acquire_owned().ok()?;
        let conn_ip = self.conn_gate.acquire(ip)?;
        let pending_ip = self.pending_gate.acquire(ip)?;
        let pending_net = self.pending_net.acquire(ip)?;
        // The server name is not known until the hello is read, so the
        // reserve is for who is calling: an address that held a proven
        // line lately. It cannot be made by an address that never proved a
        // name.
        let hello = match Arc::clone(&self.hello_permits).try_acquire_owned() {
            Ok(p) => p,
            Err(_) if self.known.contains(ip, Instant::now()) => {
                Arc::clone(&self.hello_reserve).try_acquire_owned().ok()?
            }
            Err(_) => return None,
        };
        Some(Admission {
            conn,
            conn_ip,
            pending_ip,
            pending_net,
            hello,
        })
    }
}

/// A TLS fatal alert, `unrecognized_name`: the same for every refusal.
const ALERT: [u8; 7] = [0x15, 3, 3, 0, 2, 2, 112];

async fn refuse(tcp: &mut TcpStream) {
    let _ = timeout(Duration::from_secs(1), async {
        tcp.write_all(&ALERT).await?;
        tcp.shutdown().await
    })
    .await;
}

/// Read the first TLS record. Memory grows only as bytes arrive.
async fn read_hello(tcp: &mut TcpStream) -> Result<(Vec<u8>, String), HelloError> {
    let eof = || HelloError::Malformed;
    let mut head = [0u8; hello::RECORD_HEADER];
    tcp.read_exact(&mut head).await.map_err(|_| eof())?;
    let len = hello::record_len(&head)?;
    let total = hello::RECORD_HEADER + len;
    let mut rec = Vec::with_capacity(hello::RECORD_HEADER + 1024);
    rec.extend_from_slice(&head);
    while rec.len() < total {
        let at = rec.len();
        let want = (total - at).min(4096);
        rec.resize(at + want, 0);
        let n = tcp.read(&mut rec[at..]).await.map_err(|_| eof())?;
        rec.truncate(at + n);
        if n == 0 {
            return Err(eof());
        }
    }
    let host = hello::server_name(&rec)?;
    Ok((rec, host))
}

async fn handle(ctx: Arc<Shared>, mut tcp: TcpStream, peer: SocketAddr, admission: Admission) {
    let Admission {
        conn,
        conn_ip,
        pending_ip,
        pending_net,
        hello,
    } = admission;
    let _held = (conn, conn_ip);
    let _ = tcp.set_nodelay(true);
    let first = timeout(ctx.cfg.limits.hello_timeout(), read_hello(&mut tcp)).await;
    drop((pending_ip, pending_net, hello));
    let (record, host) = match first {
        Err(_) => {
            Metrics::bump(&ctx.metrics.hello_timeouts);
            return;
        }
        Ok(Err(_)) => {
            Metrics::bump(&ctx.metrics.bad_hello);
            return;
        }
        Ok(Ok(x)) => x,
    };
    match ctx.zone.classify(&host) {
        HostKind::Relay => serve_line(ctx, tcp, record, peer).await,
        HostKind::App(name) => route(ctx, tcp, record, peer, name).await,
        HostKind::Other => {
            Metrics::bump(&ctx.metrics.unknown_or_offline);
            refuse(&mut tcp).await;
        }
    }
}

async fn route(
    ctx: Arc<Shared>,
    mut tcp: TcpStream,
    record: Vec<u8>,
    peer: SocketAddr,
    name: Name,
) {
    let l = &ctx.cfg.limits;
    let Some(line) = ctx.lines.get(&name) else {
        Metrics::bump(&ctx.metrics.unknown_or_offline);
        return refuse(&mut tcp).await;
    };
    let Ok(_stream_permit) = Arc::clone(&line.streams).try_acquire_owned() else {
        Metrics::bump(&ctx.metrics.app_busy);
        return refuse(&mut tcp).await;
    };
    let Ok(mut stream) = line.mux.open(l.open_stream_timeout()).await else {
        Metrics::bump(&ctx.metrics.open_failed);
        return refuse(&mut tcp).await;
    };
    let Ok(local) = tcp.local_addr() else {
        return;
    };
    // The caller's address first, written by us from the socket, then the
    // caller's own first bytes: nothing the caller sends can come before.
    let mut first = Caller {
        source: peer,
        destination: local,
    }
    .encode();
    first.extend_from_slice(&record);
    drop(record);
    let sent = timeout(l.write_stall(), async {
        stream.write_all(&first).await?;
        stream.flush().await
    })
    .await;
    if !matches!(sent, Ok(Ok(()))) {
        Metrics::bump(&ctx.metrics.open_failed);
        return;
    }
    drop(first);
    Metrics::bump(&ctx.metrics.routed);
    let _ = splice::splice(
        tcp,
        stream,
        SpliceLimits {
            buffer: l.copy_buffer,
            idle: l.stream_idle(),
            stall: l.write_stall(),
            min_rate: l.min_stream_bytes_per_sec,
            max_age: l.stream_max_age(),
        },
    )
    .await;
}

type LineTls = tokio_rustls::server::TlsStream<Front<TcpStream>>;

async fn serve_line(ctx: Arc<Shared>, tcp: TcpStream, record: Vec<u8>, peer: SocketAddr) {
    let l = &ctx.cfg.limits;
    let ip = peer.ip().to_canonical();
    // The pool of handshakes, and this address's and network's share of
    // it, held until the verdict: the hello's own permit is long gone, so
    // without these one address could fill the pool by finishing a hello
    // and stalling, over and over.
    let (Ok(pool), Some(per_ip), Some(per_net)) = (
        Arc::clone(&ctx.handshake_permits).try_acquire_owned(),
        ctx.handshake_gate.acquire(ip),
        ctx.handshake_net.acquire(ip),
    ) else {
        Metrics::bump(&ctx.metrics.shed);
        return;
    };
    let handshaking = (pool, per_ip, per_net);
    Metrics::bump(&ctx.metrics.line_attempts);
    let activity = Activity::new();
    let front = Front::new(record, tcp, Some(Arc::clone(&activity)));
    let up = timeout(l.handshake_timeout(), handshake(&ctx, front, ip)).await;
    drop(handshaking);
    let Ok(Ok((tls, name))) = up else { return };
    ctx.known.mark(ip, Instant::now());
    // A control request that finds the name blocked ends the line by this.
    let kick = CancellationToken::new();

    let streams = Arc::new(Semaphore::new(l.streams_per_app as usize));
    let control = Arc::new(Semaphore::new(l.control_streams_per_app as usize));
    let limits = MuxLimits {
        // The permits, and a few over: a stream the app has closed may
        // still be counted on its side for a moment after our permit is
        // back, and yamux ends a line that is asked for a stream over its
        // count. Every stream may hold a 256 KiB receive window, so the
        // few are few.
        max_streams: (l.streams_per_app + l.control_streams_per_app + STREAM_SLACK) as usize,
        idle: l.line_idle(),
    };
    let ctx2 = Arc::clone(&ctx);
    let kick2 = kick.clone();
    let (mux, mut task) = mux::spawn(
        tls.compat(),
        yamux::Mode::Server,
        limits,
        Some(activity),
        move |stream| {
            if let Ok(permit) = Arc::clone(&control).try_acquire_owned() {
                let ctx = Arc::clone(&ctx2);
                let kick = kick2.clone();
                tokio::spawn(async move {
                    serve_control(ctx, name, ip, kick, stream).await;
                    drop(permit);
                });
            }
        },
    );
    let id = ctx.lines.next.fetch_add(1, Ordering::Relaxed);
    let handle = LineHandle {
        id,
        mux: mux.clone(),
        streams,
    };
    if ctx.lines.insert(name, handle).is_err() {
        mux.close();
        let _ = task.await;
        return;
    }
    ctx.metrics.lines_up.fetch_add(1, Ordering::Relaxed);
    tokio::select! {
        _ = &mut task => {}
        () = kick.cancelled() => {
            mux.close();
            let _ = task.await;
        }
    }
    ctx.lines.remove_if(&name, id);
    ctx.metrics.lines_up.fetch_sub(1, Ordering::Relaxed);
}

/// Streams over the permits that a line may count at once. Each can hold
/// a 256 KiB receive window; see `mux`.
const STREAM_SLACK: u32 = 4;

/// TLS, the challenge, the proof, the verdict.
async fn handshake(
    ctx: &Arc<Shared>,
    front: Front<TcpStream>,
    ip: std::net::IpAddr,
) -> Result<(LineTls, Name), ()> {
    let mut tls = ctx.acceptor.accept(front).await.map_err(|_| ())?;
    let mut exporter = [0u8; 32];
    tls.get_ref()
        .1
        .export_keying_material(&mut exporter[..], EXPORTER_LABEL, None)
        .map_err(|_| ())?;
    let nonce = proto::random32();
    write_frame(
        &mut tls,
        &Challenge {
            v: VERSION,
            nonce: proto::hex(&nonce),
        },
    )
    .await
    .map_err(|_| ())?;
    let hello: Hello = read_frame(&mut tls).await.map_err(|_| ())?;
    let (verdict, name) = decide(ctx, &hello, &exporter, &nonce, ip).await;
    write_frame(&mut tls, &verdict).await.map_err(|_| ())?;
    match name {
        Some(n) if verdict.ok => Ok((tls, n)),
        _ => {
            let _ = timeout(Duration::from_secs(1), tls.shutdown()).await;
            Err(())
        }
    }
}

fn refused(why: Refusal) -> (Verdict, Option<Name>) {
    (
        Verdict {
            ok: false,
            refusal: Some(why),
            claimed: false,
            max_streams: 0,
        },
        None,
    )
}

/// Is this hello a proof, for a name and key the registry will take?
async fn decide(
    ctx: &Arc<Shared>,
    hello: &Hello,
    exporter: &[u8; 32],
    nonce: &[u8; 32],
    ip: std::net::IpAddr,
) -> (Verdict, Option<Name>) {
    let l = &ctx.cfg.limits;
    let (Ok(name), Some(key), Some(sig), true) = (
        Name::parse(&hello.name),
        proto::unhex::<32>(&hello.public_key),
        proto::unhex::<64>(&hello.signature),
        hello.v == VERSION,
    ) else {
        Metrics::bump(&ctx.metrics.refused_other);
        return refused(Refusal::BadRequest);
    };
    // The proof first, before any database work: a stranger without a
    // key never reaches the registry.
    if !proto::verify(
        &key,
        &proto::proof_message(exporter, nonce, &name, hello.claim),
        &sig,
    ) {
        Metrics::bump(&ctx.metrics.refused_proof);
        return refused(Refusal::BadProof);
    }
    let mut claimed = false;
    match ctx.registry.lookup(&name).await {
        Err(_) => {
            Metrics::bump(&ctx.metrics.refused_other);
            return refused(Refusal::Busy);
        }
        Ok(Some(e)) => {
            // The key first: only the holder of the name's key is told it
            // is blocked. Anyone else gets what a wrong key gets, whether
            // the name is blocked or not.
            if e.public_key != key {
                Metrics::bump(&ctx.metrics.refused_proof);
                return refused(if hello.claim {
                    Refusal::Taken
                } else {
                    Refusal::BadProof
                });
            }
            if e.blocked {
                Metrics::bump(&ctx.metrics.refused_other);
                return refused(Refusal::Blocked);
            }
        }
        Ok(None) => {
            if !hello.claim {
                // The same answer as a wrong key: a caller with a key of
                // its own cannot ask which names exist.
                Metrics::bump(&ctx.metrics.refused_proof);
                return refused(Refusal::BadProof);
            }
            if !ctx.claim_ips.allow(ip, Instant::now()) {
                Metrics::bump(&ctx.metrics.refused_other);
                return refused(Refusal::RateLimited);
            }
            match ctx
                .registry
                .claim(&name, key, l.claim_window(), l.claims_per_window)
                .await
            {
                Ok(Claimed::New) => {
                    claimed = true;
                    Metrics::bump(&ctx.metrics.claims);
                }
                Ok(Claimed::Exists(e)) if e.public_key == key && !e.blocked => {}
                Ok(Claimed::Exists(_)) => {
                    Metrics::bump(&ctx.metrics.refused_proof);
                    return refused(Refusal::Taken);
                }
                Ok(Claimed::LimitReached) => {
                    Metrics::bump(&ctx.metrics.refused_other);
                    return refused(Refusal::RateLimited);
                }
                Err(_) => {
                    Metrics::bump(&ctx.metrics.refused_other);
                    return refused(Refusal::Busy);
                }
            }
        }
    }
    if !ctx.lines.has_room(&name) {
        Metrics::bump(&ctx.metrics.refused_other);
        return refused(Refusal::Busy);
    }
    (
        Verdict {
            ok: true,
            refusal: None,
            claimed,
            max_streams: l.streams_per_app,
        },
        Some(name),
    )
}

/// One request from an app on its own line. The name is the line's.
async fn serve_control(
    ctx: Arc<Shared>,
    name: Name,
    ip: std::net::IpAddr,
    kick: CancellationToken,
    mut stream: mux::Stream,
) {
    let _ = timeout(ctx.cfg.limits.control_timeout(), async {
        let request: Request = read_frame(&mut stream).await.ok()?;
        Metrics::bump(&ctx.metrics.control);
        let response = match request {
            Request::Ping => Response {
                ok: true,
                ..Response::default()
            },
            Request::PublishTxt { value } => publish(&ctx, name, ip, &kick, &value).await,
            Request::ClearTxt => {
                ctx.txt.clear(&name);
                Response {
                    ok: true,
                    ..Response::default()
                }
            }
            Request::Authorities => Response {
                ok: true,
                authorities: Some(ctx.authorities.for_app(&name)),
                ..Response::default()
            },
        };
        write_frame(&mut stream, &response).await.ok()?;
        let _ = stream.shutdown().await;
        Some(())
    })
    .await;
}

fn failure(why: &str) -> Response {
    Response {
        ok: false,
        error: Some(why.into()),
        ..Response::default()
    }
}

/// Publish a proof record for the line's own name.
///
/// The name is asked of the registry on every publish: a blocked name's
/// line is closed here at its next publish, and by the sweep in any case.
/// The first publish of a name is the one that costs the certificate
/// authority's weekly budget, under the same per-prefix limits as a claim.
async fn publish(
    ctx: &Arc<Shared>,
    name: Name,
    ip: std::net::IpAddr,
    kick: &CancellationToken,
    value: &str,
) -> Response {
    if !crate::txt::is_proof_value(value) {
        return failure("bad_value");
    }
    let l = &ctx.cfg.limits;
    let entry = match ctx.registry.lookup(&name).await {
        Ok(Some(e)) if !e.blocked => e,
        Ok(_) => {
            // Blocked (or gone): the line ends.
            Metrics::bump(&ctx.metrics.blocked_closed);
            kick.cancel();
            return failure("blocked");
        }
        Err(_) => return failure("busy"),
    };
    if !entry.published {
        if !ctx.cert_ips.allow(ip, Instant::now()) {
            return failure("limit");
        }
        match ctx
            .registry
            .charge_publish(&name, l.cert_window(), l.cert_publishes_per_window)
            .await
        {
            Ok(Charged::New) => Metrics::bump(&ctx.metrics.cert_charges),
            Ok(Charged::Already) => {}
            Ok(Charged::LimitReached) => return failure("limit"),
            Ok(Charged::NoSuchName) => return failure("bad_value"),
            Err(_) => return failure("busy"),
        }
    }
    match ctx.txt.publish(name, value, Instant::now()) {
        Ok(()) => Response {
            ok: true,
            ..Response::default()
        },
        Err(PublishError::BadValue) => failure("bad_value"),
        Err(PublishError::Limit) => failure("limit"),
    }
}

async fn accept_loop(ctx: Arc<Shared>, listener: TcpListener, cancel: CancellationToken) {
    loop {
        let (tcp, peer) = tokio::select! {
            () = cancel.cancelled() => return,
            r = listener.accept() => match r {
                Ok(x) => x,
                Err(_) => {
                    // Out of descriptors, or a reset: do not spin.
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            },
        };
        // An IPv4 caller on a dual-stack listener is `::ffff:a.b.c.d`:
        // made canonical, it is its IPv4 address in every table.
        let ip = peer.ip().to_canonical();
        let Some(admission) = ctx.admit(ip) else {
            Metrics::bump(&ctx.metrics.shed);
            continue; // closed by drop, before any work
        };
        Metrics::bump(&ctx.metrics.accepted);
        tokio::spawn(handle(Arc::clone(&ctx), tcp, peer, admission));
    }
}

/// A running relay.
pub struct Relay {
    shared: Arc<Shared>,
    router: SocketAddr,
    dns_udp: SocketAddr,
    dns_tcp: SocketAddr,
    cancel: CancellationToken,
    tasks: Vec<JoinHandle<()>>,
}

impl Relay {
    /// Bind and start the router and DNS. Port 0 picks a free port.
    pub async fn start(
        cfg: Config,
        tls: Arc<rustls::ServerConfig>,
        authorities: AuthorityList,
    ) -> io::Result<Relay> {
        let l = cfg.limits.clone();
        let zone = Zone::new(&cfg.domain, &cfg.relay_host);
        let registry = Registry::open(&cfg.registry_path, l.registry_queue)
            .map_err(|e| io::Error::other(e.to_string()))?;
        let txt = Arc::new(TxtStore::new(
            l.txt_ttl(),
            l.txt_max_entries,
            l.txt_per_name,
            l.txt_publishes_per_hour,
        ));
        let authorities = Arc::new(authorities);
        let dns = Arc::new(
            DnsService::new(
                zone.clone(),
                &cfg,
                Arc::clone(&txt),
                Arc::clone(&authorities),
            )
            .map_err(io::Error::other)?,
        );
        let router_listener = TcpListener::bind(cfg.router_addr).await?;
        let router = router_listener.local_addr()?;
        // TCP DNS on the same port number when it was asked for 0: a UDP
        // port that is free is not always free for TCP, so ask again.
        let mut attempt = 0;
        let (udp, dns_udp, dns_tcp_listener, dns_tcp) = loop {
            attempt += 1;
            let udp = UdpSocket::bind(cfg.dns_addr).await?;
            let dns_udp = udp.local_addr()?;
            let tcp_addr = SocketAddr::new(cfg.dns_addr.ip(), dns_udp.port());
            match TcpListener::bind(tcp_addr).await {
                Ok(l) => {
                    let dns_tcp = l.local_addr()?;
                    break (udp, dns_udp, l, dns_tcp);
                }
                Err(e) if cfg.dns_addr.port() == 0 && attempt < 16 => {
                    let _ = e;
                }
                Err(e) => return Err(e),
            }
        };

        let shared = Arc::new(Shared {
            zone,
            lines: LineMap::new(l.max_lines),
            registry,
            txt: Arc::clone(&txt),
            authorities,
            acceptor: TlsAcceptor::from(tls),
            metrics: Metrics::default(),
            conn_permits: Arc::new(Semaphore::new(l.max_connections)),
            hello_permits: Arc::new(Semaphore::new(l.max_pending_hellos)),
            hello_reserve: Arc::new(Semaphore::new(l.hello_reserve)),
            handshake_permits: Arc::new(Semaphore::new(l.max_handshakes)),
            conn_gate: IpGate::new(l.ip_slots, l.max_connections_per_ip),
            pending_gate: IpGate::new(l.ip_slots, l.max_pending_per_ip),
            pending_net: IpGate::net(l.ip_slots, l.max_pending_per_net),
            handshake_gate: IpGate::new(l.ip_slots, l.max_handshakes_per_ip),
            handshake_net: IpGate::net(l.ip_slots, l.max_handshakes_per_net),
            known: KnownAddrs::new(l.known_bits, l.known_period()),
            claim_ips: PrefixLimit::new(
                l.ip_slots,
                l.claims_per_ip,
                l.claims_per_net,
                l.claim_ip_window(),
            ),
            cert_ips: PrefixLimit::new(
                l.ip_slots,
                l.cert_publishes_per_ip,
                l.cert_publishes_per_net,
                l.cert_window(),
            ),
            cfg,
        });
        let cancel = CancellationToken::new();
        let mut tasks = vec![
            tokio::spawn(accept_loop(
                Arc::clone(&shared),
                router_listener,
                cancel.clone(),
            )),
            tokio::spawn(crate::dns::serve_udp(
                Arc::new(udp),
                Arc::clone(&dns),
                l.dns_qps_per_ip,
                l.dns_qps_global,
                l.ip_slots,
                cancel.clone(),
            )),
            tokio::spawn(crate::dns::serve_tcp(
                dns_tcp_listener,
                dns,
                TcpLimits {
                    max: l.dns_tcp_max,
                    per_ip: l.dns_tcp_per_ip,
                    timeout: l.dns_tcp_timeout(),
                    queries: 16,
                },
                l.ip_slots,
                cancel.clone(),
            )),
        ];
        // Housekeeping: expired proof records, a changed authority list.
        let (sweep_shared, sweep_cancel) = (Arc::clone(&shared), cancel.clone());
        tasks.push(tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(30));
            loop {
                tokio::select! {
                    () = sweep_cancel.cancelled() => return,
                    _ = tick.tick() => {
                        sweep_shared.txt.sweep(Instant::now());
                        let _ = sweep_shared.authorities.reload();
                    }
                }
            }
        }));
        // The operator blocks from another process, into the registry file;
        // this is what makes a block reach a line that is already up.
        let (block_shared, block_cancel) = (Arc::clone(&shared), cancel.clone());
        tasks.push(tokio::spawn(async move {
            let mut tick = tokio::time::interval(block_shared.cfg.limits.block_check());
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    () = block_cancel.cancelled() => return,
                    _ = tick.tick() => {
                        if let Ok(names) = block_shared.registry.blocked_names().await {
                            for n in names {
                                if block_shared.lines.close_name(&n) {
                                    Metrics::bump(&block_shared.metrics.blocked_closed);
                                }
                            }
                        }
                    }
                }
            }
        }));
        Ok(Relay {
            shared,
            router,
            dns_udp,
            dns_tcp,
            cancel,
            tasks,
        })
    }

    /// Where the router listens.
    pub fn router_addr(&self) -> SocketAddr {
        self.router
    }

    /// Where DNS listens over UDP.
    pub fn dns_udp_addr(&self) -> SocketAddr {
        self.dns_udp
    }

    /// Where DNS listens over TCP.
    pub fn dns_tcp_addr(&self) -> SocketAddr {
        self.dns_tcp
    }

    /// The shared state: tables and counters.
    pub fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    /// Stop accepting, close every line, and wait for the listeners.
    pub async fn shutdown(self) {
        self.cancel.cancel();
        self.shared.lines.close_all();
        for t in self.tasks {
            let _ = t.await;
        }
    }
}
