# QuickBooks Online bank and credit-card CSV upload format
Governs `ledger_import::bank_csv` (the QBO-style bank/card CSV parser). Intuit's help articles are © Intuit Inc., not public domain, so per the vendoring rules (docs/spec/README.md) they are cited, with short quotes only; everything not inside a `>` blockquote is our description of the page, not Intuit's words. Intuit states two layouts: 3 columns (Date, Description, Amount) and 4 columns (Date, Description, Credit, Debit). The pages were read in a browser; the hashes are of the HTML the browser fetched at retrieval (a dynamic page, so a later fetch will differ).

## Format CSV files in Excel to get bank transactions into QuickBooks (US, updated 8/24/2026)
Source: <https://quickbooks.intuit.com/learn-support/en-us/help-article/bank-transactions/format-csv-files-excel-bank-transactions-quickbooks/L4BjLWckq_US_en_US>  
Retrieved: 2026-10-13  
SHA-256 of the retrieved file (`L4BjLWckq_US_en_US.html`, 533,442 bytes): `cbdd8c42ddda944b639db450b0a5c7986c17516c7e699612bea24b950e115bd0`

**Layouts**

> You can upload CSVs with either the 3-column or 4-column format to QuickBooks.

Cited (not quoted): the 3-column layout's columns are Date, Description, Amount; the 4-column layout's columns are Date, Description, Credit, Debit, in that order.

**Sign and column meaning (from Intuit's example tables, transcribed cell by cell)**

3-column example (header row, then two rows):

| Date | Description | Amount |
|---|---|---|
| 1/1/2018 | Example of a payment | -100.00 |
| 1/1/2018 | Example of a deposit | 200.00 |

4-column example:

| Date | Description | Credit | Debit |
|---|---|---|---|
| 1/1/2018 | Example of a payment | 100.00 | |
| 1/1/2018 | Example of a deposit | | 200.00 |

Cited (not quoted): in the 3-column layout a payment (money out) is negative and a deposit (money in) positive. In the 4-column layout Intuit's example puts the payment in the **Credit** column and the deposit in the **Debit** column, each amount positive, the other cell empty. The page has no sentence stating which of Credit/Debit means money in; the example is the only evidence.

**Header rules**

Cited (not quoted): remove the word "amount" from Credit/Debit headers, so they read only "Credit" and "Debit". The page does not say whether a header row is required or optional; both examples have one.

**Zeros**

> Remove any zeros (0) from the file. Leave those cells blank.

**Description column**

Cited (not quoted): correct transactions whose Description cell shows numbers.

**Dates**

Cited (not quoted): keep every date in one format; Intuit recommends dd/mm/yyyy (its own examples are written 1/1/2018, which reads the same either way). A day-of-week suffix in the date cell (its example: 20/11/2018 TUE) must be split off and removed. No list of accepted date formats is given.

**Credit card files**

Cited (not quoted): card CSVs may show signs reversed relative to a bank account (payments to the card can appear as negative deposits); upload them into the credit card account, not a bank account.

**Platform note**

Cited (not quoted): Mac users must save as a Windows CSV file.

## Manually upload transactions into QuickBooks Online (US, updated 8/24/2026)
Source: <https://quickbooks.intuit.com/learn-support/en-us/help-article/import-transactions/manually-upload-transactions-quickbooks-online/L0rE9OXBz_US_en_US>  
Retrieved: 2026-10-13  
SHA-256 of the retrieved file (`L0rE9OXBz_US_en_US.html`): `b32d52c6efe6e4e6dced48876cc93b351ac66168bf2f80e0bedef27b4b4337d7`

**File limits and columns**

> Your upload file must be in English and 350 KB or less.

Cited (not quoted): each file needs either 3 columns (Date, Description, Amount) or 4 (Date, Description, Credit, Debit); cells in Amount, Credit, or Debit holding only zero are left blank; dates in one format, dd/mm/yyyy recommended. During upload the user matches the file's columns to QuickBooks fields on screen (so QBO does not rely on header text or position alone).

## Fix bank upload errors (US, updated 8/4/2026)
Source: <https://quickbooks.intuit.com/learn-support/en-us/help-article/bank-transactions/fix-bank-upload-errors-quickbooks-online/L6FBh3zDC_US_en_US>  
Retrieved: 2026-10-13  
SHA-256 of the retrieved file (`L6FBh3zDC_US_en_US.html`): `d730b312cd5b0583f9e6947b30e4bca0a9be8f5935363c6e3f96d264d19f784c`

**Amount size limit**

Cited (not quoted): the error "The uploaded file contains invalid transaction amount information." occurs when a transaction has 12 or more digits; QuickBooks cannot upload amounts that large.

## Not found in any Intuit source

Searched 2026-10-13 (the three US articles above; Intuit's other regional editions were not read); none states:

- whether thousands separators (`1,234.56`) are accepted in amount cells;
- whether currency symbols (`$`) are accepted;
- whether parentheses for negatives (`(100.00)`) are accepted;
- whether the 4-column order may be Debit before Credit, or other header names (Expense/Income, Withdrawal/Deposit) are accepted;
- an exhaustive list of accepted date formats, or how QBO disambiguates `1/2/2018`;
- whether a header row is mandatory.

Rules wanderbooks applies on these points are CONVENTION or CHOICE, not Intuit's.
