//! The app's side of the line: dial out, prove the name, hold the line.
//!
//! This is the client the tests use, and the part of the protocol the
//! desktop app (unit B3) builds on. It does not do the app's policy:
//! when to dial, when to renew, what to do with a caller.
//!
//! ```text
//! GOAL: connect to the relay, prove the key for a name for this session
//!       only, and hand the app each caller with the caller's address.
//! SOURCE: WB-relay-2 (the line and its proof), WB-relay-6 (the caller's
//!         address).
//! INVARIANTS:
//!   1. The proof signs this session's TLS exporter and the relay's fresh
//!      nonce: it cannot be reused in another session.
//!   2. A caller stream is handed on only after its address header
//!      parsed; a stream with a bad header is dropped.
//!   3. Pending headers are bounded in number and in time.
//! SCENARIOS:
//!   1. `connect` with a new name and `claim = true` gives a line with
//!      `verdict.claimed`; a second connect with `claim = false` and the
//!      same key gives a line, and with another key is refused.
//! ```

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use tokio::net::TcpStream;
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;
use tokio_util::compat::TokioAsyncReadCompatExt;

use crate::mux::{self, Mux, MuxLimits, Stream};
use crate::names::Name;
use crate::proto::{
    self, AppKey, Authority, Caller, Challenge, EXPORTER_LABEL, Hello, Refusal, Request, Response,
    VERSION, Verdict, read_frame, write_frame,
};

/// Why a line could not be made, or a request failed.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The network or TLS failed.
    #[error("i/o: {0}")]
    Io(#[from] io::Error),
    /// A message was malformed or too long.
    #[error("protocol: {0}")]
    Frame(#[from] proto::FrameError),
    /// The relay said no.
    #[error("the relay refused the line: {0:?}")]
    Refused(Refusal),
    /// The relay did not answer in time.
    #[error("timed out")]
    Timeout,
    /// The line is closed.
    #[error("the line is closed")]
    Closed,
}

/// A caller handed to the app.
pub struct Incoming {
    /// Who the caller is, as the relay saw them.
    pub caller: Caller,
    /// The caller's bytes: their TLS ClientHello first.
    pub stream: Stream,
}

/// Options for a line.
#[derive(Debug, Clone, Copy)]
pub struct LineOptions {
    /// Streams at once on the line. At least the relay's `streams_per_app`
    /// plus `control_streams_per_app` plus 4, or the relay's opens are
    /// refused.
    pub max_streams: usize,
    /// How long a caller stream may take to send its address header.
    pub header_timeout: Duration,
    /// How long a request waits for its answer.
    pub request_timeout: Duration,
}

impl Default for LineOptions {
    fn default() -> Self {
        LineOptions {
            max_streams: 64,
            header_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(10),
        }
    }
}

/// An app's open line.
pub struct Line {
    /// The relay's verdict: claimed or not, and the stream limit.
    pub verdict: Verdict,
    handle: LineHandle,
    incoming: mpsc::Receiver<Incoming>,
    task: JoinHandle<()>,
}

/// TLS to the relay, as `server_name` (the relay's host).
pub async fn tls_connect(
    addr: SocketAddr,
    tls: Arc<ClientConfig>,
    server_name: &str,
) -> io::Result<tokio_rustls::client::TlsStream<TcpStream>> {
    let tcp = TcpStream::connect(addr).await?;
    tcp.set_nodelay(true)?;
    let name = ServerName::try_from(server_name.to_owned())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    TlsConnector::from(tls).connect(name, tcp).await
}

/// This TLS session's exporter value, the thing a proof is bound to.
pub fn session_exporter(tls: &tokio_rustls::client::TlsStream<TcpStream>) -> io::Result<[u8; 32]> {
    let mut out = [0u8; 32];
    tls.get_ref()
        .1
        .export_keying_material(&mut out[..], EXPORTER_LABEL, None)
        .map_err(|e| io::Error::other(e.to_string()))?;
    Ok(out)
}

/// The hello for a session: the name, the key, and the signature.
pub fn make_hello(
    key: &AppKey,
    name: &Name,
    claim: bool,
    exporter: &[u8; 32],
    nonce: &[u8; 32],
) -> Hello {
    Hello {
        v: VERSION,
        name: name.to_string(),
        public_key: proto::hex(&key.public_key()),
        claim,
        signature: proto::hex(&key.sign(&proto::proof_message(exporter, nonce, name, claim))),
    }
}

/// Dial the relay and bring up the line for `name`. `claim` is true only
/// the first time, to take the name.
pub async fn connect(
    addr: SocketAddr,
    tls: Arc<ClientConfig>,
    server_name: &str,
    name: Name,
    key: &AppKey,
    claim: bool,
    opts: LineOptions,
) -> Result<Line, ClientError> {
    let mut stream = timeout(opts.request_timeout, tls_connect(addr, tls, server_name))
        .await
        .map_err(|_| ClientError::Timeout)??;
    let exporter = session_exporter(&stream)?;
    let challenge: Challenge = timeout(opts.request_timeout, read_frame(&mut stream))
        .await
        .map_err(|_| ClientError::Timeout)??;
    let nonce = proto::unhex::<32>(&challenge.nonce)
        .ok_or_else(|| ClientError::Io(io::Error::new(io::ErrorKind::InvalidData, "bad nonce")))?;
    let hello = make_hello(key, &name, claim, &exporter, &nonce);
    write_frame(&mut stream, &hello).await?;
    let verdict: Verdict = timeout(opts.request_timeout, read_frame(&mut stream))
        .await
        .map_err(|_| ClientError::Timeout)??;
    if !verdict.ok {
        return Err(ClientError::Refused(
            verdict.refusal.unwrap_or(Refusal::BadRequest),
        ));
    }
    Ok(Line::up(stream, verdict, opts))
}

impl Line {
    /// Start the multiplexer over a TLS stream that has passed the
    /// handshake.
    pub fn up(
        stream: tokio_rustls::client::TlsStream<TcpStream>,
        verdict: Verdict,
        opts: LineOptions,
    ) -> Line {
        let (tx, incoming) = mpsc::channel::<Incoming>(16);
        let pending = Arc::new(Semaphore::new(64));
        let header_timeout = opts.header_timeout;
        let (mux, task) = mux::spawn(
            stream.compat(),
            yamux::Mode::Client,
            MuxLimits {
                max_streams: opts.max_streams,
                idle: Duration::from_secs(3600 * 24 * 365),
            },
            None,
            move |mut stream| {
                let Ok(permit) = Arc::clone(&pending).try_acquire_owned() else {
                    return; // too many callers waiting to be read: drop
                };
                let tx = tx.clone();
                tokio::spawn(async move {
                    let header = timeout(header_timeout, Caller::read(&mut stream)).await;
                    drop(permit);
                    if let Ok(Ok(caller)) = header {
                        let _ = tx.try_send(Incoming { caller, stream });
                    }
                });
            },
        );
        Line {
            verdict,
            handle: LineHandle { mux, opts },
            incoming,
            task,
        }
    }

    /// The next caller, or `None` when the line has ended.
    pub async fn accept(&mut self) -> Option<Incoming> {
        self.incoming.recv().await
    }

    /// A clonable handle for requests and closing, for use beside
    /// [`Line::accept`].
    pub fn handle(&self) -> LineHandle {
        self.handle.clone()
    }

    /// Close the line and wait for it.
    pub async fn close(self) {
        self.handle.mux.close();
        let _ = self.task.await;
    }
}

/// What can be done with a line besides taking callers: requests, and
/// closing. Clones share the line.
#[derive(Clone)]
pub struct LineHandle {
    mux: Mux,
    opts: LineOptions,
}

impl LineHandle {
    /// One request on a stream of its own.
    pub async fn request(&self, request: Request) -> Result<Response, ClientError> {
        let wait = self.opts.request_timeout;
        let mut stream = self.mux.open(wait).await.map_err(|_| ClientError::Closed)?;
        timeout(wait, async {
            write_frame(&mut stream, &request).await?;
            let r: Response = read_frame(&mut stream).await?;
            Ok::<_, ClientError>(r)
        })
        .await
        .map_err(|_| ClientError::Timeout)?
    }

    /// Ask the relay to publish an ACME DNS-01 proof value for this
    /// app's name.
    pub async fn publish_txt(&self, value: &str) -> Result<Response, ClientError> {
        self.request(Request::PublishTxt {
            value: value.to_owned(),
        })
        .await
    }

    /// The certificate authorities to try, in order.
    pub async fn authorities(&self) -> Result<Vec<Authority>, ClientError> {
        let r = self.request(Request::Authorities).await?;
        Ok(r.authorities.unwrap_or_default())
    }

    /// True once the line has ended.
    pub fn is_closed(&self) -> bool {
        self.mux.is_closed()
    }

    /// Ask the line to close.
    pub fn close(&self) {
        self.mux.close();
    }
}

impl std::ops::Deref for Line {
    type Target = LineHandle;
    fn deref(&self) -> &LineHandle {
        &self.handle
    }
}
