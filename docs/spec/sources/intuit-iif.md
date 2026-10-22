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

### IIF Header Help: Class list (CLASS)
Retrieved: 2026-10-22 (the kit above, unchanged: same SHA-256)  
SHA-256 of `IIF Import Kit/IIF Header Help/!CLASS.htm`: `de8a89aac069946583ed1fec7b9042162343b5b34146fb456d17851bccca0ff8`

Cited (not quoted, but for the lines below): the Class list's keyword is CLASS; of its columns (NAME, and TIMESTAMP and REFNUM in export files only) the page says:

> Only the NAME column is required.

> If the class is a subclass of another class, the name includes the name of the parent classes, beginning with the highest level class. If you are creating an import file, use a colon (:) to separate subclass names.

So a class's NAME, like an account's, is one level per colon: the books, whose classes have no parents, write a colon in a class's name as '-', as in an account's (rules.md B-34 rule 8). The page states no length limit and nothing about case.

### IIF Example Files: list declarations
SHA-256 of `IIF Import Kit/IIF Example Files/QB 2007 & Enterprise 7.0 and later/Import First - List Declarations (Not Version Specific).iif` (the same bytes in each version's folder): `13c730b23bcb8c142621b04206e3628241d83051442c9b14ea2416d21163439c`

Cited (not quoted): a file named "Import First", declaring the lists (accounts, classes, names) that the transaction examples use; its class rows, transcribed (each `→` one tab; the trailing empty fields left out):

```text
!CLASS→NAME→REFNUM→TIMESTAMP→HIDDEN→DELCOUNT
CLASS→Class Name→2→1154540261→N→0
CLASS→Owner→1→1154540254→N→0
```

The kit's `2004 Deposit (Income).iif` (`IIF Example Files/QB 2004 & Enterprise 4.0/Deposit/`) (SHA-256 `82a5d0efbdeb9440a81e48fa1675355e37d2d78650a026c87caa640d0955ffdf`) names the declared class "Class Name" in the CLASS column of its SPL line, its TRNS line's left empty: a line's class is the one it names, per line. So a class is declared in a `!CLASS` list before the transactions that name it, as accounts are (the Import Manual and `journal.zip`'s read-me, above and below: lists first, or default types are assumed). The books write the one required column, NAME.

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

## Character limitations for fields in QuickBooks (US, updated 8/3/2026)
Source: <https://quickbooks.intuit.com/learn-support/en-us/help-article/printing-preferences/character-limitations-fields-quickbooks/L7eIy5gE3_US_en_US>  
Retrieved: 2026-10-20  
SHA-256: not available (read through a fetch tool; a direct download was refused by the server)

Tagged for QuickBooks Desktop (Mac Plus, Premier Plus, Pro Plus). Its one sentence of text:

> Each field in your company data file has a maximum number of characters that can be entered.

Cited (not quoted), from its table: Account Name 31; Account Numbers 7; Memo 4,095 (of which printed checks show about 30 to 60 and the Transaction Detail report 96); invoice number 11; customer, vendor and other names 41. It has no row for a journal entry's number, and says nothing of what QuickBooks does with longer text, on import or otherwise.

The same article for the UK (`.../en-uk/help-article/printing-preferences/character-limitations-fields-quickbooks/L7eIy5gE3_GB_en_GB`, retrieved 2026-10-20) gives Account Name 31 but Account Numbers 25: a different product. The books are a US company's, so the US figures hold.

## Intuit's .NET SDK reference: `Account.Name`
Source: <https://static.developer.intuit.com/sdkdocs/qbv3doc/ippdotnetdevkitv3/html/cd41295e-8e12-8b96-1ff3-728a807fa9cd.htm>  
Retrieved: 2026-10-20

Cited (not quoted): "User recognizable name for the Account", with ValidRange "QBW: Max=31" (QuickBooks Desktop; QuickBooks Online: 100). Its `Account.AcctNum` page (`.../95a0835b-80bd-4fe1-2d7f-96ea2c372442.htm`) states no length.

So the limits are Intuit's own: an account name (one level of a subaccount's NAME, as the account's own name is entered) at most 31 characters, an account number at most 7, a memo at most 4,095. The books write each within them: a longer level is cut to 31 (then no two NAMEs may match), a longer number is left out of ACCNUM (it is optional; the NAME identifies the account), a longer memo is cut to 4,095, and every one is listed (rules.md B-34).

## Add, edit, or delete accounts in QuickBooks Desktop (US, updated 8/4/2026)
Source: <https://quickbooks.intuit.com/learn-support/en-us/help-article/chart-accounts/add-edit-delete-accounts-quickbooks-desktop-mac/L95VT34uN_US_en_US>  
Retrieved: 2026-10-20  
SHA-256: not available (read through a fetch tool)

Tagged for QuickBooks Desktop (Mac Plus, Premier Plus, Pro Plus, Accountant, Enterprise). Under "Add a subaccount", a note:

> The parent and subaccount must be the same account type.

So the books write a subaccount whose QuickBooks type differs from its parent's (a bank account under a header that is no bank, an asset's cost account under a "Fixed assets" header) at the top level, its own subaccounts with it, and list it (rules.md B-34). Intuit's own journal sample (`journal.zip`, above) declares `Construction:Labor` without ever declaring `Construction`, so QuickBooks makes a missing parent itself; the books declare each parent before its subaccounts all the same, so a parent keeps its own type and number rather than taking ones QuickBooks guesses.

## Customer and vendor names on accounts receivable and payable

Searched 2026-10-20. What Intuit says:

- The IIF Import Kit's header help (`!TRNS.htm`, `!SPL.htm`, above) marks TRNSTYPE, ACCNT and AMOUNT required; NAME is "the name of the customer, vendor, payee, or employee", not marked required, with no word on accounts receivable or payable. The kit's General Journal example and the journal sample have no NAME column at all.
- The journal sample's read-me (`journal.zip`, `General Journal IIF file read me.txt`, SHA-256 `2e45b0f7a15d78f487d510547084d401d99bda02630d10747e37e00ca56f2c52`): "A customer name attached to the spl line of a General Journal transaction will import to the vendor list and the vendor name attached to the trns line of a General Journal transaction will import to the other names list unless the names are already on the correct lists." So a NAME written without declaring the customer and vendor lists first lands on the wrong list.
- Intuit's QuickBooks Team, answering "Can I post a journal entry to Accounts Receivable without specifying a customer name?" (Maybelle_S, QuickBooks Team badge, 12 January 2024), <https://quickbooks.intuit.com/community/reports-and-accounting-5/can-i-post-a-journal-entry-to-accounts-receivable-without-specifying-a-customer-name-64395> (retrieved 2026-10-20), about QuickBooks Desktop:

  > Accounts Receivable (AR) or Accounts Payable (AP) requires a customer or vendor name assignment.

- "Troubleshoot ... Warning: You Cannot Use More Than One" (US, updated 8/5/2026; QuickBooks Desktop Mac Plus, Premier Plus, Pro Plus), <https://quickbooks.intuit.com/learn-support/en-us/help-article/accounts-receivable/warning-cannot-use-one-ar-ap-account-transaction/L7xqDWcmE_US_en_US> (retrieved 2026-10-20): QuickBooks will not record a transaction with two A/R accounts, two A/P accounts, or one of each; its message:

  > Warning: You cannot use more than one A/R or A/P account in the same transaction.

Not Intuit, for the record: a third-party IIF vendor answering on Intuit's community (BigRedConsulting, 14 April 2020, <https://quickbooks.intuit.com/learn-support/en-us/other-questions/iif-import-error-3040/00/535675>) wrote that "a NAME is required for any AR row", about a bill, not a journal; QuickBooks Desktop sync tools quote the refusal "Transaction split lines to accounts receivable must include a customer on that split line" (Acctivate, <https://hub.acctivate.com/articles/troubleshoot/synchronization/transaction-split-lines-to-accounts-receivable-must-include-a-customer-on-that-split-line>).

No Intuit source says what an IIF journal import does with an A/R or A/P line that has no NAME (refuses it, imports it, or asks). So the books write no NAME (writing names would need the customer and vendor lists declared too, with a name unique across them) and the package README names the entries with an A/R or A/P line, so the CPA can add the name in QuickBooks if it asks (rules.md B-34 rule 7).

## How long a class's name may be

Searched 2026-10-22. Intuit's limits article (above) has no row for a class; its `!CLASS` header help states none. Intuit's .NET SDK reference gives QuickBooks Desktop ("QBW") 31 characters for the names of its other lists (`Account.Name`, above; `Item.Name`, `Term.Name`, `JobType.Name`, `CustomerType.Name`), but for a class its only page, `Department.Name` (<https://static.developer.intuit.com/sdkdocs/qbv3doc/ippdotnetdevkitv3/html/d658b447-b3f9-26f1-3a65-e213cfe3c379.htm>, retrieved 2026-10-22, SHA-256 of the retrieved page `9a33e3cbed97cea28558c8530b6d4950b54346ab8f8581897f1a333b072b026d`), describes "User recognizable name for the Class" with "QBW: 1024": a page for departments (a QuickBooks Online list) whose wording names classes, so not taken as Desktop's class limit.

### Intuit's qbXML Onscreen Reference: `ClassAdd`
Source: the QuickBooks Desktop SDK's Onscreen Reference (OSR), <https://static.developer.intuit.com/qbSDK-current/common/newosr/index.html>, whose `ClassAdd` request page is drawn from <https://static.developer.intuit.com/qbSDK-current/common/newosr/qbsdk/json/ClassAddRq.json>  
Retrieved: 2026-10-22  
SHA-256 of the retrieved `ClassAddRq.json`: `760505a11638ccdb3ea30a9714fa02651509660291ae4eff09e20d4942a310de`

The request's `Name` element (required, a string, `"xmlNameHtml" : "Class-Name.html"`) carries the OSR's length limits per edition:

> "maxCA" : 31, ... "maxUK" : 31, "maxAU" : 31, ... "maxDT" : 31, ... "maxOE" : 100

So 31 characters for a class's name in every QuickBooks Desktop edition the OSR covers (the US desktop, Canada, the UK and Australia), 100 only in Online Edition ("OE"). The same reference's `AccountAddRq.json` (retrieved the same day, SHA-256 `856c64ea3aeea8229692f98cdcfe32278cc9070c56bff7eed74d9e53702c3a40`) gives an account's `Name` the same figures (31 in each Desktop edition, "maxOE" : 100), matching the limits article's Account Name 31 and the .NET reference's "QBW: Max=31", so the fields read as Desktop's own limits.

Not Intuit, for the record: Conductor's QuickBooks Desktop API reference (a service over QuickBooks Desktop's own SDK, which mirrors the OSR), "Create a class", <https://docs.conductor.is/api-ref/qbd/classes/create.md>, retrieved 2026-10-22, SHA-256 of the retrieved file `8588c0ffaa7d5082abd0329734a00ee42179894109ec7572a66d7f844e908d89`, of a class's `name`:

> The case-insensitive name of this class. Not guaranteed to be unique because it does not include the names of its hierarchical parent objects like `fullName` does.

> Maximum length: 31 characters.

So the books hold a class's NAME to 31 characters, as an account's own level (Intuit's own limit for a class, above), and keep no two alike ignoring case (Conductor: case-insensitive; rules.md B-34 rule 8).

## What QuickBooks does with an over-long name (not Intuit)
Source: Thomson Reuters, Accounting CS help, "Export payroll journal entries", <https://thomsonreuters.com/en-us/help/accounting-cs/integrate-data-with-other-applications/export-payroll-journal-entries>  
Retrieved: 2026-10-20

Cited (not quoted), a QuickBooks integrator's documentation, not Intuit's: QuickBooks limits an account's name to 31 characters; importing a chart of accounts IIF file cuts a longer name to 31, but importing a journal IIF file does not, so the journal's accounts no longer match and QuickBooks creates a new (duplicate) bank account for each it doesn't recognize. It advises names of 31 characters or fewer. This is why the books cut each level themselves: the ACCNT line and every TRNS and SPL line then carry the same name.

## Double quotes in an IIF field (not Intuit)
Intuit's own journal sample (above) wraps fields holding a comma in double quotes, as a spreadsheet writes tab-delimited text, so QuickBooks' reader treats a double quote as more than a character. Two non-Intuit sources say so outright:

- Reckon, "Importing the IIF file in Reckon Accounts Business, changes the Inch “ symbol to Feet ‘ symbol", <https://help.reckon.com/article/1ap8nwl0v2-kba-5589-importing-the-iif-file-in-reckon-accounts-business-changes-the-inch-symbol-to-feet-symbol> (retrieved 2026-10-20, no date given). Cited (not quoted): Reckon Accounts Business, which imports IIF files, changes each double quote to a single one by design, because many file formats treat the double quote as a special character.
- Jonathan Clark, interchange-users mailing list, 28 July 2003, <https://www.interchangecommerce.org/pipermail/interchange-users/2003-July/034578.html>:

  > From what we can tell, QB uses double quotation marks (") as text delimiters when there are certain characters in the field.

  > For imports, you will break the import if you have double quotes within your text

So the books write a double quote as a single one, listed like any other change (rules.md B-34).

## Not found in any Intuit source

Searched 2026-10-13 and 2026-10-20 (the help articles above and every document in the IIF Import Kit and journal sample); none states:

- an escaping mechanism for tabs, line breaks, or double quotes inside a field (no escape syntax is documented; the only safe reading is that fields must not contain tabs or line breaks, and, from the reports above, double quotes);
- an explicit sentence that a transaction's TRNS and SPL amounts must sum to zero (every example does; the sign rule implies it);
- the allowed number of decimal places, or whether thousands separators or currency symbols are accepted in AMOUNT;
- the character encoding beyond "ASCII text" (so the books write plain ASCII, transliterating anything else and listing what changed: rules.md B-34, DECISION D-36);
- whether a four-digit year is accepted everywhere despite the "MM/DD/YY" wording (Intuit's 2007+ example uses one);
- what QuickBooks Desktop does on import with an account name, account number or memo longer than its limits (only the non-Intuit note above, for names), how many levels of subaccounts it allows, or a limit on a whole NAME with its parents;
- how long a journal entry's number (DOCNUM) may be (the limits article has no row for it; the books' entry ids are a few digits);
- whether QuickBooks matches account names or numbers ignoring case (the books keep no two alike even ignoring case);
- what an IIF journal import does with a line on accounts receivable or payable that names no customer or vendor (above).
