//! The records of a backup: pieces, series of pieces, and the manifest
//! that names them, and the names a piece is kept under.
//!
//! A *series* is what grows: the journal, the access log and the index of
//! stored files are each a run of immutable segments whose bytes, read one
//! after another, are the whole. A backup that has new lines adds a
//! segment; nothing already written is touched.

use super::BackupError;
use super::seal::Keys;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// What a manifest says it is.
pub(crate) const FORMAT: &str = "wanderbooks-backup";
/// The form's version. A reader refuses one it does not know.
pub(crate) const VERSION: u32 = 1;
/// The folder of manifests in a set.
pub(crate) const MANIFESTS: &str = "manifests";
/// The words a person can open.
pub(crate) const READ_ME: &str = "READ-ME.txt";
/// The sealed form's header: how to derive the key.
pub(crate) const HEADER: &str = "header.json";
/// Where a sealed set keeps every piece, under names that say nothing.
pub(crate) const PIECES: &str = "pieces";
/// A series with this many segments is next written whole as one: a
/// manifest stays small however long the books have been backed up.
pub(crate) const CONSOLIDATE_AT: usize = 200;

/// The kinds of piece.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A segment of the journal.
    Journal,
    /// A segment of the access log.
    Access,
    /// A segment of the index of stored files.
    Index,
    /// A stored file.
    File,
    /// A vault record.
    Vault,
    /// A key file.
    Key,
}

impl Kind {
    pub(crate) fn dir(self) -> &'static str {
        match self {
            Kind::Journal => "journal",
            Kind::Access => "access",
            Kind::Index => "files-index",
            Kind::File => "files",
            Kind::Vault => "vault",
            Kind::Key => "keys",
        }
    }
}

/// One piece: a file of the set, by where it is, how long it is as it is
/// kept, and what is in it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Piece {
    /// Its path in the set.
    pub file: String,
    /// Its length as kept (sealed pieces are longer than their contents).
    pub stored: u64,
    /// Its contents' length.
    pub bytes: u64,
    /// Its contents' SHA-256, hex.
    pub sha256: String,
}

/// A run of segments, and what they make together.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Series {
    /// Bytes in all the segments.
    pub bytes: u64,
    /// SHA-256 of all of them, one after another, hex.
    pub sha256: String,
    /// The segments, in order.
    pub segments: Vec<Piece>,
}

/// A file named by its place in a flat folder of a book (the vault, the
/// keys).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Named {
    /// Its name in the book's folder.
    pub name: String,
    /// Where it is kept.
    #[serde(flatten)]
    pub piece: Piece,
}

/// The journal's head.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Head {
    /// Its last line's seq.
    pub seq: u64,
    /// Its last line's hash.
    pub hash: String,
}

/// What makes a backup whole: written last, and never again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Manifest {
    /// [`FORMAT`].
    pub format: String,
    /// [`VERSION`].
    pub version: u32,
    /// The books' id, as they were backed up.
    pub book: String,
    /// Its number in the set: also its name.
    pub number: u64,
    /// When it was written (UTC).
    pub written: String,
    /// The journal's head it reaches.
    pub head: Head,
    /// The journal.
    pub journal: Series,
    /// The access log.
    pub access: Series,
    /// The index of stored files: one `{"sha256","bytes"}` per line.
    pub files: Series,
    /// How many stored files the index names, and their bytes.
    pub file_count: u64,
    /// Bytes of the stored files.
    pub file_bytes: u64,
    /// The vault's records.
    pub vault: Vec<Named>,
    /// The keys.
    pub keys: Vec<Named>,
}

/// A line of the index of stored files.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IndexLine {
    /// The file's SHA-256, hex.
    pub sha256: String,
    /// Its length.
    pub bytes: u64,
}

/// SHA-256, hex.
pub(crate) fn sha_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Whether `s` is a SHA-256 in hex as this program writes it.
pub(crate) fn is_sha(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A name in a book's flat folders: nothing that could leave the folder.
pub(crate) fn is_plain_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 200
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// The manifest's file name in a set, as its number.
pub(crate) fn manifest_path(number: u64, sealed: bool) -> String {
    if sealed {
        format!("{MANIFESTS}/{number:06}")
    } else {
        format!("{MANIFESTS}/{number:06}.json")
    }
}

/// The number a manifest file name carries, if it is one.
pub(crate) fn manifest_number(name: &str, sealed: bool) -> Option<u64> {
    let digits = if sealed {
        name
    } else {
        name.strip_suffix(".json")?
    };
    if digits.len() < 6 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u64 = digits.parse().ok()?;
    // Exactly the name this program writes (no conflict copy of one, no
    // other spelling of the same number).
    (manifest_path(n, sealed) == format!("{MANIFESTS}/{name}")).then_some(n)
}

/// Where a piece of `kind` whose contents hash to `sha` is kept, as the
/// `ordinal`th of its series. Sealed sets keep every piece under a name
/// made from the key and the hash: it says nothing of what the piece is.
pub(crate) fn piece_path(keys: Option<&Keys>, kind: Kind, ordinal: usize, sha: &str) -> String {
    match keys {
        Some(k) => format!("{PIECES}/{}", k.piece_name(kind.dir(), sha)),
        None => match kind {
            Kind::Journal | Kind::Access | Kind::Index => {
                format!("{}/{:06}-{}", kind.dir(), ordinal + 1, &sha[..16])
            }
            Kind::File => format!("{}/{}/{sha}", kind.dir(), &sha[..2]),
            Kind::Vault | Kind::Key => format!("{}/{sha}", kind.dir()),
        },
    }
}

/// A manifest as written: its JSON, a newline, and a line that is the
/// SHA-256 of that JSON, so a short or changed one is known for what it is.
pub(crate) fn encode_manifest(m: &Manifest) -> Vec<u8> {
    let body = serde_json::to_string(m).expect("a manifest serializes");
    let check = sha_hex(body.as_bytes());
    format!("{body}\nwanderbooks-manifest-sha256 {check}\n").into_bytes()
}

/// A manifest read back, or why it is not one.
pub(crate) fn decode_manifest(bytes: &[u8]) -> Result<Manifest, BackupError> {
    let bad = || BackupError::Damaged("a manifest of the backup is cut short or changed".into());
    let text = std::str::from_utf8(bytes).map_err(|_| bad())?;
    let text = text.strip_suffix('\n').ok_or_else(bad)?;
    let (body, check) = text.rsplit_once('\n').ok_or_else(bad)?;
    let check = check
        .strip_prefix("wanderbooks-manifest-sha256 ")
        .ok_or_else(bad)?;
    if check != sha_hex(body.as_bytes()) {
        return Err(bad());
    }
    let m: Manifest = serde_json::from_str(body).map_err(|_| bad())?;
    if m.format != FORMAT {
        return Err(BackupError::Damaged(
            "that isn't a wanderbooks backup".into(),
        ));
    }
    if m.version != VERSION {
        return Err(BackupError::Damaged(
            "that backup was made by a newer wanderbooks: update to read it".into(),
        ));
    }
    Ok(m)
}

impl Manifest {
    /// What this manifest holds, apart from its number and time: two with
    /// the same content need not both be written.
    pub(crate) fn same_content(&self, other: &Manifest) -> bool {
        let mut a = self.clone();
        let mut b = other.clone();
        for m in [&mut a, &mut b] {
            m.number = 0;
            m.written.clear();
        }
        a == b
    }

    /// Every name in it is one this program would have written: nothing
    /// that could lead out of the set or into another's.
    pub(crate) fn check_names(&self, sealed: bool) -> Result<(), BackupError> {
        let bad = |why: &str| BackupError::Damaged(format!("a manifest of the backup {why}"));
        if !is_sha(&self.head.hash) && self.head.seq > 0 {
            return Err(bad("has a head that is not a hash"));
        }
        for series in [&self.journal, &self.access, &self.files] {
            if !is_sha(&series.sha256) && series.bytes > 0 {
                return Err(bad("has a series that is not a hash"));
            }
            let total = series
                .segments
                .iter()
                .fold(0u64, |a, s| a.saturating_add(s.bytes));
            if total != series.bytes {
                return Err(bad("has segments that don't add up"));
            }
            for p in &series.segments {
                self.check_piece(p, sealed)?;
            }
        }
        for n in self.vault.iter().chain(&self.keys) {
            if !is_plain_name(&n.name) {
                return Err(bad("names a file no book keeps"));
            }
            self.check_piece(&n.piece, sealed)?;
        }
        Ok(())
    }

    fn check_piece(&self, p: &Piece, sealed: bool) -> Result<(), BackupError> {
        let bad = |why: &str| BackupError::Damaged(format!("a manifest of the backup {why}"));
        if !is_sha(&p.sha256) {
            return Err(bad("has a piece that is not a hash"));
        }
        let ok_path = p.file.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        });
        let starts = [
            "journal/",
            "access/",
            "files-index/",
            "files/",
            "vault/",
            "keys/",
        ];
        let right_place = if sealed {
            p.file.starts_with(&format!("{PIECES}/"))
        } else {
            starts.iter().any(|s| p.file.starts_with(s))
        };
        if !ok_path || !right_place {
            return Err(bad("names a piece outside the backup"));
        }
        Ok(())
    }
}
