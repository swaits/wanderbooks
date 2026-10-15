# Truncated taxpayer identification numbers
Governs what of a TIN the party directory may hold (crates/ledger-core/src/parties.rs): the last four digits, the same part a truncated TIN shows. The full number lives only in the encrypted vault (docs/parties-and-attachments.md), never in the journal.

## Treas. Reg. §301.6109-4(a) and (b)(1)
Source: <https://www.ecfr.gov/current/title-26/part-301/section-301.6109-4>  
Retrieved: 2026-10-15  
SHA-256 of the retrieved file (`ecfr301-6109-4.html`): `45702c366f7f185e41452a32acf685bb5371e00b91bfb6214174ffd153eba949`

> (a) In general — Definition. An IRS truncated taxpayer identification number (TTIN) is an individual's social security number (SSN), IRS individual taxpayer identification number (ITIN), IRS adoption taxpayer identification number (ATIN), or IRS employer identification number (EIN) in which the first five digits of the nine-digit number are replaced with Xs or asterisks. The TTIN takes the same format of the identifying number it replaces, for example XXX-XX-1234 when replacing an SSN, or XX-XXX1234 when replacing an EIN.

> (b) Use of a TTIN. (1) In general. Except as provided in paragraph (b)(2) of this section, a TTIN may be used to identify any person on any statement or other document that the internal revenue laws require to be furnished to another person. Use of a TTIN is permissive and not mandatory.
