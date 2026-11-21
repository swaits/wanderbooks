//! Client ID metadata documents: a client whose id is an https address,
//! at which it publishes who it is (the IETF draft "OAuth Client ID
//! Metadata Document"; plan P15 section 4, rule 2).
//!
//! ```text
//! GOAL: the door can learn what an unknown client says of itself from the
//!       address it gives as its id, without ever being made to reach into
//!       the owner's own network or to read more than a small document.
//! SOURCE: WB-remote-4 (three ways to introduce a client); the draft above
//!         (the document's `client_id` is its own address; public clients
//!         only); plan/p15-app-and-relay.md 4, rule 2.
//! INVARIANTS:
//!   1. Only an https address with a host and a path, no userinfo and no
//!      fragment, is fetched; the host's addresses are all looked up
//!      first, and every one must be a public address (none loopback,
//!      private, link-local, shared, documentation, multicast or
//!      unspecified, and no IPv4-mapped form of one): a name that points
//!      into the owner's network is refused, and the connection is made to
//!      the address that was checked, not to a second lookup.
//!   2. No redirect is followed; the answer must be 200, at most
//!      MAX_DOCUMENT bytes, within FETCH_TIME.
//!   3. The document's `client_id` must be exactly the address it was
//!      fetched from; its `redirect_uris` are 1 to MAX_REDIRECTS valid
//!      redirect addresses; its `token_endpoint_auth_method`, if given, is
//!      `none` (a document names no secret).
//! SCENARIOS:
//!   1. A document that names another client_id, or has no redirects, or
//!      names a secret method, is refused.
//!   2. The checks on an address: loopback, private, link-local, shared and
//!      mapped addresses are not public; a documentation-free global
//!      address is.
//! ```

use super::{MAX_REDIRECTS, clean, valid_redirect};
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::pin::Pin;
use std::time::Duration;

/// The largest document read.
pub const MAX_DOCUMENT: usize = 64 * 1024;
/// How long a fetch may take.
pub const FETCH_TIME: Duration = Duration::from_secs(5);

/// Where documents come from: the web in service, a table in the tests
/// (nothing in a test touches a network).
pub trait ClientDocs: Send + Sync {
    /// The bytes served at `url`, or why not.
    fn fetch<'a>(
        &'a self,
        url: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, String>> + Send + 'a>>;
}

/// The web, with the checks of INVARIANTS 1 and 2.
pub struct WebDocs;

fn v4_public(a: Ipv4Addr) -> bool {
    let [o0, o1, o2, _] = a.octets();
    !(a.is_unspecified()
        || a.is_loopback()
        || a.is_private()
        || a.is_link_local()
        || a.is_broadcast()
        || a.is_multicast()
        || o0 == 0
        // Shared address space (carrier-grade NAT), 100.64.0.0/10.
        || (o0 == 100 && (o1 & 0xc0) == 64)
        // IETF protocol assignments, 192.0.0.0/24; documentation ranges.
        || (o0 == 192 && o1 == 0 && (o2 == 0 || o2 == 2))
        || (o0 == 198 && o1 == 51 && o2 == 100)
        || (o0 == 203 && o1 == 0 && o2 == 113)
        // Benchmarking, 198.18.0.0/15; reserved, 240.0.0.0/4.
        || (o0 == 198 && (o1 & 0xfe) == 18)
        || o0 >= 240)
}

fn v6_public(a: Ipv6Addr) -> bool {
    if let Some(v4) = a.to_ipv4_mapped() {
        return v4_public(v4);
    }
    let s = a.segments();
    !(a.is_unspecified()
        || a.is_loopback()
        || a.is_multicast()
        // Unique local fc00::/7, link-local fe80::/10.
        || (s[0] & 0xfe00) == 0xfc00
        || (s[0] & 0xffc0) == 0xfe80
        // Documentation 2001:db8::/32, discard 100::/64.
        || (s[0] == 0x2001 && s[1] == 0x0db8)
        || (s[0] == 0x0100 && s[1] == 0 && s[2] == 0 && s[3] == 0)
        // IPv4-compatible and NAT64 forms of a private address.
        || (s[0] == 0x0064 && s[1] == 0xff9b))
}

/// Whether `ip` is a public address (INVARIANT 1).
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => v4_public(a),
        IpAddr::V6(a) => v6_public(a),
    }
}

impl ClientDocs for WebDocs {
    fn fetch<'a>(
        &'a self,
        url: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, String>> + Send + 'a>> {
        Box::pin(async move {
            let parsed = reqwest::Url::parse(url).map_err(|_| "not an address".to_string())?;
            let host = parsed
                .host_str()
                .filter(|_| {
                    parsed.scheme() == "https"
                        && parsed.username().is_empty()
                        && parsed.password().is_none()
                        && parsed.fragment().is_none()
                        && parsed.path() != "/"
                })
                .ok_or("the client_id must be an https address with a path")?
                .to_string();
            let port = parsed.port_or_known_default().unwrap_or(443);
            let addrs: Vec<std::net::SocketAddr> =
                tokio::time::timeout(FETCH_TIME, tokio::net::lookup_host((host.as_str(), port)))
                    .await
                    .map_err(|_| "the client's host didn't answer in time".to_string())?
                    .map_err(|_| "the client's host can't be found".to_string())?
                    .collect();
            if addrs.is_empty() || addrs.iter().any(|a| !is_public(a.ip())) {
                return Err("the client's host isn't a public address".into());
            }
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(FETCH_TIME)
                .https_only(true)
                // The connection goes to the addresses just checked.
                .resolve_to_addrs(&host, &addrs)
                .build()
                .map_err(|_| "no way to fetch the client's document".to_string())?;
            let mut response = client
                .get(url)
                .header("accept", "application/json")
                .send()
                .await
                .map_err(|_| "the client's document couldn't be fetched".to_string())?;
            if response.status() != reqwest::StatusCode::OK {
                return Err(format!(
                    "the client's document answered {}",
                    response.status().as_u16()
                ));
            }
            let mut body = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| "the client's document couldn't be read".to_string())?
            {
                body.extend_from_slice(&chunk);
                if body.len() > MAX_DOCUMENT {
                    return Err("the client's document is too large".into());
                }
            }
            Ok(body)
        })
    }
}

/// What a document says (INVARIANT 3): the client's name and redirects.
pub fn parse(url: &str, bytes: &[u8]) -> Result<(String, Vec<String>), String> {
    let doc: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|_| "the client's document isn't JSON".to_string())?;
    if doc.get("client_id").and_then(|v| v.as_str()) != Some(url) {
        return Err("the document's client_id isn't the address it came from".into());
    }
    if doc
        .get("token_endpoint_auth_method")
        .and_then(|v| v.as_str())
        .is_some_and(|m| m != "none")
    {
        return Err("a client document may not name a secret".into());
    }
    let uris = doc
        .get("redirect_uris")
        .and_then(|v| v.as_array())
        .filter(|a| !a.is_empty() && a.len() <= MAX_REDIRECTS)
        .ok_or("the document's redirect_uris are missing or too many")?;
    let mut redirects = Vec::new();
    for u in uris {
        let text = u.as_str().unwrap_or_default();
        if valid_redirect(text).is_none() {
            return Err("a redirect address in the document can't be used".into());
        }
        if !redirects.iter().any(|r| r == text) {
            redirects.push(text.to_string());
        }
    }
    let host = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default();
    let name = clean(
        doc.get("client_name")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        100,
    );
    Ok((if name.is_empty() { host } else { name }, redirects))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_public_addresses_are_fetched() {
        for bad in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "198.18.0.1",
            "203.0.113.7",
            "::1",
            "::",
            "fe80::1",
            "fd12::1",
            "ff02::1",
            "2001:db8::1",
            "::ffff:10.0.0.1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!is_public(bad.parse().unwrap()), "{bad}");
        }
        for good in [
            "8.8.8.8",
            "93.184.216.34",
            "2606:4700:4700::1111",
            "::ffff:8.8.8.8",
        ] {
            assert!(is_public(good.parse().unwrap()), "{good}");
        }
    }

    #[test]
    fn a_document_must_be_what_it_says_it_is() {
        let url = "https://client.example/meta.json";
        let good = br#"{"client_id":"https://client.example/meta.json","client_name":"Fine","redirect_uris":["https://client.example/cb"]}"#;
        assert_eq!(
            parse(url, good).unwrap(),
            (
                "Fine".to_string(),
                vec!["https://client.example/cb".to_string()]
            )
        );
        for bad in [
            br#"{"client_id":"https://other.example/meta.json","redirect_uris":["https://client.example/cb"]}"# as &[u8],
            br#"{"client_id":"https://client.example/meta.json","redirect_uris":[]}"#,
            br#"{"client_id":"https://client.example/meta.json"}"#,
            br#"{"client_id":"https://client.example/meta.json","redirect_uris":["javascript:alert(1)"]}"#,
            br#"{"client_id":"https://client.example/meta.json","redirect_uris":["https://c.example/cb#x"]}"#,
            br#"{"client_id":"https://client.example/meta.json","redirect_uris":["https://c.example/cb"],"token_endpoint_auth_method":"client_secret_post"}"#,
            b"not json",
        ] {
            assert!(parse(url, bad).is_err(), "{}", String::from_utf8_lossy(bad));
        }
    }
}
