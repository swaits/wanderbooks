# Area: format

Interchange formats and the journal's format gate.

### BS-format-1: The journal format gate: newer is not corrupt (B-27)

- **Status:** CHOICE (the owner, 2026-10-16; D-39, 2026-10-20)
- **Statement:** A reader MUST distinguish a journal line it does not know because it is newer from one that is corrupt: a line's own hash is checked before it is called newer; a reader stops at the first line it cannot read and says the journal is newer; older books move to a newer format only with the owner's go-ahead.
- **Sources:**
  - external: CHOICE only: the owner, 2026-10-16 (docs/spec/rules.md B-27); no authority governs a file format's evolution
- **Symbolic form:** `hash_ok(line) and type(line) unknown implies newer`; `not hash_ok(line) implies corrupt`.
- **Examples:** none yet. The example schema has no journal-file event; the gate is checked by wanderbooks' store tests.
- **Applicability:** engines that persist a journal.

### BS-format-2: The QuickBooks Desktop file (IIF): plain ASCII, every entry through as_of, closes left out, classes carried (B-34)

- **Status:** AUTHORITY
- **Statement:** An IIF file MUST be plain ASCII, tab-separated, with each line ending CR LF; lists (accounts, classes) MUST come before the transactions that use them; an account is identified by its name, a subaccount's name its path joined by colons; every entry through the as-of day MUST be in the file and closing entries left out; classes MUST be carried; text the file must write differently from the books (a name past 31 characters, a memo past 4,095) MUST be reported as a change.
- **Sources:**
  - vendored `sources/intuit-iif.md`: "Accounts are identified by NAME"
- **Symbolic form:** `file = header lists transactions`, `forall line: ascii(line) and tabs(line)`.
- **Examples:** `iif-quickbooks-journal-file`
- **Applicability:** exports to QuickBooks Desktop.
