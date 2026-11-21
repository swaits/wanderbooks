//! The caller's address, from the PROXY protocol version 2 header the
//! relay writes before a caller's first byte (crates/relay `proto.rs`,
//! WB-relay-6), and the listener that reads it.
//!
//! ```text
//! GOAL: the remote door knows the real address of every caller, from the
//!       one place no caller can write: a header that comes before the
//!       caller's own bytes, from the app that forwards them.
//! SOURCE: WB-remote-1 (the remote door), WB-relay-6 (the caller's
//!         address); the PROXY protocol specification version 2;
//!         plan/p15-app-and-relay.md 4, rule 8.
//! INVARIANTS:
//!   5. Every connection starts with this start's secret, then the header:
//!      the first SECRET_LEN bytes are the 64 lowercase hex characters the
//!      server gave the app over the admin socket when the door started
//!      (`proxy_secret` of the remote status), compared in constant time. A
//!      connection that begins with anything else is dropped before a byte
//!      of anything is read, so only the app's relay line can state a
//!      caller's address: another process on this computer that finds the
//!      port cannot say a caller is anyone, nor use up what an address is
//!      limited to by saying it is another.
//!   1. Every connection to the remote door, after the secret, starts with exactly the header
//!      the relay writes: the 12-byte signature, version 2 with the PROXY
//!      command, then TCP over IPv4 (length 12) or IPv6 (length 36) and
//!      the addresses. A connection that starts with anything else, that
//!      is cut short, or that is silent for HEADER_WAIT, is dropped before
//!      a byte of HTTP is read.
//!   2. The header is read once, at the start. A second header, or one
//!      sent by the caller inside its own bytes, is HTTP text to the
//!      server and is refused as such: the address of a connection can
//!      never change after its first bytes.
//!   3. No HTTP header ever names a caller's address for the server:
//!      `X-Forwarded-For` and its kind are not read anywhere.
//!   4. The number of connections is bounded: a connection past
//!      MAX_CONNECTIONS is closed at once, and a connection that waits for
//!      its header holds one of them.
//! SCENARIOS:
//!   1. The relay's own header for IPv4 and for IPv6 reads back as its
//!      source address; an IPv4-mapped IPv6 source is the IPv4 address.
//!   2. A header with another signature, version, command or length, one
//!      cut short, and plain HTTP with no header, are each refused.
//! ```

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// The 12 bytes every PROXY protocol version 2 header begins with.
pub const SIGNATURE: [u8; 12] = [
    0x0D, 0x0A, 0x0D, 0x0A, 0x00, 0x0D, 0x0A, 0x51, 0x55, 0x49, 0x54, 0x0A,
];

/// The length of the secret every connection begins with: 64 lowercase
/// hexadecimal characters, the text of 32 random bytes.
pub const SECRET_LEN: usize = 64;

/// A secret for one start of the door: 32 random bytes as 64 lowercase hex
/// characters.
pub fn new_secret() -> Result<String, String> {
    let mut bytes = [0u8; SECRET_LEN / 2];
    getrandom::getrandom(&mut bytes).map_err(|_| "the system gave no randomness".to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// How long a connection has to send its header.
pub const HEADER_WAIT: Duration = Duration::from_secs(5);

/// The most connections the remote door holds at once (INVARIANT 4).
pub const MAX_CONNECTIONS: usize = 512;

/// Why a header is refused.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not the relay's exact form.
    NotAHeader,
    /// The stream ended, or failed, inside it.
    Cut,
    /// It did not begin with this start's secret.
    BadSecret,
}

/// Read this start's secret from the front of `stream` and compare it in
/// constant time (INVARIANT 5).
pub async fn read_secret<R: AsyncRead + Unpin>(
    stream: &mut R,
    secret: &str,
) -> Result<(), Refused> {
    use subtle::ConstantTimeEq;
    let mut given = [0u8; SECRET_LEN];
    stream
        .read_exact(&mut given)
        .await
        .map_err(|_| Refused::Cut)?;
    let same: bool = given.as_slice().ct_eq(secret.as_bytes()).into();
    if secret.len() == SECRET_LEN && same {
        Ok(())
    } else {
        Err(Refused::BadSecret)
    }
}

/// Read the header from the front of `stream`, giving the caller's
/// address (the header's source). Exactly the forms the relay writes are
/// accepted (INVARIANT 1).
pub async fn read_header<R: AsyncRead + Unpin>(stream: &mut R) -> Result<SocketAddr, Refused> {
    let mut head = [0u8; 16];
    stream
        .read_exact(&mut head)
        .await
        .map_err(|_| Refused::Cut)?;
    if head[..12] != SIGNATURE || head[12] != 0x21 {
        return Err(Refused::NotAHeader);
    }
    let len = usize::from(u16::from_be_bytes([head[14], head[15]]));
    match (head[13], len) {
        (0x11, 12) => {
            let mut b = [0u8; 12];
            stream.read_exact(&mut b).await.map_err(|_| Refused::Cut)?;
            Ok(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(b[0], b[1], b[2], b[3])),
                u16::from_be_bytes([b[8], b[9]]),
            ))
        }
        (0x21, 36) => {
            let mut b = [0u8; 36];
            stream.read_exact(&mut b).await.map_err(|_| Refused::Cut)?;
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&b[..16]);
            let ip = Ipv6Addr::from(octets);
            let ip = ip.to_ipv4_mapped().map_or(IpAddr::V6(ip), IpAddr::V4);
            Ok(SocketAddr::new(ip, u16::from_be_bytes([b[32], b[33]])))
        }
        _ => Err(Refused::NotAHeader),
    }
}

/// The header for a caller `source` that reached `destination`, in the
/// form the relay writes (what the app writes to this door, B3, after the
/// secret; and the tests).
#[allow(dead_code)] // the app writes it to this door (unit B3); the tests do now
pub fn encode(source: SocketAddr, destination: SocketAddr) -> Vec<u8> {
    let mut out = Vec::with_capacity(52);
    out.extend(SIGNATURE);
    out.push(0x21);
    match (source, destination) {
        (SocketAddr::V4(s), SocketAddr::V4(d)) => {
            out.push(0x11);
            out.extend(12u16.to_be_bytes());
            out.extend(s.ip().octets());
            out.extend(d.ip().octets());
            out.extend(s.port().to_be_bytes());
            out.extend(d.port().to_be_bytes());
        }
        (s, d) => {
            let v6 = |a: SocketAddr| match a {
                SocketAddr::V6(v) => (v.ip().octets(), v.port()),
                SocketAddr::V4(v) => (v.ip().to_ipv6_mapped().octets(), v.port()),
            };
            let ((si, sp), (di, dp)) = (v6(s), v6(d));
            out.push(0x21);
            out.extend(36u16.to_be_bytes());
            out.extend(si);
            out.extend(di);
            out.extend(sp.to_be_bytes());
            out.extend(dp.to_be_bytes());
        }
    }
    out
}

/// A connection whose header was read: its stream, and who is on the
/// other end. It holds one of the door's connection places until dropped
/// (INVARIANT 4).
pub struct Proxied {
    io: TcpStream,
    _place: OwnedSemaphorePermit,
}

impl AsyncRead for Proxied {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_read(cx, buf)
    }
}

impl AsyncWrite for Proxied {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.io).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }
}

/// Accept connections on `listener` until `stop`, read each one's header
/// (INVARIANT 1), and hand on the ones that have one, with the address
/// they name. Never waits on one connection to accept another.
pub async fn accept_loop(
    listener: TcpListener,
    stop: tokio_util::sync::CancellationToken,
    secret: String,
    mut handle: impl FnMut(Proxied, SocketAddr) + Send + 'static,
) {
    let places = std::sync::Arc::new(Semaphore::new(MAX_CONNECTIONS));
    let (tx, mut rx) = tokio::sync::mpsc::channel::<(Proxied, SocketAddr)>(64);
    let accepting = {
        let stop = stop.clone();
        async move {
            loop {
                let accepted = tokio::select! {
                    () = stop.cancelled() => break,
                    accepted = listener.accept() => accepted,
                };
                let Ok((mut io, _local_peer)) = accepted else {
                    // A failed accept (out of descriptors, a reset): try again
                    // after a breath, never spin.
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                };
                // Closed at once past the limit, before anything is read.
                let Ok(place) = places.clone().try_acquire_owned() else {
                    drop(io);
                    continue;
                };
                let tx = tx.clone();
                let secret = secret.clone();
                tokio::spawn(async move {
                    let read = tokio::time::timeout(HEADER_WAIT, async {
                        read_secret(&mut io, &secret).await?;
                        read_header(&mut io).await
                    })
                    .await;
                    if let Ok(Ok(caller)) = read {
                        let _ = tx.send((Proxied { io, _place: place }, caller)).await;
                    }
                });
            }
        }
    };
    tokio::pin!(accepting);
    loop {
        tokio::select! {
            () = &mut accepting => break,
            got = rx.recv() => match got {
                Some((io, caller)) => handle(io, caller),
                None => break,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(text: &str) -> SocketAddr {
        text.parse().unwrap()
    }

    #[tokio::test]
    async fn the_relays_header_reads_back_its_source() {
        for (src, dst) in [
            ("203.0.113.9:51000", "198.51.100.1:443"),
            ("[2001:db8::7]:4000", "[2001:db8::1]:443"),
        ] {
            let bytes = encode(addr(src), addr(dst));
            let mut slice = &bytes[..];
            assert_eq!(read_header(&mut slice).await, Ok(addr(src)));
            assert!(slice.is_empty());
        }
        // An IPv4 caller on a dual-stack relay arrives as a mapped address.
        let bytes = encode(addr("[::ffff:203.0.113.9]:5"), addr("[2001:db8::1]:443"));
        let mut slice = &bytes[..];
        assert_eq!(read_header(&mut slice).await, Ok(addr("203.0.113.9:5")));
    }

    #[tokio::test]
    async fn a_connection_must_begin_with_this_starts_secret() {
        let secret = new_secret().unwrap();
        assert_eq!(secret.len(), SECRET_LEN);
        assert!(
            secret
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        assert_ne!(secret, new_secret().unwrap());
        let mut good = secret.clone().into_bytes();
        good.extend(b"rest");
        let mut slice = &good[..];
        assert_eq!(read_secret(&mut slice, &secret).await, Ok(()));
        assert_eq!(slice, b"rest");
        // Another secret, one byte off, no secret, and a short one.
        let mut off = secret.clone().into_bytes();
        off[63] = if off[63] == b'0' { b'1' } else { b'0' };
        for (what, bytes) in [("off by one", off), ("other text", vec![b'x'; SECRET_LEN])] {
            let mut slice = &bytes[..];
            assert_eq!(
                read_secret(&mut slice, &secret).await,
                Err(Refused::BadSecret),
                "{what}"
            );
        }
        let mut short = &b"abc"[..];
        assert_eq!(read_secret(&mut short, &secret).await, Err(Refused::Cut));
    }

    #[tokio::test]
    async fn anything_but_the_exact_form_is_refused() {
        let good = encode(addr("203.0.113.9:51000"), addr("198.51.100.1:443"));
        let mut cases: Vec<(&str, Vec<u8>, Refused)> = Vec::new();
        let mut v = good.clone();
        v[0] ^= 1;
        cases.push(("signature", v, Refused::NotAHeader));
        let mut v = good.clone();
        v[12] = 0x20; // the LOCAL command: no address
        cases.push(("local command", v, Refused::NotAHeader));
        let mut v = good.clone();
        v[12] = 0x11; // version 1
        cases.push(("version", v, Refused::NotAHeader));
        let mut v = good.clone();
        v[13] = 0x12; // UDP
        cases.push(("udp", v, Refused::NotAHeader));
        let mut v = good.clone();
        v[15] = 13; // a length the form never has
        cases.push(("length", v, Refused::NotAHeader));
        cases.push(("cut", good[..20].to_vec(), Refused::Cut));
        cases.push((
            "plain http",
            b"GET /b/x/mcp HTTP/1.1\r\nHost: h\r\n\r\n".to_vec(),
            Refused::NotAHeader,
        ));
        for (what, bytes, why) in cases {
            let mut slice = &bytes[..];
            assert_eq!(read_header(&mut slice).await, Err(why), "{what}");
        }
    }
}
