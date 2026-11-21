//! The wire protocol between an app and the relay.
//!
//! Three things travel the app's line, all small and bounded:
//!
//! 1. The **handshake**, once, right after the line's TLS: the relay sends
//!    a challenge; the app answers with its name, its public key and a
//!    signature; the relay says yes, or why not.
//! 2. **Caller streams**, opened by the relay: the caller's address in a
//!    PROXY protocol version 2 header, then the caller's own encrypted
//!    bytes.
//! 3. **Control streams**, opened by the app: one request, one reply.
//!
//! ```text
//! GOAL: an app proves it holds the key for its name on every connection,
//!       so that neither a stranger nor a replayed recording of an old
//!       proof can take the name's line; and the app is told the caller's
//!       real address in a form no caller can forge.
//! SOURCE: WB-relay-2 (the line and its proof), WB-relay-6 (the caller's
//!         address); RFC 8032 (Ed25519), RFC 5705 and RFC 8446 section 7.5
//!         (TLS exporters), the PROXY protocol specification version 2.
//! INVARIANTS:
//!   1. The proof signs the line's own TLS exporter value, a fresh 32-byte
//!      nonce, the name and the claim flag: it is good for one TLS
//!      session and no other, and no relay or stranger who sat in the
//!      middle of another session can use it.
//!   2. Every frame is at most 2 048 bytes and every field has a length:
//!      a stranger cannot make the relay read or hold more.
//!   3. The caller-address header is written by the relay before any
//!      byte of the caller's, and parses only in its exact form.
//! SCENARIOS:
//!   1. A proof made for one session fails on another session's exporter,
//!      on another nonce, with another name, and with the claim flag
//!      flipped.
//!   2. A PROXY header for IPv4 and for IPv6 round-trips; a header with
//!      trailing bytes, a wrong signature, or a wrong length is refused.
//! ```

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use ring::rand::SecureRandom;
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::names::Name;

/// The protocol version in every handshake message.
pub const VERSION: u8 = 1;
/// The most a handshake or control frame may carry.
pub const MAX_FRAME: usize = 2048;
/// The TLS exporter label the proof binds to.
pub const EXPORTER_LABEL: &[u8] = b"EXPORTER-wanderbooks-relay-line-v1";
const PROOF_CONTEXT: &[u8] = b"wanderbooks-relay/line/v1\0";

/// The relay's opening message on a new line.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Challenge {
    /// Protocol version.
    pub v: u8,
    /// 32 fresh random bytes, hex.
    pub nonce: String,
}

/// The app's answer: who it is, and the proof.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    /// Protocol version.
    pub v: u8,
    /// The four-word name.
    pub name: String,
    /// The Ed25519 public key (32 bytes), hex.
    pub public_key: String,
    /// True to claim the name if nobody has it. False to reconnect.
    pub claim: bool,
    /// The signature (64 bytes), hex, over [`proof_message`].
    pub signature: String,
}

/// Why the relay turned a line away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    /// The name is already held by another key.
    Taken,
    /// Reserved. A reconnect for a name nobody has claimed is answered
    /// [`Refusal::BadProof`], the same as a wrong key and a block, so a
    /// caller without the key learns nothing about which names exist.
    UnknownName,
    /// The proof did not verify, or the key is not the name's, or there is
    /// no such name, or the name is blocked and this is not its key: one
    /// answer for all of them.
    BadProof,
    /// The name is blocked by the operator, said only to the holder of its
    /// key.
    Blocked,
    /// The relay is busy; try again later.
    Busy,
    /// Too many new names, or too many tries, from here or in total.
    RateLimited,
    /// The message was malformed or of another version.
    BadRequest,
}

/// The relay's verdict on a [`Hello`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verdict {
    /// True: the line is up.
    pub ok: bool,
    /// Why not, when not.
    pub refusal: Option<Refusal>,
    /// True when this connection claimed the name for the first time.
    pub claimed: bool,
    /// The most caller streams the relay will open on this line at once.
    pub max_streams: u32,
}

/// What an app asks of the relay on a control stream. The name is never
/// in a request: it is the line's, proven at the handshake.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    /// A keep-alive; the reply shows the relay is there.
    Ping,
    /// Publish this ACME DNS-01 proof value at `_acme-challenge.<name>`.
    PublishTxt {
        /// The 43-character base64url value.
        value: String,
    },
    /// Remove this app's proof records.
    ClearTxt,
    /// The certificate authorities to try, in order.
    Authorities,
}

/// A certificate authority the app may try, and how to reach it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Authority {
    /// A short stable id (`letsencrypt`).
    pub id: String,
    /// The ACME directory address (RFC 8555).
    pub directory: String,
    /// True when the authority requires an external account binding.
    pub needs_binding: bool,
    /// True when the app can use this authority now: it needs no binding,
    /// or the relay has one for this app. An authority that needs a
    /// binding the relay has none for is listed (so the app can say what
    /// it is waiting for) and is not usable yet.
    #[serde(default = "yes")]
    pub usable: bool,
    /// The binding credential for this app, when one is needed and the
    /// relay has one for it.
    pub binding: Option<Binding>,
}

fn yes() -> bool {
    true
}

/// An external account binding credential (RFC 8555 section 7.3.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    /// The key identifier.
    pub kid: String,
    /// The HMAC key, base64url.
    pub hmac_key: String,
}

/// The reply to a [`Request`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    /// True when the request was done.
    pub ok: bool,
    /// A short reason when not.
    pub error: Option<String>,
    /// The list, for [`Request::Authorities`].
    pub authorities: Option<Vec<Authority>>,
}

/// A frame could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    /// The peer closed, or the stream failed.
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
    /// The frame is longer than [`MAX_FRAME`].
    #[error("frame too long")]
    TooLong,
    /// The frame is not the message it should be.
    #[error("bad message: {0}")]
    Bad(#[from] serde_json::Error),
}

/// Read one frame: a two-byte length (at most [`MAX_FRAME`]), then that
/// many bytes of JSON.
pub async fn read_frame<R, T>(r: &mut R) -> Result<T, FrameError>
where
    R: AsyncRead + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let mut len = [0u8; 2];
    r.read_exact(&mut len).await?;
    let len = usize::from(u16::from_be_bytes(len));
    if len > MAX_FRAME {
        return Err(FrameError::TooLong);
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    Ok(serde_json::from_slice(&buf)?)
}

/// Write one frame.
pub async fn write_frame<W, T>(w: &mut W, msg: &T) -> Result<(), FrameError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let body = serde_json::to_vec(msg)?;
    if body.len() > MAX_FRAME {
        return Err(FrameError::TooLong);
    }
    let mut out = Vec::with_capacity(2 + body.len());
    out.extend(
        u16::try_from(body.len())
            .map_err(|_| FrameError::TooLong)?
            .to_be_bytes(),
    );
    out.extend(body);
    w.write_all(&out).await?;
    w.flush().await?;
    Ok(())
}

/// Lower-case hex.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 15)]));
    }
    out
}

/// Decode exactly `N` bytes of hex, or `None`.
pub fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let bytes = text.as_bytes();
    if bytes.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    let (pairs, _) = bytes.as_chunks::<2>();
    for (slot, [h, l]) in out.iter_mut().zip(pairs) {
        let hi = char::from(*h).to_digit(16)?;
        let lo = char::from(*l).to_digit(16)?;
        *slot = u8::try_from(hi * 16 + lo).ok()?;
    }
    Some(out)
}

/// 32 random bytes.
pub fn random32() -> [u8; 32] {
    let mut b = [0u8; 32];
    if ring::rand::SystemRandom::new().fill(&mut b).is_err() {
        // The system generator failing is not recoverable.
        panic!("the system random number generator failed");
    }
    b
}

/// What the app signs: the context, this TLS session's exporter value,
/// the relay's nonce, the name and the claim flag.
pub fn proof_message(exporter: &[u8; 32], nonce: &[u8; 32], name: &Name, claim: bool) -> Vec<u8> {
    let text = name.to_string();
    let mut m = Vec::with_capacity(PROOF_CONTEXT.len() + 32 + 32 + 2 + text.len());
    m.extend(PROOF_CONTEXT);
    m.extend(exporter);
    m.extend(nonce);
    m.push(u8::from(claim));
    // A name is under 255 bytes by construction (four words of at most 11).
    m.push(u8::try_from(text.len()).unwrap_or(u8::MAX));
    m.extend(text.as_bytes());
    m
}

/// Verify an Ed25519 signature.
pub fn verify(public_key: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> bool {
    UnparsedPublicKey::new(&ED25519, public_key)
        .verify(message, signature)
        .is_ok()
}

/// An app's own key: it owns the name.
pub struct AppKey(Ed25519KeyPair);

impl AppKey {
    /// A new key, and its PKCS#8 encoding to keep.
    pub fn generate() -> (Self, Vec<u8>) {
        let rng = ring::rand::SystemRandom::new();
        let Ok(doc) = Ed25519KeyPair::generate_pkcs8(&rng) else {
            // The system generator failing is not recoverable.
            panic!("the system random number generator failed");
        };
        let bytes = doc.as_ref().to_vec();
        let key = Self::from_pkcs8(&bytes).unwrap_or_else(|| unreachable!("a fresh key parses"));
        (key, bytes)
    }

    /// A key from its PKCS#8 encoding.
    pub fn from_pkcs8(bytes: &[u8]) -> Option<Self> {
        Ed25519KeyPair::from_pkcs8(bytes).ok().map(AppKey)
    }

    /// The public key, 32 bytes.
    pub fn public_key(&self) -> [u8; 32] {
        let mut out = [0u8; 32];
        out.copy_from_slice(self.0.public_key().as_ref());
        out
    }

    /// Sign.
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        let mut out = [0u8; 64];
        out.copy_from_slice(self.0.sign(message).as_ref());
        out
    }
}

const PP2_SIGNATURE: [u8; 12] = *b"\r\n\r\n\0\r\nQUIT\n";

/// The caller's address and the one it reached, as the relay saw them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caller {
    /// Who connected to the relay.
    pub source: SocketAddr,
    /// The relay address they connected to.
    pub destination: SocketAddr,
}

/// An IPv4-mapped IPv6 address is the IPv4 address.
fn unmap(addr: SocketAddr) -> SocketAddr {
    match addr {
        SocketAddr::V6(v6) => match v6.ip().to_ipv4_mapped() {
            Some(v4) => SocketAddr::new(IpAddr::V4(v4), v6.port()),
            None => addr,
        },
        v4 => v4,
    }
}

impl Caller {
    /// The PROXY protocol version 2 header for this caller (28 bytes for
    /// IPv4, 52 for IPv6), with no extensions.
    pub fn encode(&self) -> Vec<u8> {
        let (src, dst) = (unmap(self.source), unmap(self.destination));
        let mut out = Vec::with_capacity(52);
        out.extend(PP2_SIGNATURE);
        out.push(0x21); // version 2, PROXY
        match (src, dst) {
            (SocketAddr::V4(s), SocketAddr::V4(d)) => {
                out.push(0x11); // TCP over IPv4
                out.extend(12u16.to_be_bytes());
                out.extend(s.ip().octets());
                out.extend(d.ip().octets());
                out.extend(s.port().to_be_bytes());
                out.extend(d.port().to_be_bytes());
            }
            (s, d) => {
                let (s6, d6) = (to_v6(s), to_v6(d));
                out.push(0x21); // TCP over IPv6
                out.extend(36u16.to_be_bytes());
                out.extend(s6.0.octets());
                out.extend(d6.0.octets());
                out.extend(s6.1.to_be_bytes());
                out.extend(d6.1.to_be_bytes());
            }
        }
        out
    }

    /// Read a header from the front of a stream. Exactly the forms
    /// [`Caller::encode`] writes are accepted.
    pub async fn read<R: AsyncRead + Unpin>(r: &mut R) -> Result<Caller, FrameError> {
        let mut head = [0u8; 16];
        r.read_exact(&mut head).await?;
        let bad = || FrameError::Io(std::io::ErrorKind::InvalidData.into());
        if head[..12] != PP2_SIGNATURE || head[12] != 0x21 {
            return Err(bad());
        }
        let len = usize::from(u16::from_be_bytes([head[14], head[15]]));
        match (head[13], len) {
            (0x11, 12) => {
                let mut b = [0u8; 12];
                r.read_exact(&mut b).await?;
                Ok(Caller {
                    source: SocketAddr::new(
                        IpAddr::V4(Ipv4Addr::new(b[0], b[1], b[2], b[3])),
                        u16::from_be_bytes([b[8], b[9]]),
                    ),
                    destination: SocketAddr::new(
                        IpAddr::V4(Ipv4Addr::new(b[4], b[5], b[6], b[7])),
                        u16::from_be_bytes([b[10], b[11]]),
                    ),
                })
            }
            (0x21, 36) => {
                let mut b = [0u8; 36];
                r.read_exact(&mut b).await?;
                let ip = |at: usize| {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b[at..at + 16]);
                    IpAddr::V6(Ipv6Addr::from(o))
                };
                Ok(Caller {
                    source: SocketAddr::new(ip(0), u16::from_be_bytes([b[32], b[33]])),
                    destination: SocketAddr::new(ip(16), u16::from_be_bytes([b[34], b[35]])),
                })
            }
            _ => Err(bad()),
        }
    }
}

fn to_v6(addr: SocketAddr) -> (Ipv6Addr, u16) {
    match addr {
        SocketAddr::V4(v4) => (v4.ip().to_ipv6_mapped(), v4.port()),
        SocketAddr::V6(v6) => (*v6.ip(), v6.port()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_and_refuses_the_wrong_length() {
        let b = random32();
        assert_eq!(unhex::<32>(&hex(&b)), Some(b));
        assert_eq!(unhex::<32>("00"), None);
        assert_eq!(unhex::<2>("zz00"), None);
        assert_eq!(unhex::<2>("+f00"), None);
    }

    #[test]
    fn a_proof_holds_for_one_session_and_nothing_else() {
        let (key, _) = AppKey::generate();
        let name = Name::parse("ledger-accrual-tally-mint").unwrap();
        let other = Name::parse("ledger-accrual-tally-vault").unwrap();
        let (exporter, nonce) = (random32(), random32());
        let sig = key.sign(&proof_message(&exporter, &nonce, &name, true));
        let pk = key.public_key();
        assert!(verify(
            &pk,
            &proof_message(&exporter, &nonce, &name, true),
            &sig
        ));
        assert!(!verify(
            &pk,
            &proof_message(&random32(), &nonce, &name, true),
            &sig
        ));
        assert!(!verify(
            &pk,
            &proof_message(&exporter, &random32(), &name, true),
            &sig
        ));
        assert!(!verify(
            &pk,
            &proof_message(&exporter, &nonce, &other, true),
            &sig
        ));
        assert!(!verify(
            &pk,
            &proof_message(&exporter, &nonce, &name, false),
            &sig
        ));
        let (stranger, _) = AppKey::generate();
        assert!(!verify(
            &stranger.public_key(),
            &proof_message(&exporter, &nonce, &name, true),
            &sig
        ));
    }

    #[tokio::test]
    async fn a_caller_header_round_trips_for_v4_and_v6_and_nothing_else_parses() {
        let v4 = Caller {
            source: "203.0.113.9:51000".parse().unwrap(),
            destination: "198.51.100.1:443".parse().unwrap(),
        };
        let v6 = Caller {
            source: "[2001:db8::7]:51000".parse().unwrap(),
            destination: "[2001:db8::1]:443".parse().unwrap(),
        };
        assert_eq!(v4.encode().len(), 28);
        assert_eq!(v6.encode().len(), 52);
        for c in [v4, v6] {
            let bytes = c.encode();
            assert_eq!(Caller::read(&mut &bytes[..]).await.unwrap(), c);
        }
        // A mapped IPv4 caller is written as IPv4.
        let mapped = Caller {
            source: "[::ffff:203.0.113.9]:51000".parse().unwrap(),
            destination: "198.51.100.1:443".parse().unwrap(),
        };
        assert_eq!(mapped.encode(), v4.encode());
        let good = v4.encode();
        for at in 0..good.len() {
            let mut cut = good.clone();
            cut.truncate(at);
            assert!(Caller::read(&mut &cut[..]).await.is_err(), "cut at {at}");
        }
        let mut wrong = good.clone();
        wrong[0] = b'G';
        assert!(Caller::read(&mut &wrong[..]).await.is_err());
        let mut wrong_len = good.clone();
        wrong_len[15] = 13;
        assert!(Caller::read(&mut &wrong_len[..]).await.is_err());
        assert!(
            Caller::read(&mut &b"GET / HTTP/1.1\r\n\r\n"[..])
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn frames_are_bounded() {
        let mut buf = Vec::new();
        write_frame(&mut buf, &Request::Ping).await.unwrap();
        let back: Request = read_frame(&mut &buf[..]).await.unwrap();
        assert!(matches!(back, Request::Ping));
        let long = [0xffu8, 0xff];
        assert!(matches!(
            read_frame::<_, Request>(&mut &long[..]).await,
            Err(FrameError::TooLong)
        ));
        let junk = [0u8, 3, b'n', b'o', b'!'];
        assert!(matches!(
            read_frame::<_, Request>(&mut &junk[..]).await,
            Err(FrameError::Bad(_))
        ));
        let unknown = br#"{"op":"publish_txt","value":"x","extra":1}"#;
        let mut framed = u16::try_from(unknown.len()).unwrap().to_be_bytes().to_vec();
        framed.extend(unknown);
        assert!(read_frame::<_, Request>(&mut &framed[..]).await.is_err());
    }
}
