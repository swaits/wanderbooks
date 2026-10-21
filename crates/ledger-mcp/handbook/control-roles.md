# Control roles

- Invoices, bills, payroll, and closes post to the account filling each role (receivables, revenue, payables, general expense, retained earnings, payroll expense and withholdings); books.roles lists them.
- A role uses its default code (1200, 4000, 2000, 6999, 3999, 6100, 6110, 2110-2150) until it is assigned one of the chart's own accounts with books.assign_role (journaled). If the chart uses a default code for a different kind of account, the engine refuses and says which. It can't tell an account of the right type but another purpose (a hosting expense at payroll's 6100): check books.roles before running payroll, bills, or invoices.
- Never assign roles or renumber the human's accounts on your own: explain the conflict, and when they decide, assign the role to their account (best: their numbering stays) or books.renumber_account the account (its history stays); books.rename_account changes only its name, never a role.
