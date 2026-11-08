# Claims: revenue

Wanderbooks' claims for bookspec area **revenue**: the engine's pick for each rule, where it enforces it, and which tests check it. The accounting text lives in bookspec's records (`rules/revenue/`).

Rules: B-17.

---

## B-17 Platform payouts are recorded gross when the business is the principal

**bookspec:** `BS-revenue-1`

**Claims:** yes

**Enforced:** `payouts::payout_plan`. **Checked:**
`a_payout_is_grossed_up_to_revenue_and_fees`.

### Product rules

**WB-revenue-1** `books.record_payout` is the tool that records a
platform payout: it takes the gross, the platform's fee and the deposit
account and posts the gross presentation of BS-revenue-1 in one call.

**Enforced:** `payouts::payout_plan`, `books.record_payout`. **Checked:**
`a_payout_is_grossed_up_to_revenue_and_fees`.
