# QuickBooks Desktop IIF (Intuit Interchange Format): general journal entries
Governs `ledger_import::iif` (IIF export of posted entries for QuickBooks Desktop). Intuit's help pages and IIF Import Kit are © Intuit Inc., not public domain, so per the vendoring rules (docs/spec/README.md) they are cited, with short quotes only; everything not inside a `>` blockquote is our description, not Intuit's words. The most authoritative source found is Intuit's own IIF Import Kit (header reference "IIF Header Help", the "IIF Import Manual" PDF updated 9/30/2008, and per-version example files), linked from the current help article below.

## IIF Overview: import kit, sample files, and headers (US, updated 8/4/2026)
Source: <https://quickbooks.intuit.com/learn-support/en-us/help-article/list-management/iif-overview-import-kit-sample-files-headers/L5CZIpJne_US_en_US>  
Retrieved: 2026-10-13  
SHA-256: not available (read in a browser; the page's fetch included session query data and was not saved)

**What an IIF file is**

Cited (not quoted): IIF files are ASCII text, tab-separated values, used by QuickBooks Desktop to import or export lists or transactions. Header rows begin with an exclamation point and their first column holds a keyword. A transaction import file usually has three header rows, `!TRNS`, `!SPL`, `!ENDTRNS`.

**Intuit's worked example (a check), transcribed cell by cell**

| A | B | C | D | E | F | G |
|---|---|---|---|---|---|---|
| !TRNS | TRNSTYPE | DATE | ACCNT | NAME | CLASS | AMOUNT |
| !SPL | TRNSTYPE | DATE | ACCNT | NAME | CLASS | AMOUNT |
| !ENDTRNS | | | | | | |
| TRNS | CHECK | 3/3/2010 | Main Checking | ArchCo Gas | | -70 |
| SPL | CHECK | 3/3/2010 | Fuel | Courtney, Ed | | 10 |
| SPL | CHECK | 3/3/2010 | Auto Repair | | | 60 |
| ENDTRNS | | | | | | |

Cited (not quoted): the bank account (credited) is -70 and the two expense splits (debited) are +10 and +60, so TRNS plus SPL amounts sum to zero. Not every header is needed for a transaction to import.

**Unknown accounts**

> If you import a transaction with an account that is not already created in the QuickBooks data file, that account will be created as a new bank account.

Cited (not quoted): so every account named in the file must exist in QuickBooks before import. IIF import does only limited error checking; a malformed file can import incorrectly with no error. Intuit gives no assisted support for creating IIF files.

## Export, import, and edit IIF files (US, updated 8/4/2026)
Source: <https://quickbooks.intuit.com/learn-support/en-us/help-article/import-export-data-files/export-import-edit-iif-files/L56LT9Z0Q_US_en_US>  
Retrieved: 2026-10-13  
SHA-256 of the retrieved file (`L56LT9Z0Q_US_en_US.html`): `a52d5ccb79014765831421f73dd13c64c85c6bd98a56d3e47234edd93af941ab`

Cited (not quoted): general journal entries are among the importable transaction types; transactions can be imported but not exported; imported transactions are not linked to each other. Import is File > Utilities > Import > IIF Files.

## IIF Import Kit (`IIF_Import_Kit.zip`)
Source: <https://support.quickbooks.intuit.com/support/pages/executable/iif/iif_import_kit.zip> (redirects to <https://http-download.intuit.com/http.intuit/OpenCms/sites/default/QBSupportSite/executables/IIF/IIF_Import_Kit.zip>)  
Retrieved: 2026-10-13  
SHA-256 of the retrieved file (`IIF_Import_Kit.zip`): `45dde93a8bec83569820a63c50bb2ca0477e10380c358b4838e87c74382f0834`

### IIF Header Help: Transactions (TRNS and SPL)
SHA-256 of `IIF Import Kit/IIF Header Help/!TRNS.htm`: `a303221fb779a6d9383e6f3a5730e44f40e16fcd04ae1cc122dad142ec37d499`

**Structure**

Cited (not quoted): the transaction keyword is TRNS and each distribution line's is SPL; the SPL lines follow the TRNS line immediately, and ENDTRNS follows the last SPL. TRNSTYPE, ACCNT, and AMOUNT are the required TRNS columns. `GENERAL JOURNAL` is the TRNSTYPE keyword for general journal transactions.

**AMOUNT sign**

> Debit amounts are always positive, credit amounts are always negative.

**Other TRNS columns (cited, not quoted)**

- `TRNSID`, `TIMESTAMP`: identifiers.
- `DATE`: the help says the date is always in MM/DD/YY format, with the example 1/30/94. (Intuit's own general journal example file, below, uses a four-digit year: 7/1/1998.)
- `ACCNT` (required): the account's name.
- `NAME`: customer, vendor, payee, or employee.
- `CLASS`: class name; a subclass is written with its parents, highest first, separated by colons.
- `DOCNUM`: the transaction's number.
- `MEMO`: memo text.
- `CLEAR`: `Y` cleared, `N` not cleared.
- `TOPRINT`, `ADDR1`–`ADDR5`, `DUEDATE`, `TERMS`, `PAID`, `PAYMETH`, `SHIPVIA`, `SHIPDATE`, `REP`, `FOB`, `PONUM`, `INVTITLE`, `INVMEMO`, `SADDR1`–`SADDR5`: document-specific fields not used by a general journal entry.

### IIF Header Help: distribution lines (SPL)
SHA-256 of `IIF Import Kit/IIF Header Help/!SPL.htm`: `87cc94da55a7291b76402bb505677f4cd5e3317e1c7602fcaca81f56b2fba4c1`

Cited (not quoted): `SPLID` is marked required (a unique number for the line within the transaction); `TRNSTYPE` and `DATE` on SPL must match the TRNS line's; `ACCNT` is required; `AMOUNT` is required and credit amounts are negative; `NAME`, `CLASS`, `DOCNUM`, `MEMO`, `CLEAR` as on TRNS. (Intuit's own general journal example file leaves the SPLID column empty, below.)

### IIF Header Help: Chart of accounts (ACCNT)
SHA-256 of `IIF Import Kit/IIF Header Help/!ACCNT.htm`: `91b5cbcc90b3b251b467448123945b5388717095aed0e75a1a14847e1469262e`

**Subaccount names**

> If you are creating an import file, use a colon (:) to separate subaccount names.

Cited (not quoted): a subaccount's NAME includes its parents' names, highest level first. Accounts are identified by NAME; the account number (`ACCNUM`) is a separate, optional column.

### IIF Example Files: QB 2007 & Enterprise 7.0 and later, General Journal Entry
SHA-256 of `IIF Import Kit/IIF Example Files/QB 2007 & Enterprise 7.0 and later/Journal Entry/2007 General Journal Entry.iif`: `78917e0f848f38b55f0f1bbde16ef19b9144132a2b21f11831ff29b2ba549879`

The file's records, transcribed (each `→` is one tab character; every line ends CR LF in the file):

```text
!TRNS→TRNSID→TRNSTYPE→DATE→ACCNT→CLASS→AMOUNT→DOCNUM→MEMO
!SPL→SPLID→TRNSTYPE→DATE→ACCNT→CLASS→AMOUNT→DOCNUM→MEMO
!ENDTRNS→→→→→→→→
TRNS→→GENERAL JOURNAL→7/1/1998→Checking→→650→→
SPL→→GENERAL JOURNAL→7/1/1998→Expense Account→→-650→→
ENDTRNS→→→→→→→→
```

Cited (not quoted): debit +650, credit -650, sum zero; TRNSID and SPLID left empty; DATE is M/D/YYYY without leading zeros.

### IIF Import Manual ("Start Here - IIF Import Manual.pdf", updated 9/30/2008)
SHA-256 of `IIF Import Kit/Start Here - IIF Import Manual.pdf`: `e8f4185ac61194e1e18a8f5a61f1ea47d1d073efd815f8127e3fc77682c744f6`

Cited (not quoted): IIF is tab-delimited, usually edited in a spreadsheet; the format is unforgiving of syntax errors; nothing stops the same file being imported twice (duplicating every transaction) and there is no undo; list elements (accounts, names) must already exist or be declared at the top of the file before the transactions, or default types are assumed; an error's line number is the spreadsheet row, usually the ENDTRNS that tried to commit the transaction; the file format depends on the QuickBooks product year.

## General Journal sample (`journal.zip`)
Source: <https://support.quickbooks.intuit.com/support/pages/executable/iif/journal.zip> (redirects to <https://http-download.intuit.com/http.intuit/OpenCms/sites/default/QBSupportSite/executables/IIF/journal.zip>)  
Retrieved: 2026-10-13  
SHA-256 of the retrieved file (`journal.zip`): `4f578d227810f5bfadde142cdab9f941f96de3d2c8c201da8d61126799e9f500`

Cited (not quoted): its read-me says list declarations (accounts, classes, customers, vendors, other names) come first so names land on the correct lists, and that header columns must be present even when left blank. Its `!ACCNT` rows show a subaccount written `Construction:Labor`. Its address fields that contain a comma are wrapped in double quotes (`"Anywhere, AZ 85740"`), as a spreadsheet writes tab-delimited text; the kit does not say whether QuickBooks strips those quotes.

## Not found in any Intuit source

Searched 2026-10-13 (the help articles above and every document in the IIF Import Kit and journal sample); none states:

- an escaping mechanism for tabs, line breaks, or double quotes inside a field (no escape syntax is documented; the only safe reading is that fields must not contain tabs or line breaks);
- an explicit sentence that a transaction's TRNS and SPL amounts must sum to zero (every example does; the sign rule implies it);
- the allowed number of decimal places, or whether thousands separators or currency symbols are accepted in AMOUNT;
- the character encoding beyond "ASCII text";
- whether a four-digit year is accepted everywhere despite the "MM/DD/YY" wording (Intuit's 2007+ example uses one).
