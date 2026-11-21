//! Reading the server name from a TLS ClientHello, without terminating TLS.
//!
//! The first record of a TLS connection is unencrypted. The router reads
//! it, takes the server name, and forwards the bytes untouched. The
//! parser works on a byte slice, never allocates beyond the name it
//! returns, and treats every length as hostile.
//!
//! ```text
//! GOAL: find the server name in the first TLS record, or say it is not
//!       there, on any bytes a stranger can send, without panicking and
//!       without reading past what was given.
//! SOURCE: WB-relay-1 (the router), RFC 8446 section 4.1.2 and RFC 6066
//!         section 3 (the record and the server_name extension).
//! INVARIANTS:
//!   1. `record_len` and `server_name` never panic and never read beyond
//!      the slice, whatever the bytes.
//!   2. A name is returned only from a well-formed server_name extension
//!      of a complete ClientHello that fits one record, and only when it
//!      is a plausible host name (ASCII letters, digits, `-`, `.`, `_`,
//!      at most 253 bytes, no empty label).
//!   3. The record length is read from five bytes and is at most 16 384,
//!      so a caller never holds more than 16 389 bytes for the hello.
//! SCENARIOS:
//!   1. A real rustls ClientHello for `a.example` gives `a.example`.
//!   2. A hello with no server_name, a truncated hello, an SSLv2 hello
//!      and random bytes give an error, not a name.
//! ```

/// The most a TLS record may carry (RFC 8446 section 5.1).
pub const MAX_RECORD: usize = 16 * 1024;
/// A record's header: type, version, length.
pub const RECORD_HEADER: usize = 5;

/// Why the first bytes are not a hello with a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HelloError {
    /// Not a TLS handshake record, or one longer than TLS allows.
    #[error("not a TLS handshake record")]
    NotTls,
    /// The record is well formed but has no usable server name.
    #[error("no server name")]
    NoName,
    /// The bytes are not a well-formed ClientHello.
    #[error("malformed ClientHello")]
    Malformed,
}

/// From the first five bytes: how many more bytes the record has.
///
/// Refuses anything that is not a handshake record of a TLS version
/// 1.0 to 1.3, or longer than a TLS record may be.
pub fn record_len(header: &[u8]) -> Result<usize, HelloError> {
    let [kind, major, minor, hi, lo] = *header
        .first_chunk::<RECORD_HEADER>()
        .ok_or(HelloError::NotTls)?;
    if kind != 0x16 || major != 3 || minor > 4 {
        return Err(HelloError::NotTls);
    }
    let len = usize::from(u16::from_be_bytes([hi, lo]));
    if len == 0 || len > MAX_RECORD {
        return Err(HelloError::NotTls);
    }
    Ok(len)
}

/// A cursor over a slice that refuses to read past its end.
struct Cur<'a>(&'a [u8]);

impl<'a> Cur<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], HelloError> {
        let (head, rest) = self.0.split_at_checked(n).ok_or(HelloError::Malformed)?;
        self.0 = rest;
        Ok(head)
    }
    fn u8(&mut self) -> Result<u8, HelloError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<usize, HelloError> {
        let b = self.take(2)?;
        Ok(usize::from(u16::from_be_bytes([b[0], b[1]])))
    }
    fn u24(&mut self) -> Result<usize, HelloError> {
        let b = self.take(3)?;
        Ok((usize::from(b[0]) << 16) | (usize::from(b[1]) << 8) | usize::from(b[2]))
    }
    fn vec8(&mut self) -> Result<&'a [u8], HelloError> {
        let n = usize::from(self.u8()?);
        self.take(n)
    }
    fn vec16(&mut self) -> Result<&'a [u8], HelloError> {
        let n = self.u16()?;
        self.take(n)
    }
}

/// The server name in a complete first record (header and body), lower
/// case and without a trailing dot.
///
/// The record must hold the whole ClientHello: a hello split over several
/// records is refused (`Malformed`), as no client sends one in practice
/// and holding several records would let a stranger make us hold more.
pub fn server_name(record: &[u8]) -> Result<String, HelloError> {
    let len = record_len(record)?;
    let body = record
        .get(RECORD_HEADER..RECORD_HEADER + len)
        .ok_or(HelloError::Malformed)?;
    let mut hs = Cur(body);
    if hs.u8()? != 0x01 {
        return Err(HelloError::Malformed); // not a ClientHello
    }
    let hello_len = hs.u24()?;
    let mut hello = Cur(hs.take(hello_len)?);
    hello.take(2)?; // client_version
    hello.take(32)?; // random
    hello.vec8()?; // session_id
    hello.vec16()?; // cipher_suites
    hello.vec8()?; // compression_methods
    if hello.0.is_empty() {
        return Err(HelloError::NoName); // a hello with no extensions
    }
    let mut exts = Cur(hello.vec16()?);
    let mut found: Option<String> = None;
    while !exts.0.is_empty() {
        let kind = exts.u16()?;
        let data = exts.vec16()?;
        if kind == 0 {
            if found.is_some() {
                return Err(HelloError::Malformed); // two server_name extensions
            }
            found = Some(parse_server_name_ext(data)?);
        }
    }
    found.ok_or(HelloError::NoName)
}

fn parse_server_name_ext(data: &[u8]) -> Result<String, HelloError> {
    let mut ext = Cur(data);
    let mut list = Cur(ext.vec16()?);
    if !ext.0.is_empty() {
        return Err(HelloError::Malformed);
    }
    let mut host: Option<&[u8]> = None;
    while !list.0.is_empty() {
        let name_type = list.u8()?;
        let name = list.vec16()?;
        if name_type == 0 {
            if host.is_some() {
                return Err(HelloError::Malformed);
            }
            host = Some(name);
        }
    }
    let host = host.ok_or(HelloError::NoName)?;
    plausible_host(host)
}

/// A host name the router can look at: ASCII, at most 253 bytes, labels
/// of 1 to 63 bytes from letters, digits, `-` and `_`; lowered, and the
/// trailing dot dropped. An IP address is a host name that will not match
/// any app, which is the right answer.
fn plausible_host(raw: &[u8]) -> Result<String, HelloError> {
    let text = std::str::from_utf8(raw).map_err(|_| HelloError::Malformed)?;
    let text = text.strip_suffix('.').unwrap_or(text);
    if text.is_empty() || text.len() > 253 {
        return Err(HelloError::NoName);
    }
    for label in text.split('.') {
        if label.is_empty()
            || label.len() > 63
            || !label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(HelloError::NoName);
        }
    }
    Ok(text.to_ascii_lowercase())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A ClientHello record built by hand, with the given extensions.
    pub(crate) fn hello_record(exts: &[(u16, Vec<u8>)]) -> Vec<u8> {
        let mut ext_bytes = Vec::new();
        for (kind, data) in exts {
            ext_bytes.extend(kind.to_be_bytes());
            ext_bytes.extend(u16::try_from(data.len()).unwrap().to_be_bytes());
            ext_bytes.extend(data);
        }
        let mut hello = vec![3, 3];
        hello.extend([7u8; 32]);
        hello.push(0); // session id
        hello.extend([0, 2, 0x13, 0x01]); // one cipher suite
        hello.extend([1, 0]); // null compression
        hello.extend(u16::try_from(ext_bytes.len()).unwrap().to_be_bytes());
        hello.extend(ext_bytes);
        let mut hs = vec![1];
        hs.extend(&u32::try_from(hello.len()).unwrap().to_be_bytes()[1..]);
        hs.extend(hello);
        let mut rec = vec![0x16, 3, 1];
        rec.extend(u16::try_from(hs.len()).unwrap().to_be_bytes());
        rec.extend(hs);
        rec
    }

    pub(crate) fn sni_ext(host: &str) -> (u16, Vec<u8>) {
        let mut entry = vec![0];
        entry.extend(u16::try_from(host.len()).unwrap().to_be_bytes());
        entry.extend(host.as_bytes());
        let mut data = u16::try_from(entry.len()).unwrap().to_be_bytes().to_vec();
        data.extend(entry);
        (0, data)
    }

    #[test]
    fn a_hand_built_hello_gives_its_name_lowered() {
        let rec = hello_record(&[(43, vec![2, 3, 4]), sni_ext("A.Example.")]);
        assert_eq!(server_name(&rec).as_deref(), Ok("a.example"));
    }

    #[test]
    fn no_name_no_extensions_and_odd_names_are_refused() {
        assert_eq!(server_name(&hello_record(&[])), Err(HelloError::NoName));
        assert_eq!(
            server_name(&hello_record(&[(43, vec![2, 3, 4])])),
            Err(HelloError::NoName)
        );
        for bad in ["", "a..b", "a b", "ex\u{e9}.com", &"a".repeat(64)] {
            assert!(
                server_name(&hello_record(&[sni_ext(bad)])).is_err(),
                "{bad:?}"
            );
        }
        let long = format!(
            "{}.{}.{}.{}",
            "a".repeat(63),
            "b".repeat(63),
            "c".repeat(63),
            "d".repeat(63)
        );
        assert!(server_name(&hello_record(&[sni_ext(&long)])).is_err());
        // Two server_name extensions are ambiguous: refused.
        assert_eq!(
            server_name(&hello_record(&[sni_ext("a.example"), sni_ext("b.example")])),
            Err(HelloError::Malformed)
        );
    }

    #[test]
    fn not_tls_and_cut_off_bytes_are_refused() {
        assert_eq!(record_len(b""), Err(HelloError::NotTls));
        assert_eq!(record_len(b"GET /"), Err(HelloError::NotTls));
        assert_eq!(record_len(&[0x16, 3, 1, 0, 0]), Err(HelloError::NotTls));
        assert_eq!(record_len(&[0x16, 3, 1, 0x40, 1]), Err(HelloError::NotTls));
        assert_eq!(record_len(&[0x16, 3, 1, 0x40, 0]), Ok(16384));
        // SSLv2-style hello.
        assert_eq!(record_len(&[0x80, 0x2e, 1, 0, 2]), Err(HelloError::NotTls));
        let rec = hello_record(&[sni_ext("a.example")]);
        for cut in 0..rec.len() {
            assert!(server_name(&rec[..cut]).is_err(), "cut at {cut}");
        }
        // A handshake that is not a ClientHello.
        let mut other = rec.clone();
        other[5] = 2;
        assert_eq!(server_name(&other), Err(HelloError::Malformed));
    }

    #[test]
    fn a_real_rustls_hello_gives_its_name() {
        use std::io::Write;
        let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
        let name = rustls::pki_types::ServerName::try_from("real.example").unwrap();
        let mut conn = rustls::ClientConnection::new(std::sync::Arc::new(config), name).unwrap();
        let mut out = Vec::new();
        conn.write_tls(&mut out).unwrap();
        out.flush().unwrap();
        assert_eq!(server_name(&out).as_deref(), Ok("real.example"));
        assert_eq!(record_len(&out[..5]).unwrap(), out.len() - 5);
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(4000))]

        /// Invariant 1: random bytes never panic.
        #[test]
        fn random_bytes_never_panic(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..600)) {
            let _ = record_len(&bytes);
            let _ = server_name(&bytes);
        }

        /// Invariant 1 again, on bytes that look like a hello: a valid
        /// one with a few bytes flipped, or cut anywhere.
        #[test]
        fn a_mutated_hello_never_panics(
            flips in proptest::collection::vec((0usize..400, proptest::prelude::any::<u8>()), 0..8),
            cut in 0usize..400,
        ) {
            let mut rec = hello_record(&[(43, vec![2, 3, 4]), sni_ext("a.example")]);
            for (at, to) in flips {
                if let Some(b) = rec.get_mut(at) { *b = to; }
            }
            rec.truncate(cut.max(1));
            if let Ok(name) = server_name(&rec) {
                // Whatever came out is a plausible host name.
                proptest::prop_assert!(!name.is_empty() && name.len() <= 253);
                proptest::prop_assert!(name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.' || b == b'_'));
            }
        }
    }
}
