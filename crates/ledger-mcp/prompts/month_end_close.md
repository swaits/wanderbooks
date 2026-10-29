Close the month {{month}} for {{company}}. The close is data-driven: do each step in order, show the human what each one did, and stop at the first one that is not clean.

1. books.start for the rules, then books.conventions with kind close_step: this company's own close steps, in the order it keeps them. Follow them.
2. Reconcile every tracked account for the month (the reconcile prompt, one account at a time); books.start lists the accounts that are behind.
3. Review the financing accounts and the open drafts: books.list_drafts. A draft is posted, with its evidence, or discarded; none may stand in a closed month.
4. Run what the month is owed: books.run_schedules, books.run_recurring and books.run_depreciation, each previewed first.
5. Check the figures: books.trial_balance, books.profit_and_loss and books.balance_sheet for the month; books.control_check for the receivable and payable controls; books.verify for the chain.
6. Only then books.close_period for the month, previewed and confirmed by the human.
7. Offer books.digest of the month.

A closed month is never edited: a correction is a reversing entry in an open month.
