# Entity classification: which return a business files

Governs `ledger_config`'s reading of `[company] tax_classification`.
Fetched 2026-10-14.

## Treas. Reg. §301.7701-3 (eCFR, via law.cornell.edu/cfr/text/26/301.7701-3)

**(a) In general**

> A business entity that is not classified as a corporation under
> § 301.7701-2(b) (1), (3), (4), (5), (6), (7), or (8) (an eligible
> entity) can elect its classification for federal tax purposes as
> provided in this section. An eligible entity with at least two members
> can elect to be classified as either an association (and thus a
> corporation under § 301.7701-2(b)(2)) or a partnership, and an eligible
> entity with a single owner can elect to be classified as an association
> or to be disregarded as an entity separate from its owner.

**(b)(1) Domestic eligible entities**

> Except as provided in paragraph (b)(3) of this section, unless the
> entity elects otherwise, a domestic eligible entity is— (i) A
> partnership if it has two or more members; or (ii) Disregarded as an
> entity separate from its owner if it has a single owner.

## Instructions for Schedule C (Form 1040) (2025), "Single-member limited liability company (LLC)"

> Generally, a single-member domestic LLC is not treated as a separate
> entity for federal income tax purposes. If you are the sole member of a
> domestic LLC, file Schedule C (or Schedule E or F, if applicable) unless
> you have elected to treat the domestic LLC as a corporation.

## The returns (titles, from irs-tax-lines.md)

- Form 1065, U.S. Return of Partnership Income: a partnership.
- Form 1120-S, U.S. Income Tax Return for an S Corporation: an S
  corporation.
- Form 1120, U.S. Corporation Income Tax Return: a corporation (an
  association included) that is not an S corporation.

## Derived (wanderbooks)

| classification says | return |
|---|---|
| partnership | Form 1065 |
| S corporation (S corp, S-corp) | Form 1120-S |
| C corporation; corporation, corp, or association alone | Form 1120 |
| disregarded entity, sole proprietor(ship), single-member | Schedule C |

A phrase naming two returns ("corporation or S corporation") and anything
else names no return wanderbooks can be sure of, so it is
refused rather than guessed.
