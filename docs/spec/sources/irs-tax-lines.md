# Tax return line items: Schedule C, Form 1065, Form 1120-S, Form 1120
Governs `tax::CATEGORIES` (crates/ledger-core/src/tax.rs) and the tax categories on the starter charts (crates/ledger-import/src/templates.rs). The forms quoted are the 2025 revisions (filed in 2026), the current ones on irs.gov at retrieval. Line numbers move between years; the categories are stable keys, and the mapping at the end must be re-checked against each new revision. Captions are copied from the form's page 1 with dot leaders and the right-hand line-number boxes removed (whitespace normalized); a two-column form layout is given one caption per line.

## Schedule C (Form 1040) 2025, Profit or Loss From Business
Source: <https://www.irs.gov/pub/irs-pdf/f1040sc.pdf>  
Retrieved: 2026-10-13  
SHA-256 of the retrieved file (`f1040sc.pdf`): `ddf401dbe060467d39f90ad2abf645df1de31512821a150dc68a3882bbf19716`

**Part I, Income**

> 1 Gross receipts or sales. See instructions for line 1 and check the box if this income was reported to you on Form W-2 and the “Statutory employee” box on that form was checked
>
> 2 Returns and allowances
>
> 3 Subtract line 2 from line 1
>
> 4 Cost of goods sold (from line 42)
>
> 5 Gross profit. Subtract line 4 from line 3
>
> 6 Other income, including federal and state gasoline or fuel tax credit or refund (see instructions)
>
> 7 Gross income. Add lines 5 and 6

**Part II, Expenses**

> Part II Expenses. Enter expenses for business use of your home only on line 30.
>
> 8 Advertising
>
> 9 Car and truck expenses (see instructions)
>
> 10 Commissions and fees
>
> 11 Contract labor (see instructions)
>
> 12 Depletion
>
> 13 Depreciation and section 179 expense deduction (not included in Part III) (see instructions)
>
> 14 Employee benefit programs (other than on line 19)
>
> 15 Insurance (other than health)
>
> 16 Interest (see instructions): a Mortgage (paid to banks, etc.) b Other
>
> 17 Legal and professional services
>
> 18 Office expense (see instructions)
>
> 19 Pension and profit-sharing plans
>
> 20 Rent or lease (see instructions): a Vehicles, machinery, and equipment b Other business property
>
> 21 Repairs and maintenance
>
> 22 Supplies (not included in Part III)
>
> 23 Taxes and licenses
>
> 24 Travel and meals: a Travel b Deductible meals (see instructions)
>
> 25 Utilities
>
> 26 Wages (less employment credits)
>
> 27a Energy efficient commercial bldgs deduction (attach Form 7205)
>
> b Other expenses (from line 48)
>
> 28 Total expenses before expenses for business use of home. Add lines 8 through 27b

## 2025 Instructions for Schedule C
Source: <https://www.irs.gov/pub/irs-pdf/i1040sc.pdf>  
Retrieved: 2026-10-13  
SHA-256 of the retrieved file (`i1040sc.pdf`): `b5536db91498a450572e37078d7d5ee8fdc400e385d4fee3a26c230a33e399b6`

**Line 6 (other income), in part**

> Report on line 6 business income not reported elsewhere in Part I. Be sure to include amounts from the following. • Finance reserve income. • Scrap sales. • Bad debts you recovered. • Interest (such as on notes and accounts receivable).

**Line 24b (meals), in part**

> Enter your deductible business meal expenses. This includes expenses for meals while traveling away from home for business. Your deductible business meal expenses are a percentage of your actual business meal expenses or standard meal allowance. See Amount of deduction, later, for the percentage that applies to your actual meal expenses or standard meal allowance. In most cases, the percentage is 50%.

**Part V. Other Expenses. Line 48**

> Include all ordinary and necessary business expenses not deducted elsewhere on Schedule C. List the type and amount of each expense separately in the space provided. Enter the total on lines 48 and 27b. Do not include the cost of business equipment or furniture; replacements or permanent improvements to property; or personal, living, and family expenses. Do not include charitable contributions. Also, you can’t deduct fines or penalties paid to a government for violating any law.

**Part V: bad debts**

> Bad debts. Include debts and partial debts from sales or services that were included in income and are definitely known to be worthless. If you later collect a debt that you deducted as a bad debt, include it as income in the year collected.

**Part V: business startup costs**

> Business startup costs. If your business began in 2025, you can elect to deduct up to $5,000 of certain business startup costs. The $5,000 limit is reduced (but not below zero) by the amount by which your total startup costs exceed $50,000. Your remaining startup costs can be amortized over a 180-month period beginning with the month the business began.

## Form 1065 (2025), U.S. Return of Partnership Income, page 1
Source: <https://www.irs.gov/pub/irs-pdf/f1065.pdf>  
Retrieved: 2026-10-13  
SHA-256 of the retrieved file (`f1065.pdf`): `0f19f556e12ef53c41ba27e5930b4373103f3abcf64693c4ea686451b2a8f56f`

**Income**

> Caution: Include only trade or business income and expenses on lines 1a through 23 below. See instructions for more information.
>
> 1a Gross receipts or sales b Less returns and allowances c Balance
>
> 2 Cost of goods sold (attach Form 1125-A)
>
> 3 Gross profit. Subtract line 2 from line 1c
>
> 4 Ordinary income (loss) from other partnerships, estates, and trusts (attach statement)
>
> 5 Net farm profit (loss) (attach Schedule F (Form 1040))
>
> 6 Net gain (loss) from Form 4797, Part II, line 17 (attach Form 4797)
>
> 7 Other income (loss) (attach statement)
>
> 8 Total income (loss). Combine lines 3 through 7

**Deductions (see instructions for limitations)**

> 9 Salaries and wages (other than to partners) (less employment credits)
>
> 10 Guaranteed payments to partners
>
> 11 Repairs and maintenance
>
> 12 Bad debts
>
> 13 Rent
>
> 14 Taxes and licenses
>
> 15 Interest (see instructions)
>
> 16a Depreciation (if required, attach Form 4562)
>
> b Less depreciation reported on Form 1125-A and elsewhere on return
>
> 17 Depletion (Do not deduct oil and gas depletion.)
>
> 18 Retirement plans, etc.
>
> 19 Employee benefit programs
>
> 20 Energy efficient commercial buildings deduction (attach Form 7205)
>
> 21 Other deductions (attach statement)
>
> 22 Total deductions. Add the amounts shown in the far right column for lines 9 through 21
>
> 23 Ordinary business income (loss). Subtract line 22 from line 8

## Instructions for Form 1065 (2025)
Source: <https://www.irs.gov/pub/irs-pdf/i1065.pdf>  
Retrieved: 2026-10-13  
SHA-256 of the retrieved file (`i1065.pdf`): `f4a35a24c7abac486908d3610754397a66b2130f0eb425dda897d78a9d91b786`

**Separately stated items (charitable contributions), in part**

> Items the partnership must state separately that require separate computations by the partners. Examples include expenses incurred for the production of income instead of in a trade or business, charitable contributions, foreign taxes paid or accrued, intangible drilling and development costs, soil and water conservation expenditures, amortizable basis of reforestation expenditures, and exploration expenditures.

**Line 10. Guaranteed Payments to Partners, in part**

> Deduct payments or credits to a partner for services or for the use of capital if the payments or credits are determined without regard to partnership income and are allocable to a trade or business activity.

**Business startup and organizational costs, in part**

> Business startup and organizational costs. Generally, a partnership can elect to deduct a limited amount of startup or organizational costs paid or incurred. Any costs not deducted must be amortized as explained below. See sections 195(b) and 709(b).

> Amortization. Any costs not deducted under the above rules must be amortized ratably over a 180-month period, beginning with the month the partnership begins business. See the Instructions for Form 4562 for details.

**Line 21. Other Deductions**

> Enter the total allowable trade or business deductions that aren’t deductible elsewhere on page 1 of Form 1065. Attach a statement listing by type and amount each deduction included on this line. Examples of other deductions include the following. • Amortization. See the Instructions for Form 4562 for more information. Complete and attach Form 4562 if the partnership is claiming amortization of costs that began during the tax year. • Insurance premiums. • Legal and professional fees. • Supplies used and consumed in the business. • Utilities. • Certain business startup and organizational costs. See Limitations on Deductions, earlier, for more details. • Any net negative section 481(a) adjustment.

## Form 1120-S (2025), U.S. Income Tax Return for an S Corporation, page 1
Source: <https://www.irs.gov/pub/irs-pdf/f1120s.pdf>  
Retrieved: 2026-10-13  
SHA-256 of the retrieved file (`f1120s.pdf`): `c4b7cae7f2283a2e3225523ea001a710109fee34aad909461421025b287cd18d`

**Income**

> Caution: Include only trade or business income and expenses on lines 1a through 22. See the instructions for more information.
>
> 1a Gross receipts or sales b Less returns and allowances c Balance
>
> 2 Cost of goods sold (attach Form 1125-A)
>
> 3 Gross profit. Subtract line 2 from line 1c
>
> 4 Net gain (loss) from Form 4797, Part II, line 17 (attach Form 4797)
>
> 5 Other income (loss) (see instructions—attach statement)
>
> 6 Total income (loss). Add lines 3 through 5

**Deductions**

> 7 Compensation of officers (see instructions—attach Form 1125-E)
>
> 8 Salaries and wages (less employment credits)
>
> 9 Repairs and maintenance
>
> 10 Bad debts
>
> 11 Rents
>
> 12 Taxes and licenses
>
> 13 Interest (see instructions)
>
> 14 Depreciation from Form 4562 not claimed on Form 1125-A or elsewhere on return (attach Form 4562)
>
> 15 Depletion (do not deduct oil and gas depletion)
>
> 16 Advertising
>
> 17 Pension, profit-sharing, etc., plans
>
> 18 Employee benefit programs
>
> 19 Energy efficient commercial buildings deduction (attach Form 7205)
>
> 20 Other deductions (attach statement)
>
> 21 Total deductions. Add lines 7 through 20
>
> 22 Ordinary business income (loss). Subtract line 21 from line 6

## Instructions for Form 1120-S (2025)
Source: <https://www.irs.gov/pub/irs-pdf/i1120s.pdf>  
Retrieved: 2026-10-13  
SHA-256 of the retrieved file (`i1120s.pdf`): `eab655cff9226dbc6d80213350bac050009ba90daf621c182d31c541bc62c3f2`

**Start-up and organizational costs, in part**

> Report the deductible amount of start-up and organizational costs and any amortization on line 20. For amortization that begins during the current tax year, complete and attach Form 4562, Depreciation and Amortization.

**Line 20. Other Deductions, in part**

> Enter the total allowable trade or business deductions that aren’t deductible elsewhere on Form 1120-S, page 1. Attach a statement listing by type and amount each deduction included on this line. Examples of other deductions include the following. • Amortization. See Form 4562, Part VI. • Certain business start-up and organizational costs (discussed earlier). • Insurance premiums. • Legal and professional fees. • Supplies used and consumed in the business. • Travel, meal, and entertainment expenses. Special rules apply (discussed later). • Utilities.

## Form 1120 (2025), U.S. Corporation Income Tax Return, page 1
Source: <https://www.irs.gov/pub/irs-pdf/f1120.pdf>  
Retrieved: 2026-10-13  
SHA-256 of the retrieved file (`f1120.pdf`): `016c7fb4042a565c23a98014b17019af7ddc89e3260dd3f926b84d6688b7be6e`

**Income**

> 1a Gross receipts or sales
>
> b Returns and allowances
>
> c Balance. Subtract line 1b from line 1a
>
> 2 Cost of goods sold (attach Form 1125-A)
>
> 3 Gross profit. Subtract line 2 from line 1c
>
> 4 Dividends and inclusions (Schedule C, line 23)
>
> 5 Interest
>
> 6 Gross rents
>
> 7 Gross royalties
>
> 8 Capital gain net income (attach Schedule D (Form 1120))
>
> 9 Net gain or (loss) from Form 4797, Part II, line 17 (attach Form 4797)
>
> 10 Other income (see instructions—attach statement)
>
> 11 Total income. Add lines 3 through 10

**Deductions**

> 12 Compensation of officers (see instructions—attach Form 1125-E)
>
> 13 Salaries and wages (less employment credits)
>
> 14 Repairs and maintenance
>
> 15 Bad debts
>
> 16 Rents
>
> 17 Taxes and licenses
>
> 18 Interest (see instructions)
>
> 19 Charitable contributions
>
> 20 Depreciation from Form 4562 not claimed on Form 1125-A or elsewhere on return (attach Form 4562)
>
> 21 Depletion
>
> 22 Advertising
>
> 23 Pension, profit-sharing, etc., plans
>
> 24 Employee benefit programs
>
> 25 Energy efficient commercial buildings deduction (attach Form 7205)
>
> 26 Other deductions (attach statement)
>
> 27 Total deductions. Add lines 12 through 26
>
> 28 Taxable income before net operating loss deduction and special deductions. Subtract line 27 from line 11.

## Instructions for Form 1120 (2025)
Source: <https://www.irs.gov/pub/irs-pdf/i1120.pdf>  
Retrieved: 2026-10-13  
SHA-256 of the retrieved file (`i1120.pdf`): `49476a6f69a38a0fdb96fea1d66ddc936404e9b2eebb249da8e20830b0743358`

**Start-up and organizational costs, in part**

> Report the deductible amount of start-up and organizational costs and any amortization on line 26.

---

## Separately stated items on the pass-through returns (Schedule K)
Source: <https://www.irs.gov/pub/irs-pdf/i1065.pdf>  
Retrieved: 2026-10-13  
SHA-256 of the retrieved file (`i1065.pdf`): `f4a35a24c7abac486908d3610754397a66b2130f0eb425dda897d78a9d91b786`

**Portfolio income (Form 1065 instructions)**

> Generally, portfolio income includes all gross income, other than income derived in the ordinary course of a trade or business, that is attributable to interest; dividends; royalties; income from a real estate investment trust (REIT), a regulated investment company (RIC), a REMIC, a common trust fund, a CFC, a QEF, or a cooperative; income from the disposition of property that produces income of a type defined as portfolio income; and income from the disposition of property held for investment.

**Schedule K, line 5, Interest Income (Form 1065 instructions)**

> Enter only taxable portfolio interest on this line.

Source: <https://www.irs.gov/pub/irs-pdf/i1120s.pdf>  
Retrieved: 2026-10-13  
SHA-256 of the retrieved file (`i1120s.pdf`): `eab655cff9226dbc6d80213350bac050009ba90daf621c182d31c541bc62c3f2`

**Separately stated items (Form 1120-S instructions)**

> Items the corporation must state separately that require separate computations by the shareholders. Examples include expenses incurred for the production of income instead of in a trade or business, charitable contributions, foreign taxes paid or accrued, intangible drilling and development costs, soil and water conservation expenditures, amortizable basis of reforestation expenditures, and exploration expenditures.

**Schedule K, line 4, Interest Income (Form 1120-S instructions)**

> Enter only taxable portfolio interest on this line. Taxable interest is interest from all sources except interest exempt from tax and interest on tax-free covenant bonds.

---

## Gains and losses on business property (Form 4797), and tax-exempt income
Governs `form_4797_gain_loss` and `tax_exempt_income` (added in P3, defect 8, 2026-10-20: under `policy.require_tax_category = refuse` every revenue and expense account needs a category that fits it, and the gain or loss account `books.dispose_asset` takes had none). Each file below was retrieved again on 2026-10-20 and is byte for byte the one quoted above (its SHA-256 is the same); only the passages are new. Captions as above.

**Form 1065 (2025), Schedule K** (Source: <https://www.irs.gov/pub/irs-pdf/f1065.pdf>; Retrieved: 2026-10-20; SHA-256 `0f19f556e12ef53c41ba27e5930b4373103f3abcf64693c4ea686451b2a8f56f`)

> 10 Net section 1231 gain (loss) (attach Form 4797)
>
> 18a Tax-exempt interest income
>
> b Other tax-exempt income
>
> c Nondeductible expenses

**Form 1120-S (2025), Schedule K** (Source: <https://www.irs.gov/pub/irs-pdf/f1120s.pdf>; Retrieved: 2026-10-20; SHA-256 `c4b7cae7f2283a2e3225523ea001a710109fee34aad909461421025b287cd18d`)

> 9 Net section 1231 gain (loss) (attach Form 4797)
>
> 16a Tax-exempt interest income
>
> b Other tax-exempt income
>
> c Nondeductible expenses

**Form 1120 (2025), Schedule K item 9 and Schedule M-1 line 7** (Source: <https://www.irs.gov/pub/irs-pdf/f1120.pdf>; Retrieved: 2026-10-20; SHA-256 `016c7fb4042a565c23a98014b17019af7ddc89e3260dd3f926b84d6688b7be6e`)

> 9 Enter the amount of tax-exempt interest received or accrued during this tax year
>
> 7 Income recorded on books this year not included on this return (itemize): Tax-exempt interest $

**Instructions for Form 1065 (2025), Line 6 and Tax-exempt income, in part** (Source: <https://www.irs.gov/pub/irs-pdf/i1065.pdf>; Retrieved: 2026-10-20; SHA-256 `f4a35a24c7abac486908d3610754397a66b2130f0eb425dda897d78a9d91b786`)

> Line 6. Net Gain (Loss) From Form 4797
>
> Caution: Include only ordinary gains or losses from the sale, exchange, or involuntary conversion of assets used in a trade or business activity.

> Tax-exempt income. Don’t include any tax-exempt income on lines 1a through 8. A partnership that receives any tax-exempt income other than interest, or holds any property or engages in any activity that produces tax-exempt income, reports this income on Schedule K, line 18b, and in box 18 of Schedule K-1 using code B.
>
> Report tax-exempt interest income, including exempt-interest dividends received as a shareholder in a mutual fund or other RIC, on Schedule K, line 18a, and in box 18 of Schedule K-1 using code A.

**Instructions for Form 1120-S (2025), Line 4 and Tax-exempt income, in part** (Source: <https://www.irs.gov/pub/irs-pdf/i1120s.pdf>; Retrieved: 2026-10-20; SHA-256 `eab655cff9226dbc6d80213350bac050009ba90daf621c182d31c541bc62c3f2`)

> Line 4. Net Gain (Loss) From Form 4797
>
> Caution: Include only ordinary gains or losses from the sale, exchange, or involuntary conversion of assets used in a trade or business activity.

> Tax-exempt income. Don’t include any tax-exempt income on lines 1a through 5. A corporation that receives any tax-exempt income other than interest or holds any property or engages in any activity that produces tax-exempt income reports this income on Schedule K, line 16b, and in box 16 of Schedule K-1 using code B.
>
> Report tax-exempt interest income, including exempt-interest dividends received as a shareholder in a mutual fund or other regulated investment company, on Schedule K, line 16a, and in box 16 of Schedule K-1 using code A.

**Instructions for Form 1120 (2025), Line 5, in part** (Source: <https://www.irs.gov/pub/irs-pdf/i1120.pdf>; Retrieved: 2026-10-20; SHA-256 `49476a6f69a38a0fdb96fea1d66ddc936404e9b2eebb249da8e20830b0743358`)

> Report tax-exempt interest on Schedule K, item 9. Also, if required, include the same amount on Schedule M-1, line 7, or Schedule M-3 (Form 1120), Part II, line 13, if applicable.

**Instructions for Schedule C (2025), Other Schedules and Forms You May Have To File, in part** (Source: <https://www.irs.gov/pub/irs-pdf/i1040sc.pdf>; Retrieved: 2026-10-20; SHA-256 `b5536db91498a450572e37078d7d5ee8fdc400e385d4fee3a26c230a33e399b6`)

> • Form 4797 to report sales, exchanges, and involuntary conversions (not from a casualty or theft) of trade or business property.

## Mapping (wanderbooks, derived)

**This section is derived, not quoted.** It is wanderbooks' reading of the 2025 forms and instructions above, written 2026-10-13, for the categories in `crates/ledger-core/src/tax.rs`. It is not tax advice; the CPA decides the return. "—" means the category does not apply to that form (per `TaxCategory::forms`). "Not page 1" means the quoted page-1 lines have no place for it; where it goes instead (for example Schedule K) is not quoted in this file and is left to the CPA.

| Category key | Schedule C | Form 1065 | Form 1120-S | Form 1120 |
|---|---|---|---|---|
| `gross_receipts` | 1 | 1a | 1a | 1a |
| `returns_allowances` | 2 | 1b | 1b | 1b |
| `interest_income` | 6 for interest on notes and accounts receivable (instructions, line 6); other interest not determined here | Not page 1: Schedule K line 5, portfolio interest (quoted above) | Not page 1: Schedule K line 4, portfolio interest (quoted above) | 5 |
| `form_4797_gain_loss` | Not on Schedule C: Form 4797 (instructions, quoted above) | 6 (the ordinary part, from Form 4797); net section 1231 gain on Schedule K line 10 (quoted above) | 4 (the ordinary part); net section 1231 gain on Schedule K line 9 (quoted above) | 9 |
| `other_income` | 6 | 7 | 5 | 10 |
| `tax_exempt_income` | — | Not page 1: Schedule K line 18a (interest) or 18b (other) (quoted above) | Not page 1: Schedule K line 16a or 16b (quoted above) | Not page 1: Schedule K item 9 and Schedule M-1 line 7, tax-exempt interest (quoted above) |
| `cost_of_goods_sold` | 4 (from Part III, line 42) | 2 | 2 | 2 |
| `advertising` | 8 | 21 (no advertising line) | 16 | 22 |
| `bad_debts` | 27b via Part V (no line in Part II; instructions list bad debts under Part V) | 12 | 10 | 15 |
| `car_truck` | 9 | 21 | 20 | 26 |
| `charitable_contributions` | Not deductible on Schedule C (Part V: "Do not include charitable contributions") | Not page 1: separately stated (instructions) | Not page 1: separately stated (quoted above) | 19 |
| `commissions_fees` | 10 | 21 | 20 | 26 |
| `contract_labor` | 11 | 21 | 20 | 26 |
| `depreciation` | 13 (tax amount, not book; CPA) | 16a/16c (tax amount; CPA) | 14 (tax amount; CPA) | 20 (tax amount; CPA) |
| `employee_benefits` | 14 | 19 | 18 | 24 |
| `guaranteed_payments` | — | 10 | — | — |
| `insurance` | 15 | 21 (instructions: insurance premiums) | 20 (instructions: insurance premiums) | 26 |
| `interest_expense` | 16a (mortgage paid to banks) or 16b (other) | 15 | 13 | 18 |
| `legal_professional` | 17 | 21 (instructions: legal and professional fees) | 20 (instructions: legal and professional fees) | 26 |
| `meals` | 24b (deductible portion; instructions: in most cases 50%) | 21 | 20 (instructions: travel, meal, and entertainment expenses) | 26 |
| `office_expense` | 18 | 21 | 20 | 26 |
| `officer_compensation` | — | — | 7 | 12 |
| `organization_startup` | 27b via Part V (startup costs; amortization on Form 4562) | 21 (instructions: certain business startup and organizational costs; amortization) | 20 (instructions: report on line 20) | 26 (instructions: report on line 26) |
| `pension_profit_sharing` | 19 | 18 (Retirement plans, etc.) | 17 | 23 |
| `rent_equipment` | 20a | 13 | 11 | 16 |
| `rent_property` | 20b | 13 | 11 | 16 |
| `repairs` | 21 | 11 | 9 | 14 |
| `salaries_wages` | 26 | 9 (other than to partners) | 8 | 13 |
| `supplies` | 22 | 21 (instructions: supplies) | 20 (instructions: supplies) | 26 |
| `taxes_licenses` | 23 | 14 | 12 | 17 |
| `travel` | 24a | 21 | 20 (instructions: travel) | 26 |
| `utilities` | 25 | 21 (instructions: utilities) | 20 (instructions: utilities) | 26 |
| `other_deductions` | 27b via Part V | 21 | 20 | 26 |
| `nondeductible` | No line (Part V: fines or penalties paid to a government not deductible) | No line | No line | No line |

Notes on the derivation:

- Form 1065 has no lines for advertising, car and truck, commissions, contract labor, insurance, legal, meals, office, supplies, travel, or utilities; they fall to line 21. Form 1120-S and Form 1120 likewise lack lines for car and truck, commissions, contract labor, insurance, legal, meals, office, supplies, travel, and utilities, which fall to line 20 and line 26 respectively. Only the items the quoted instructions name are attributed to them in the table.
- Form 1065 line 13 "Rent" and Forms 1120-S line 11 / 1120 line 16 "Rents" do not split equipment from property; Schedule C line 20 does.
- Schedule C has no line for bad debts or for organizational and start-up costs; the Part V instructions quoted above put them in Other expenses (line 48 to line 27b).
- Schedule C line 30 (business use of home) and Form 1065 line 20 / 1120-S line 19 / 1120 line 25 (energy efficient commercial buildings) have no wanderbooks category.
- `form_4797_gain_loss` (2026-10-20) is the book gain or loss on business property. Form 4797 figures the tax gain from the tax basis (book and tax depreciation differ), and splits ordinary gain (recapture) from section 1231 gain; the CPA does both, and the split decides the line (the row above). So, unlike `depreciation` (on page 1 of every return, only its amount the CPA's), the tax summary can't place it: it lists it apart (`form_4797`), at its book amount, with no tax amount and outside income and both nets, on every return alike. A Form 4797 gain or loss never goes on Schedule C (the Schedule C instructions send sales of business property to Form 4797).
- `tax_exempt_income` (2026-10-20) is income at 0%: none of it is taxable, so it appears in the summary's book amounts and not in its tax amounts, the income twin of `nondeductible`. It is for the entity returns; a sole proprietor's tax-exempt interest is not business income.
