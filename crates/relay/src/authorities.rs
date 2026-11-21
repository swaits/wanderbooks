//! The certificate authorities: a list the relay serves, not code.
//!
//! Every authority an app may use speaks the same protocol (ACME,
//! RFC 8555). The app's certificate code knows the protocol and nothing
//! about any one authority; at each issuance it asks the relay for the
//! list to try, in order. Swapping Let's Encrypt for Google Trust Services
//! or ZeroSSL, or adding one, is an edit to the list the relay reads, with
//! no release of the app.
//!
//! ```toml
//! [[authority]]
//! id = "letsencrypt"
//! directory = "https://acme-v02.api.letsencrypt.org/directory"
//!
//! [[authority]]
//! id = "zerossl"
//! directory = "https://acme.zerossl.com/v2/DV90"
//! needs_binding = true
//! binding_kid = "…"
//! binding_hmac_key = "…"
//! ```
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
//!   2. An authority that needs a binding is served only with one: a
//!      list that names one without a credential is refused when read,
//!      and the previous list stays in force.
//!   3. A bad file never replaces a good list.
//!   4. The list is bounded: at most 16 authorities.
//! SCENARIOS:
//!   1. Two authorities, the first plain and the second with a binding,
//!      are served in that order, the second with its credential.
//!   2. The file is edited to put the second first: the next read serves
//!      that order.
//!   3. The file is edited to garbage: the old list is still served.
//! ```

use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

use serde::Deserialize;

use crate::names::Name;
use crate::proto::{Authority, Binding};

const MAX_AUTHORITIES: usize = 16;

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
    #[serde(default)]
    needs_binding: bool,
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

/// Where an app's binding credential comes from. The static list gives
/// every app the credential written in the file; a deployment that
/// obtains one per app from the authority's API implements this instead.
pub trait BindingSource: Send + Sync {
    /// The credential for this app at this authority, if one is needed.
    fn binding_for(&self, authority_id: &str, name: &Name) -> Option<Binding>;
}

#[derive(Debug, Clone)]
struct Listed {
    id: String,
    directory: String,
    needs_binding: bool,
    binding: Option<Binding>,
}

/// The list the relay serves.
pub struct AuthorityList {
    current: RwLock<Arc<Vec<Listed>>>,
    source: Option<PathBuf>,
    seen: std::sync::Mutex<Option<SystemTime>>,
    bindings: Option<Arc<dyn BindingSource>>,
}

fn is_loopback_http(url: &str) -> bool {
    ["http://127.0.0.1", "http://localhost", "http://[::1]"]
        .iter()
        .any(|p| url.starts_with(p))
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
        let binding = match (e.binding_kid, e.binding_hmac_key) {
            (Some(kid), Some(hmac_key)) => Some(Binding { kid, hmac_key }),
            (None, None) => None,
            _ => {
                return Err(ListError::Invalid(format!(
                    "{}: a binding needs both its kid and its key",
                    e.id
                )));
            }
        };
        if e.needs_binding && binding.is_none() {
            return Err(ListError::Invalid(format!(
                "{} needs a binding and has none",
                e.id
            )));
        }
        out.push(Listed {
            id: e.id,
            directory: e.directory,
            needs_binding: e.needs_binding,
            binding,
        });
    }
    Ok(out)
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
        let text = std::fs::read_to_string(&path)?;
        let list = parse(&text)?;
        let seen = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        Ok(AuthorityList {
            current: RwLock::new(Arc::new(list)),
            source: Some(path),
            seen: std::sync::Mutex::new(seen),
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

    /// Take bindings from `source` instead of the file's.
    pub fn with_binding_source(mut self, source: Arc<dyn BindingSource>) -> Self {
        self.bindings = Some(source);
        self
    }

    /// Read the file again if it changed. A bad file changes nothing.
    /// Returns whether the list was replaced.
    pub fn reload(&self) -> Result<bool, ListError> {
        let Some(path) = &self.source else {
            return Ok(false);
        };
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        {
            let mut seen = self.seen.lock().unwrap_or_else(|p| p.into_inner());
            if *seen == modified && modified.is_some() {
                return Ok(false);
            }
            *seen = modified;
        }
        let list = parse(&std::fs::read_to_string(path)?)?;
        *self.current.write().unwrap_or_else(|p| p.into_inner()) = Arc::new(list);
        Ok(true)
    }

    /// The list for this app, in order.
    pub fn for_app(&self, name: &Name) -> Vec<Authority> {
        let list = Arc::clone(&self.current.read().unwrap_or_else(|p| p.into_inner()));
        list.iter()
            .map(|a| Authority {
                id: a.id.clone(),
                directory: a.directory.clone(),
                needs_binding: a.needs_binding,
                binding: if a.needs_binding {
                    self.bindings
                        .as_ref()
                        .and_then(|b| b.binding_for(&a.id, name))
                        .or_else(|| a.binding.clone())
                } else {
                    None
                },
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO: &str = r#"
[[authority]]
id = "letsencrypt"
directory = "https://acme-v02.api.letsencrypt.org/directory"

[[authority]]
id = "zerossl"
directory = "https://acme.zerossl.com/v2/DV90"
needs_binding = true
binding_kid = "kid-1"
binding_hmac_key = "a2V5"
"#;

    fn name() -> Name {
        Name::parse("ledger-accrual-tally-mint").unwrap()
    }

    #[test]
    fn the_list_is_served_in_order_with_its_binding() {
        let l = AuthorityList::from_toml(TWO).unwrap();
        let got = l.for_app(&name());
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].id, "letsencrypt");
        assert!(!got[0].needs_binding && got[0].binding.is_none());
        assert_eq!(got[1].id, "zerossl");
        assert!(got[1].needs_binding);
        assert_eq!(
            got[1].binding,
            Some(Binding {
                kid: "kid-1".into(),
                hmac_key: "a2V5".into()
            })
        );
    }

    #[test]
    fn a_bad_list_is_refused() {
        for bad in [
            "",
            "[[authority]]\nid='a'\ndirectory='http://example.com/dir'\n",
            "[[authority]]\nid='a'\ndirectory='https://x/d'\nneeds_binding=true\n",
            "[[authority]]\nid='a'\ndirectory='https://x/d'\nbinding_kid='k'\n",
            "[[authority]]\nid='a'\ndirectory='https://x/d'\n[[authority]]\nid='a'\ndirectory='https://y/d'\n",
            "[[authority]]\nid='a'\ndirectory='https://x/d'\nsurprise=1\n",
        ] {
            assert!(AuthorityList::from_toml(bad).is_err(), "{bad:?}");
        }
        let many: String = (0..17)
            .map(|i| format!("[[authority]]\nid='a{i}'\ndirectory='https://x/d'\n"))
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
        let flipped = "[[authority]]\nid='zerossl'\ndirectory='https://acme.zerossl.com/v2/DV90'\nneeds_binding=true\nbinding_kid='k'\nbinding_hmac_key='a2V5'\n[[authority]]\nid='letsencrypt'\ndirectory='https://acme-v02.api.letsencrypt.org/directory'\n";
        std::fs::write(&path, flipped).unwrap();
        // Same-second edits may not move the mtime: force it to look new.
        *l.seen.lock().unwrap() = None;
        assert!(l.reload().unwrap());
        assert_eq!(l.for_app(&name())[0].id, "zerossl");
        std::fs::write(&path, "not toml {{{").unwrap();
        *l.seen.lock().unwrap() = None;
        assert!(l.reload().is_err());
        assert_eq!(l.for_app(&name())[0].id, "zerossl", "the good list stands");
    }

    #[test]
    fn a_binding_source_overrides_the_file() {
        struct PerApp;
        impl BindingSource for PerApp {
            fn binding_for(&self, id: &str, name: &Name) -> Option<Binding> {
                Some(Binding {
                    kid: format!("{id}-{name}"),
                    hmac_key: "x".into(),
                })
            }
        }
        let l = AuthorityList::from_toml(TWO)
            .unwrap()
            .with_binding_source(Arc::new(PerApp));
        let got = l.for_app(&name());
        assert_eq!(
            got[1].binding.as_ref().unwrap().kid,
            "zerossl-ledger-accrual-tally-mint"
        );
        assert!(got[0].binding.is_none());
    }
}
