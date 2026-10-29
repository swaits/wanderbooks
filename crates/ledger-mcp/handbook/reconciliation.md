# Reconciliation

- Every month, for each bank, cash, and card account: file the statement (books.file_document category statement), record what it says (books.record_statement), mark the lines it shows (books.mark_cleared), preview books.reconcile with statement_attachment, and confirm with confirmed=true once the difference is exactly zero: that records the reconciliation with its statement.
- Never confirm by forcing the numbers; find the difference.
- books.void_reconciliation (with a reason) undoes a mistake; books.void_statement (with a reason) a mis-recorded statement, to record it again.
- books.digest lists unreconciled_accounts: accounts not reconciled through the end of last month.

## What books.reconcile does

Give statement_attachment (a statement recorded with books.record_statement: its account, period end and closing balance), or account, statement_date and statement_balance for a quick check against any balance. It returns the book balance, the cleared balance, the statement balance, the difference (cleared minus statement; zero means reconciled), the cleared and uncleared lines through the statement date, and the problems that would stop it being recorded. Without confirmed it is a preview and records nothing. With confirmed and a recorded statement it records the reconciliation (journaled as reconciliation.completed, linked to the statement), refused unless the difference is exactly zero and it comes after the account's last reconciliation. Mark lines cleared with books.mark_cleared first.
