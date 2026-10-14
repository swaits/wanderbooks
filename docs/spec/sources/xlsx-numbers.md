# How spreadsheet numbers are stored: Office Open XML and Excel's precision
Governs `ledger_import::xlsx` (XLSX report output). ECMA-376 and Microsoft's pages are copyrighted (© Ecma International; © Microsoft), not public domain, so per the vendoring rules (docs/spec/README.md) they are cited, with short quotes only; everything not inside a `>` blockquote is our description. Summary: a number cell is a `c` element whose type `t` defaults to `n`; its value sits as text in the `v` element, whose schema type is `ST_Xstring` (not `xsd:double`); the number's value space defaults to IEC 60559 double precision, i.e. XML Schema `double`, 53-bit mantissa. Excel itself keeps 15 significant digits.

## ECMA-376 Part 1, 5th edition (December 2016), Office Open XML File Formats — Fundamentals and Markup Language Reference
Source: <https://ecma-international.org/wp-content/uploads/ECMA-376-1_5th_edition_december_2016.zip>  
Retrieved: 2026-10-13  
SHA-256 of the retrieved file (`ECMA-376-1_5th_edition_december_2016.zip`): `9d0bcad9cf06054785b03762fcfadbf6bab7e54a5f9d69434e34b7fd464d4129`  
SHA-256 of `OfficeOpenXML-XMLSchema-Strict.zip` → `sml.xsd` inside it: `dc4b61faf2b62d2caf875faf40bb14a8f3c0cf3dc52763a2db394a28e79634b4`

**§18.3.1.4 c (Cell): the `t` attribute**

Cited (not quoted): `t` (Cell Data Type) is an `ST_CellType` enumeration. The schema (§A.2, `CT_Cell` in `sml.xsd`) declares it with a default:

```xml
<xsd:attribute name="t" type="ST_CellType" use="optional" default="n"/>
```

**§18.18.11 ST_CellType (Cell Type)**

> n (Number) Cell containing a number.

Cited (not quoted): the other values are `b` boolean, `d` ISO 8601 date, `e` error, `inlineStr` inline rich string, `s` shared string, `str` formula string.

**§18.3.1.96 v (Cell Value)**

Cited (not quoted): `v` holds the cell's value directly unless the cell is a shared string (then it is an index into the shared string table); for a formula cell it holds the last calculated result. Its content type is `ST_Xstring` (§22.9.2.19); the schema line is:

```xml
<xsd:element name="v" type="s:ST_Xstring" minOccurs="0" maxOccurs="1"/>
```

Cited (not quoted): the section's own example writes a number cell as `<c r="B4"><v>360</v></c>` (no `t`, so type `n`).

**§18.17.2.1 Constants: precision of cell values**

> The default is therefore 53-bits of mantissa precision.

Cited (not quoted): for cell formulas and values the default precision is that of the XML Schema `double` type (http://www.w3.org/TR/xmlschema-2/#double); an application may state another precision in the Additional Characteristics part; a compliant consumer must parse numbers of arbitrary precision without error.

**§18.17.5.2 Precision: the value space**

Cited (not quoted): numbers range over values (-1)^s × m × 2^n, with s 0 or 1, 0 ≤ m < 2^53, and -1074 ≤ n ≤ 971; a note says this is patterned after the IEC 60559 double-precision 64-bit type. The characteristics markup (§22.7) may replace this default value space for a workbook. §18.17.2 adds that every arithmetic term is a number in the workbook's value space, by default IEC 60559 double precision.

## Microsoft Support: Excel specifications and limits
Source: <https://support.microsoft.com/en-us/office/excel-specifications-and-limits-1672b34d-7043-467e-8e27-269d656771c3> (redirects to <https://support.microsoft.com/en-us/excel/excel-specifications-and-limits>)  
Retrieved: 2026-10-13  
SHA-256 of the retrieved file (`excel-specifications-and-limits.html`): `2bc3c02cae4a23c5914d6083313166caa8529d4462b85e51f7a121de022b509f`

**Calculation specifications and limits (table rows, transcribed)**

| Feature | Maximum limit |
|---|---|
| Number precision | 15 digits |
| Smallest allowed negative number | -2.2251E-308 |
| Smallest allowed positive number | 2.2251E-308 |
| Largest allowed positive number | 9.99999999999999E+307 |
| Largest allowed negative number | -9.99999999999999E+307 |
| Largest allowed positive number via formula | 1.7976931348623158e+308 |
| Largest allowed negative number via formula | -1.7976931348623158e+308 |

Cited (not quoted): the page applies to Excel for Microsoft 365, 2024, 2021, 2019, 2016.

## Microsoft Learn: Floating-point arithmetic may give inaccurate results in Excel
Source: <https://learn.microsoft.com/en-us/troubleshoot/microsoft-365-apps/excel/floating-point-arithmetic-inaccurate-result>  
Retrieved: 2026-10-13  
SHA-256 of the retrieved file (`floating-point-arithmetic-inaccurate-result.html`): `db89ffae703bb93f096add797db58d1a91a3fe0e0934d95f6f963e5a4cfe0b33`

**15 digits**

> it can only do so within 15 digits of precision.

Cited (not quoted): Excel was designed around IEEE 754 for storing and calculating floating-point numbers; it stores numbers from about 2.2250738585072E-308 to 1.79769313486232E+308 within 15 digits of precision; it does not implement denormalized numbers, infinities, or NaN (it shows errors instead).

## Consequence for money (derived, not quoted)

A cent amount written as a numeric cell is a binary double, not a decimal: 0.10 is not exactly representable. With at most 15 significant digits honored by Excel, a cent-precise amount stays exact on display only up to 13 digits before the decimal point (9,999,999,999,999.99). Money in wanderbooks stays `Decimal`; the XLSX file is a presentation of it, never a source to read money back from.
