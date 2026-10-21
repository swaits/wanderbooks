# Partnerships: shares of income and capital accounts
Governs `CloseTo::Members`, the members' equity statement, and the K-1 worksheet's capital accounts on the tax basis (`ledger_core::tax_capital`, rules.md B-36). Partners' shares come from the partnership agreement (IRC §704(a)); a partner's capital account is one running balance of what they contributed, their allocations of income and loss, and what was distributed to them (Treas. Reg. §1.704-1(b)(2)(iv)(b)); when a partner's interest changes during the year, the distributive shares take the varying interests into account, period by period (IRC §706(d)(1), Treas. Reg. §1.706-4(a)); Form 1065 reports each partner's capital account on the tax basis (Schedule K-1 item L) and the changes in them (Schedule M-2). No authority prescribes how to round a share to the cent: that rule is the owner's decision (largest remainder, 2026-10-13), counted from the company's first close (D-10, 2026-10-16).

## 26 U.S.C. §704(a)-(b)
Source: <https://www.law.cornell.edu/uscode/text/26/704>  
Retrieved: 2026-10-13  
SHA-256 of the retrieved file (`usc704.txt`): `e596d36039cb451fe9c5bef5eb6b4c36d2451ab83b06e642a8bf44f3697cf3ee`

> A partner’s distributive share of income, gain, loss, deduction, or credit shall, except as otherwise provided in this chapter, be determined by the partnership agreement.

## Instructions for Form 1065 (2025)
Source: <https://www.irs.gov/instructions/i1065>  
Retrieved: 2026-10-13  
SHA-256 of the retrieved file (`i1065.txt`): `364ec05f6a688447def50169604ec0aa976edb2532b110e2736759213a02da78`

**Allocating among partners**

> Allocate shares of income, gain, loss, deduction, or credit among the partners according to the partnership agreement for sharing income or loss generally.

**Schedule M-2**

> Show what caused changes during the tax year in the partners’ tax-basis capital accounts.

**Rounding on the return (not in the books)**

> The partnership may enter decimal points and cents when preparing its return. However, it should round off cents to whole dollars on its return, forms, and schedules to make completing its return easier. The partnership must either round off all amounts on the return to whole dollars, or use cents for all amounts. To round, drop amounts under 50 cents and increase amounts from 50 to 99 cents to the next dollar. For example, $8.40 rounds to $8 and $8.50 rounds to $9. If two or more amounts are added to figure the amount to enter on a line, include cents when adding the amounts and round off only the total.

## Treas. Reg. §1.704-1(b)(2)(iv)(b): what a capital account carries
Source: <https://www.ecfr.gov/api/renderer/v1/content/enhanced/current/title-26?part=1&section=1.704-1>  
Retrieved: 2026-10-20  
SHA-256 of the retrieved file (`ecfr17041.html`): `0eafa6f27d7df3a3cc93ebad7cbdc0c37a49b055c53a43d5e5e8efa29f80beeb`

> (b) Basic rules. Except as otherwise provided in this paragraph (b)(2)(iv), the partners' capital accounts will be considered to be determined and maintained in accordance with the rules of this paragraph (b)(2)(iv) if, and only if, each partner's capital account is increased by (1) the amount of money contributed by him to the partnership, (2) the fair market value of property contributed by him to the partnership (net of liabilities that the partnership is considered to assume or take subject to), and (3) allocations to him of partnership income and gain (or items thereof), including income and gain exempt from tax and income and gain described in paragraph (b)(2)(iv)(g) of this section, but excluding income and gain described in paragraph (b)(4)(i) of this section; and is decreased by (4) the amount of money distributed to him by the partnership, (5) the fair market value of property distributed to him by the partnership (net of liabilities that such partner is considered to assume or take subject to), (6) allocations to him of expenditures of the partnership described in section 705 (a)(2)(B), and (7) allocations of partnership loss and deduction (or item thereof), including loss and deduction described in paragraph (b)(2)(iv)(g) of this section, but excluding items described in (6) above and loss or deduction described in paragraphs (b)(4)(i) or (b)(4)(iii) of this section; and is otherwise adjusted in accordance with the additional rules set forth in this paragraph (b)(2)(iv). For purposes of this paragraph, a partner who has more than one interest in a partnership shall have a single capital account that reflects all such interests, regardless of the class of interests owned by such partner (e.g., general or limited) and regardless of the time or manner in which such interests were acquired.

## 26 U.S.C. §706(d)(1): a change in a partner's interest
Source: <https://www.law.cornell.edu/uscode/text/26/706>  
Retrieved: 2026-10-20  
SHA-256 of the retrieved file (`usc706.html`): `d716b72177fb0f8e7ae0f5b98faf63d6e40c444f7e9368443060042b761fd53c`

> (d) Determination of distributive share when partner’s interest changes (1) In general Except as provided in paragraphs (2) and (3), if during any taxable year of the partnership there is a change in any partner’s interest in the partnership, each partner’s distributive share of any item of income, gain, loss, deduction, or credit of the partnership for such taxable year shall be determined by the use of any method prescribed by the Secretary by regulations which takes into account the varying interests of the partners in the partnership during such taxable year.

## Treas. Reg. §1.706-4(a): varying interests, segment by segment
Source: <https://www.ecfr.gov/api/renderer/v1/content/enhanced/current/title-26?part=1&section=1.706-4>  
Retrieved: 2026-10-20  
SHA-256 of the retrieved file (`ecfr17064.html`): `e7b07463190db6a610dee37660efefdd73be81b21efd81517dc714dcd72c4277`

**§1.706-4(a)(1)**

> (1) Variations subject to this section. Except as provided in paragraph (a)(2) of this section, this section provides rules for determining the partners' distributive shares of partnership items when a partner's interest in a partnership varies during the taxable year as a result of the disposition of a partial or entire interest in a partnership as described in § 1.706-1(c)(2) and (3), or with respect to a partner whose interest in a partnership is reduced as described in § 1.706-1(c)(3), including by the entry of a new partner (collectively, a “variation”).

**§1.706-4(a)(2), its last sentence**

> In all cases, all partnership items for each taxable year must be allocated among the partners, and no partnership items may be duplicated, regardless of the particular provision of section 706 (or other Code section) which applies, and regardless of the method or convention adopted by the partnership.

**§1.706-4(a)(3)(vii), its first three sentences**

> (vii) Seventh, apportion the partnership's items for the year among its segments. The partnership shall determine the items of income, gain, loss, deduction, and credit of the partnership for each segment. In general, a partnership shall treat each segment as though the segment were a separate distributive share period.

## Partner's Instructions for Schedule K-1 (Form 1065) (2025)
Source: <https://www.irs.gov/instructions/i1065sk1>  
Retrieved: 2026-10-20  
SHA-256 of the retrieved file (`i1065sk1.html`): `d7d6298286601fd95ab8e445c10fd62499ea1e37f8e5c3531f8e1783862d8d03`

**Item L, its first paragraph** (the partner's capital account, on the tax basis; the K-1 worksheet carries the book capital account and, beside it, the tax-basis one as far as the books know it, rules.md B-36)

> The partnership must report your beginning capital account and ending capital account for the year using the tax-basis method, including the amount of capital you contributed to the partnership during the year, your share of the partnership’s current-year net income or loss as computed for tax purposes, any withdrawals and distributions made to you by the partnership, and any other increases or decreases to your capital account determined in a manner generally consistent with figuring the partner’s adjusted tax basis in its partnership interest (without regard to partnership liabilities), taking into account the rules and principles of sections 705, 722, 733, and 742. See the Instructions for Form 1065 for more details.

**Basis Limitations, its third paragraph of five** (where "based on the partnership's books and records" comes from: not Item L)

> Although the partnership provides an analysis of the partner’s capital account in item L of Schedule K-1, that information is based on the partnership’s books and records and can’t be used to figure the partner’s adjusted basis.

## Instructions for Form 1065 (2025): Item L, the tax-basis method
Source: <https://www.irs.gov/instructions/i1065>  
Retrieved: 2026-10-20  
SHA-256 of the retrieved file (`i1065.html`): `3485e936c1f6532a3a8efda78d45db38c70892abe9c532704d605a4bc45714fe`

Under "Item L. Partner’s Capital Account Analysis" (Specific Instructions, Schedule K-1 only):

**Tax-basis method.**

> Figure each partner’s capital account for the partnership’s tax year using the transactional approach, discussed below, for the tax-basis method.

**How to report partnership events or transactions.** (first two sentences)

> If you’re uncertain how to report a partnership event or transaction, you should account for the event or transaction in a manner generally consistent with figuring the partner’s adjusted tax basis in its partnership interest (without regard to partnership liabilities), taking into account the rules and principles of sections 705, 722, 733, and 742 and by reporting the amount on the line for other increase (decrease). The partner’s ending capital account as reported using the tax-basis method in item L might not equal the partner’s adjusted tax basis in its partnership interest.

**Beginning capital account.**

> Enter the partner’s ending capital account as determined for last year on the line for beginning capital account. If a partner joined the partnership through a contribution to the partnership this year, enter zero as the partner’s beginning capital account.

**Capital contributed during the year.**

> On the line “Capital contributed during the year,” enter the amount of cash plus the adjusted tax basis of all property contributed by the partner to the partnership during the year. The amount you enter on this line should be reduced by any liabilities assumed by the partnership in connection with, or liabilities to which the property is subject immediately before, the contribution. This amount might be negative.

**Current year net income (loss).**

> On the line “Current year net income (loss),” enter the partner’s distributive share of partnership income and gain (including tax-exempt income) as figured for tax purposes for the year, minus the partner’s distributive share of partnership loss and deductions (including nondeductible, noncapital expenditures) as figured for tax purposes for the year.

**Other increase (decrease).** (first sentence)

> On the line “Other increase (decrease),” enter the sum of all other increases or decreases that affected the partner’s capital account for tax purposes during the year and attach a statement explaining each adjustment.

**Withdrawals and distributions.**

> On the line “Withdrawals and distributions,” enter the amount of cash plus the adjusted tax basis of all property distributed by the partnership to the partner during the year. The amount you enter on this line should be reduced by any liabilities assumed by the partner in connection with, or liabilities to which the property is subject immediately before, the distribution. This amount might be negative.

**Ending capital account.**

> The sum of the amounts shown on the lines in item L above the line “Ending capital account” must equal the amount reported on the line “Ending capital account.” A partner’s ending capital account determined under the tax-basis method may be negative if the sum of a partner’s losses and distributions exceeds the sum of the partner’s contributions and share of income.

Under "Item M. Did the Partner Contribute Property With a Built-in Gain or Loss?":

> A property’s built-in gain is the amount by which the FMV of the property exceeds its adjusted tax basis at the time the property is contributed to the partnership. A property’s built-in loss is the amount by which the FMV of the property is less than its adjusted tax basis at the time the property is contributed to the partnership. Partnerships are required to keep track of this information; see Regulations section 1.704-3.

## Instructions for Form 1065 (2020): the tax-basis method required
Source: <https://www.irs.gov/pub/irs-prior/i1065--2020.pdf>  
Retrieved: 2026-10-20  
SHA-256 of the retrieved file (`i1065--2020.pdf`): `64a48b6e8a9d713eaf641ffdd9a5a09e9811692d84a4be2c45f458e69eb7491a`

Under "Item L. Partner's Capital Account Analysis", from tax year 2020:

> Tax basis method. Figure each partner's capital account for the partnership's tax year using the transactional approach, discussed below, for the tax basis method. If you reported the partner's capital account last year using any other method (for example, GAAP, section 704(b), or other), you must use the tax basis method this year.
