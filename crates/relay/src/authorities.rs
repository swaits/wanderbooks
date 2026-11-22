//! The certificate authorities: a list the relay serves, not code.
//!
//! Every authority an app may use speaks the same protocol (ACME,
//! RFC 8555). The app's certificate code knows the protocol and nothing
//! about any one authority; at each issuance it asks the relay for the
//! list to try, in order. Swapping Let's Encrypt for Google Trust Services
//! or ZeroSSL, or adding one, is an edit to the list the relay reads, with
//! no release of the app.
//!
//! The list names each authority's directory address and the domain it
//! uses in a CAA record (its "issuer domain name", which its own
//! documentation gives). The relay's DNS answers CAA for the zone from the
//! same list, naming each listed authority and the DNS-01 method only, so
//! swapping an authority is an edit to this one file.
//!
//! ```toml
//! [[authority]]
//! id = "letsencrypt"
//! directory = "https://acme-v02.api.letsencrypt.org/directory"
//! caa_domain = "letsencrypt.org"
//!
//! [[authority]]
//! id = "zerossl"
//! directory = "https://acme.zerossl.com/v2/DV90"
//! caa_domain = "sectigo.com"
//! needs_binding = true
//! ```
//!
//! The file never holds a binding credential: an account-binding secret
//! (RFC 8555 section 7.3.4) is one an authority issues for one account,
//! and serving a shared one to every app would hand every holder of a name
//! the operator's account. A file with `binding_kid` or `binding_hmac_key`
//! is refused. A binding comes from a [`BindingSource`], per app; with none
//! installed an authority that needs one is still listed, marked not
//! usable, and served without one.
//!
//! ```text
//! GOAL: the authority an app asks is chosen by the relay's list at the
//!       moment of the request, in the list's order, so an operator can
//!       change authorities without a release of the app.
//! SOURCE: WB-relay-5 (the list of authorities), D-87, RFC 8555 section
//!         7.3.4 (external account binding).
//! INVARIANTS:
//!   1. The list is served in the order written, with each authority's
//!      directory address and whether it needs a binding.
//!   2. No binding secret is ever served from the file: a file that
//!      carries one is refused (a new file leaves the old list in force),
//!      and a binding is served only when a per-app [`BindingSource`]
//!      gives one; an authority that needs one and has none is served
//!      without a binding and marked not usable.
//!   3. A bad file never replaces a good list.
//!   4. The list is bounded: at most 16 authorities.
//!   5. The CAA issuer set is the list's `caa_domain`s, in order, and
//!      follows the file.
//! SCENARIOS:
//!   1. Two authorities, the first plain and the second needing a binding,
//!      are served in that order; the second has a binding only from a
//!      source, and is marked not usable without one.
//!   2. The file is edited to put the second first: the next read serves
//!      that order.
//!   3. The file is edited to garbage: the old list is still served.
//!   4. A file with a binding secret in it is refused in words.
//! ```

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde::Deserialize;

use crate::names::Name;
use crate::proto::{Authority, Binding};

const MAX_AUTHORITIES: usize = 16;
/// A list file larger than this is refused unread: sixteen entries need a
/// few kilobytes.
const MAX_FILE_BYTES: u64 = 64 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    authority: Vec<Entry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: String,
    directory: String,
    caa_domain: String,
    #[serde(default)]
    needs_binding: bool,
    // Never accepted: read only to be refused in words.
    binding_kid: Option<String>,
    binding_hmac_key: Option<String>,
}

/// Why a list was refused.
#[derive(Debug, thiserror::Error)]
pub enum ListError {
    /// The file is not valid TOML of this shape.
    #[error("authority list: {0}")]
    Toml(#[from] toml::de::Error),
    /// The file could not be read.
    #[error("authority list: {0}")]
    Io(#[from] std::io::Error),
    /// The list is empty, too long, or an entry is wrong.
    #[error("authority list: {0}")]
    Invalid(String),
}

/// Where an app's binding credential comes from: one per app, obtained
/// from the authority's own API by a deployment that implements this. The
/// list file never carries one.
pub trait BindingSource: Send + Sync {
    /// The credential for this app at this authority, if one is needed.
    fn binding_for(&self, authority_id: &str, name: &Name) -> Option<Binding>;
}

#[derive(Debug, Clone)]
struct Listed {
    id: String,
    directory: String,
    needs_binding: bool,
    caa_domain: String,
}

/// The list the relay serves.
pub struct AuthorityList {
    current: RwLock<Arc<Vec<Listed>>>,
    source: Option<PathBuf>,
    /// The text of the file as last read, good or bad: a change is a
    /// difference in content, never in a file time (see [`AuthorityList::reload`]).
    seen: std::sync::Mutex<Option<String>>,
    bindings: Option<Arc<dyn BindingSource>>,
}

fn is_loopback_http(url: &str) -> bool {
    ["http://127.0.0.1", "http://localhost", "http://[::1]"]
        .iter()
        .any(|p| url.starts_with(p))
}

/// A domain name fit for a CAA `issue` value: lower-case letters, digits
/// and hyphens in labels of 1 to 63, at least two labels, 253 at most. No
/// `;` or space, so no parameter can be smuggled into the record.
fn is_caa_domain(d: &str) -> bool {
    d.len() <= 253
        && d.split('.').count() >= 2
        && d.split('.').all(|l| {
            (1..=63).contains(&l.len())
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

fn parse(text: &str) -> Result<Vec<Listed>, ListError> {
    let file: File = toml::from_str(text)?;
    if file.authority.is_empty() || file.authority.len() > MAX_AUTHORITIES {
        return Err(ListError::Invalid(format!(
            "1 to {MAX_AUTHORITIES} authorities, not {}",
            file.authority.len()
        )));
    }
    let mut out: Vec<Listed> = Vec::new();
    for e in file.authority {
        if e.id.is_empty() || e.id.len() > 64 || out.iter().any(|o| o.id == e.id) {
            return Err(ListError::Invalid(format!("authority id {:?}", e.id)));
        }
        if e.directory.len() > 512
            || !(e.directory.starts_with("https://") || is_loopback_http(&e.directory))
        {
            return Err(ListError::Invalid(format!(
                "{}: the directory must be https",
                e.id
            )));
        }
        if e.binding_kid.is_some() || e.binding_hmac_key.is_some() {
            return Err(ListError::Invalid(format!(
                "{}: the list must not carry a binding secret (binding_kid, \
                 binding_hmac_key): a shared credential would be served to every app. \
                 Mark the authority needs_binding = true and give the relay a per-app \
                 binding source instead",
                e.id
            )));
        }
        if !is_caa_domain(&e.caa_domain) {
            return Err(ListError::Invalid(format!(
                "{}: caa_domain must be the authority's issuer domain name, \
                 such as letsencrypt.org",
                e.id
            )));
        }
        out.push(Listed {
            id: e.id,
            directory: e.directory,
            needs_binding: e.needs_binding,
            caa_domain: e.caa_domain,
        });
    }
    Ok(out)
}

fn read_file(path: &std::path::Path) -> Result<String, ListError> {
    if std::fs::metadata(path)?.len() > MAX_FILE_BYTES {
        return Err(ListError::Invalid(format!(
            "the file is larger than {MAX_FILE_BYTES} bytes"
        )));
    }
    Ok(std::fs::read_to_string(path)?)
}

impl AuthorityList {
    /// A list from TOML text.
    pub fn from_toml(text: &str) -> Result<Self, ListError> {
        Ok(AuthorityList {
            current: RwLock::new(Arc::new(parse(text)?)),
            source: None,
            seen: std::sync::Mutex::new(None),
            bindings: None,
        })
    }

    /// A list read from a file, and read again by [`AuthorityList::reload`].
    pub fn from_file(path: PathBuf) -> Result<Self, ListError> {
        let text = read_file(&path)?;
        let list = parse(&text)?;
        Ok(AuthorityList {
            current: RwLock::new(Arc::new(list)),
            source: Some(path),
            seen: std::sync::Mutex::new(Some(text)),
            bindings: None,
        })
    }

    /// An empty list: no authority is offered until one is configured.
    pub fn empty() -> Self {
        AuthorityList {
            current: RwLock::new(Arc::new(Vec::new())),
            source: None,
            seen: std::sync::Mutex::new(None),
            bindings: None,
        }
    }

    /// Take bindings from `source`, per app. Without one no authority that
    /// needs a binding is usable.
    pub fn with_binding_source(mut self, source: Arc<dyn BindingSource>) -> Self {
        self.bindings = Some(source);
        self
    }

    /// Read the file again and replace the list if its content changed. A
    /// bad file changes nothing. Returns whether the list was replaced.
    ///
    /// A change is found by comparing the text with the text last read,
    /// never by file time: a file time is as coarse as the operating
    /// system's clock tick (on Windows, about 15 ms), so a second edit
    /// within one tick keeps the same time and a time check would skip it.
    /// The file is small (at most 64 KiB) and the relay's housekeeping
    /// calls this every 30 seconds, so reading it each time costs nothing.
    pub fn reload(&self) -> Result<bool, ListError> {
        let Some(path) = &self.source else {
            return Ok(false);
        };
        let text = read_file(path)?;
        {
            let mut seen = self.seen.lock().unwrap_or_else(|p| p.into_inner());
            if seen.as_deref() == Some(text.as_str()) {
                return Ok(false);
            }
            // Remembered before it is parsed, so a file that stays bad is
            // refused once, not on every check; the next edit is read again.
            *seen = Some(text.clone());
        }
        let list = parse(&text)?;
        *self.current.write().unwrap_or_else(|p| p.into_inner()) = Arc::new(list);
        Ok(true)
    }

    /// The list for this app, in order.
    pub fn for_app(&self, name: &Name) -> Vec<Authority> {
        let list = Arc::clone(&self.current.read().unwrap_or_else(|p| p.into_inner()));
        list.iter()
            .map(|a| {
                let binding = if a.needs_binding {
                    self.bindings
                        .as_ref()
                        .and_then(|b| b.binding_for(&a.id, name))
                } else {
                    None
                };
                Authority {
                    id: a.id.clone(),
                    directory: a.directory.clone(),
                    needs_binding: a.needs_binding,
                    usable: !a.needs_binding || binding.is_some(),
                    binding,
                }
            })
            .collect()
    }

    /// The CAA issuer domains of the listed authorities, in order, each
    /// once: what the zone's CAA record permits to issue.
    pub fn caa_issuers(&self) -> Vec<String> {
        let list = Arc::clone(&self.current.read().unwrap_or_else(|p| p.into_inner()));
        let mut out: Vec<String> = Vec::new();
        for a in list.iter() {
            if !out.contains(&a.caa_domain) {
                out.push(a.caa_domain.clone());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO: &str = r#"
[[authority]]
id = "letsencrypt"
directory = "https://acme-v02.api.letsencrypt.org/directory"
caa_domain = "letsencrypt.org"

[[authority]]
id = "zerossl"
directory = "https://acme.zerossl.com/v2/DV90"
caa_domain = "sectigo.com"
needs_binding = true
"#;

    fn name() -> Name {
        Name::parse("ledger-accrual-tally-mint").unwrap()
    }

    struct PerApp;
    impl BindingSource for PerApp {
        fn binding_for(&self, id: &str, name: &Name) -> Option<Binding> {
            Some(Binding {
                kid: format!("{id}-{name}"),
                hmac_key: "x".into(),
            })
        }
    }

    #[test]
    fn the_list_is_served_in_order_and_a_binding_comes_only_from_a_source() {
        let l = AuthorityList::from_toml(TWO).unwrap();
        let got = l.for_app(&name());
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].id, "letsencrypt");
        assert!(!got[0].needs_binding && got[0].binding.is_none() && got[0].usable);
        assert_eq!(got[1].id, "zerossl");
        // No source: listed, needs one, served without, and not usable yet.
        assert!(got[1].needs_binding && got[1].binding.is_none() && !got[1].usable);
        let l = l.with_binding_source(Arc::new(PerApp));
        let got = l.for_app(&name());
        assert!(got[1].usable);
        assert_eq!(
            got[1].binding,
            Some(Binding {
                kid: "zerossl-ledger-accrual-tally-mint".into(),
                hmac_key: "x".into()
            })
        );
        assert!(got[0].binding.is_none());
    }

    #[test]
    fn a_list_file_that_carries_a_binding_secret_is_refused_in_words() {
        for secret in [
            "binding_kid = 'k'\nbinding_hmac_key = 'a2V5'\n",
            "binding_kid = 'k'\n",
            "binding_hmac_key = 'a2V5'\n",
        ] {
            let text = format!(
                "[[authority]]\nid='z'\ndirectory='https://x/d'\ncaa_domain='x.example'\nneeds_binding=true\n{secret}"
            );
            let err = AuthorityList::from_toml(&text).err().expect("refused");
            let said = err.to_string();
            assert!(
                said.contains("binding secret") && said.contains("per-app binding source"),
                "{said}"
            );
        }
    }

    #[test]
    fn a_bad_list_is_refused() {
        for bad in [
            "",
            "[[authority]]\nid='a'\ndirectory='http://example.com/dir'\ncaa_domain='x.example'\n",
            // no caa_domain, and a caa_domain that is not a plain domain name
            "[[authority]]\nid='a'\ndirectory='https://x/d'\n",
            "[[authority]]\nid='a'\ndirectory='https://x/d'\ncaa_domain='x.example; accounturi=y'\n",
            "[[authority]]\nid='a'\ndirectory='https://x/d'\ncaa_domain='Example.COM'\n",
            "[[authority]]\nid='a'\ndirectory='https://x/d'\ncaa_domain='localhost'\n",
            "[[authority]]\nid='a'\ndirectory='https://x/d'\ncaa_domain='x.example'\n[[authority]]\nid='a'\ndirectory='https://y/d'\ncaa_domain='y.example'\n",
            "[[authority]]\nid='a'\ndirectory='https://x/d'\ncaa_domain='x.example'\nsurprise=1\n",
        ] {
            assert!(AuthorityList::from_toml(bad).is_err(), "{bad:?}");
        }
        let many: String = (0..17)
            .map(|i| {
                format!(
                    "[[authority]]\nid='a{i}'\ndirectory='https://x/d'\ncaa_domain='x.example'\n"
                )
            })
            .collect();
        assert!(AuthorityList::from_toml(&many).is_err());
    }

    #[test]
    fn an_edit_changes_what_is_served_and_a_bad_edit_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("authorities.toml");
        std::fs::write(&path, TWO).unwrap();
        let l = AuthorityList::from_file(path.clone()).unwrap();
        assert_eq!(l.for_app(&name())[0].id, "letsencrypt");
        assert_eq!(l.caa_issuers(), ["letsencrypt.org", "sectigo.com"]);
        let flipped = "[[authority]]\nid='zerossl'\ndirectory='https://acme.zerossl.com/v2/DV90'\ncaa_domain='sectigo.com'\nneeds_binding=true\n[[authority]]\nid='letsencrypt'\ndirectory='https://acme-v02.api.letsencrypt.org/directory'\ncaa_domain='letsencrypt.org'\n";
        std::fs::write(&path, flipped).unwrap();
        assert!(l.reload().unwrap());
        assert_eq!(l.for_app(&name())[0].id, "zerossl");
        assert_eq!(l.caa_issuers(), ["sectigo.com", "letsencrypt.org"]);
        std::fs::write(&path, "not toml {{{").unwrap();
        assert!(l.reload().is_err());
        assert_eq!(l.for_app(&name())[0].id, "zerossl", "the good list stands");
        // A file that gains a binding secret is a bad file too.
        let secret = format!("{flipped}binding_kid='k'\nbinding_hmac_key='a2V5'\n");
        std::fs::write(&path, secret).unwrap();
        assert!(l.reload().is_err());
        assert_eq!(l.for_app(&name())[0].id, "zerossl", "the good list stands");
    }

    #[test]
    fn a_second_edit_of_the_same_length_at_once_is_not_missed() {
        // Nothing here moves a file time or waits: two edits in one clock
        // tick keep the same time, and the same length, and still change
        // the list.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("authorities.toml");
        let with = |ca: &str| {
            format!("[[authority]]\nid='a'\ndirectory='https://x/d'\ncaa_domain='{ca}.example'\n")
        };
        std::fs::write(&path, with("one")).unwrap();
        let l = AuthorityList::from_file(path.clone()).unwrap();
        assert!(!l.reload().unwrap(), "unchanged: nothing replaced");
        for (n, ca) in ["two", "tre", "for", "one"].into_iter().enumerate() {
            std::fs::write(&path, with(ca)).unwrap();
            assert!(l.reload().unwrap(), "edit {n} ({ca}) was missed");
            assert_eq!(l.caa_issuers(), [format!("{ca}.example")]);
            assert!(!l.reload().unwrap(), "edit {n} read twice");
        }
        // A bad file is refused once and the good list stands; fixing it back
        // to what the list already is changes nothing served but is read.
        std::fs::write(&path, "garbage {{{").unwrap();
        assert!(l.reload().is_err());
        assert!(
            !l.reload().unwrap(),
            "the same bad file is not refused twice"
        );
        assert_eq!(l.caa_issuers(), ["one.example"]);
        std::fs::write(&path, with("one")).unwrap();
        assert!(l.reload().unwrap());
        // A file too large to be a list is refused unread.
        std::fs::write(&path, " ".repeat(70 * 1024)).unwrap();
        assert!(l.reload().is_err());
        assert_eq!(l.caa_issuers(), ["one.example"]);
    }

    #[test]
    fn a_binding_source_is_asked_only_for_authorities_that_need_one() {
        let l = AuthorityList::from_toml(TWO)
            .unwrap()
            .with_binding_source(Arc::new(PerApp));
        let got = l.for_app(&name());
        assert!(got[0].binding.is_none(), "letsencrypt needs none");
        assert!(got[1].binding.is_some());
    }

    #[test]
    fn two_authorities_with_one_issuer_name_it_once() {
        let text = "[[authority]]\nid='a'\ndirectory='https://x/d'\ncaa_domain='x.example'\n[[authority]]\nid='b'\ndirectory='https://y/d'\ncaa_domain='x.example'\n";
        assert_eq!(
            AuthorityList::from_toml(text).unwrap().caa_issuers(),
            ["x.example"]
        );
        assert!(AuthorityList::empty().caa_issuers().is_empty());
    }
}
