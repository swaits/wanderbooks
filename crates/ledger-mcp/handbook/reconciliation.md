# Reconciliation

- Every month, for each bank, cash, and card account: file the statement (books.file_document category statement), record what it says (books.record_statement), mark the lines it shows (books.mark_cleared), preview books.reconcile with statement_attachment, and confirm with confirmed=true once the difference is exactly zero: that records the reconciliation with its statement.
- Never confirm by forcing the numbers; find the difference.
- books.void_reconciliation (with a reason) undoes a mistake; books.void_statement (with a reason) a mis-recorded statement, to record it again.
- books.digest lists unreconciled_accounts: accounts not reconciled through the end of last month.
