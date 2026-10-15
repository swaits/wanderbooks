# Avery 5160 label sheet geometry
Governs `ledger_import::labels` (`LabelSheet::Avery5160`, printable asset-tag sheets). Avery's pages and templates are © Avery Products Corporation, not public domain, so per the vendoring rules (docs/spec/README.md) they are cited, with short quotes only; everything not inside a `>` blockquote is our description, not Avery's words. Avery's pages are dynamic (Next.js), so the hashes are of the files as fetched at retrieval and a later fetch will differ.

## Avery Template 5160 (product template page)
Source: <https://www.avery.com/templates/5160>  
Retrieved: 2026-10-15  
SHA-256 of the retrieved file (`templates_5160.html`, 90,873 bytes): `a274842ad34b3cb83e3129306df8b4f4ff51eb520c96cc263d96eff7b15c0a95`

> 1" x 2-5/8"

> 30 per sheet

The page links Avery's blank downloadable template for 5160 as `U-0087-01` (PDF, Word, Illustrator, InDesign).

## Blank labels, 1" x 2-5/8" (Presta 94200; lists 5160 as compatible)
Source: <https://www.avery.com/blank/labels/94200>  
Retrieved: 2026-10-15  
SHA-256 of the retrieved file (`blank_labels_94200.html`, 274,099 bytes): `c4e7c7eb3428654b1244ed04da5f52ef7668c14d6703f971ea7d0be7daa95b17`

Avery's specification table on this page (key and value, transcribed as given):

| Key | Value |
|---|---|
| Labels per Sheet | 30 labels per sheet |
| Size | 1" x 2-5/8" |
| Print to the edge | No |
| Sheet Size | 8-1/2" x 11" |
| Margin (Top and Bottom) | 0.5" |
| Margin (Left and Right) | 0.1875" |
| Corner Shape | Rounded |

The page's `compatibleSkus` field lists `5160` first.

## Avery 5160 blank PDF template (`U-0087-01.pdf`)
Source: the "PDF" download on <https://www.avery.com/templates/5160> (a signed S3 link: `https://s3.amazonaws.com/avery.dpp.projects.s3uspdownloadables/US_en/Downloadables/pdf/U-0087-01.pdf`, served as `Avery5160AddressLabels.pdf`)  
Retrieved: 2026-10-15  
SHA-256 of the retrieved file (`U-0087-01.pdf`, 675,971 bytes): `486b464c0380d41225f752dc834c5d7e6362364c1454d7ed92a77fc4164d0466`

One US Letter page (612 x 792 pt). We read the cell outlines from its content stream (PDF points, origin bottom-left): 30 rounded rectangles, left edges at x = 13.5, 211.5, 409.5 pt; bottom edges at y = 36, 108, ..., 684 pt (step 72); each 72 pt tall with 7.2 pt (0.1") corner radii. The outline is drawn 189.36 pt wide (2.63"); the stated label width is 2-5/8" (189 pt), and we use the stated size.

## What wanderbooks uses
Derived from the figures above (1 in = 72 pt):

- Page: 8.5" x 11" (US Letter), portrait.
- Label: 2.625" wide x 1" tall; 3 columns x 10 rows = 30 per sheet.
- Top margin 0.5" (the template's top row spans y = 756..684 pt, i.e. 36 pt below the top edge); bottom margin 0.5" (11 - 0.5 - 10 x 1 = 0.5).
- Side margin 0.1875" (13.5 pt).
- Column pitch 2.75" (211.5 - 13.5 = 198 pt), so the horizontal gap is 0.125" (8.5 - 2 x 0.1875 - 3 x 2.625 = 0.25, over two gaps).
- Row pitch 1" (72 pt): no vertical gap.
