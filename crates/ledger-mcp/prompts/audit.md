Audit the books for scope {{scope}} (month, annual, cpa_handoff or full). Treat the books as something to disprove: look for what is wrong.

1. books.verify: the chain, the replay, the database and every stored file.
2. books.control_check: the receivable and payable controls agree with their documents.
3. books.digest for the scope's period, and read every posted entry in it against its evidence: books.search_entries, books.get_entry, books.get_receipt. An entry that needs a receipt and has none, or whose receipt shows another amount, date or party, is a finding.
4. books.trial_balance, books.profit_and_loss and books.balance_sheet for the period; compare them with the previous period and explain each large movement.
5. Report findings to the human as a list: what, where (entry or account), why it matters, and the tool that fixes it. Fix nothing without the human's confirmation; corrections are reversing entries, not edits.
