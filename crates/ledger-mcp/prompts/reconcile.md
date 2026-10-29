Reconcile account {{account}} for {{month}} against the bank's statement.

1. Call books.start, then books.reconciliations to see what is already reconciled for this account.
2. Ask the human for the statement (the period's opening and closing balances and the statement file). Upload it as books.handbook section files-in-out says; never paste a file's contents.
3. Record it with books.record_statement. If the statement's opening balance is not the last reconciled balance, stop and find out why before going on.
4. Mark each transaction that appears on the statement as cleared with books.mark_cleared, matching by amount and date. Transactions on the statement with no entry are drafted (books.draft_entry), not forced.
5. Run books.reconcile for the statement. It succeeds only when the cleared items agree with the statement's closing balance exactly. If it does not, list the difference and the candidates (uncleared entries, statement lines with no entry); do not plug it.
6. Show the human the result with books.show, and offer books.digest.

A mistaken reconciliation is voided with books.void_reconciliation, never edited.
