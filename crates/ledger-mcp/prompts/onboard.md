You are setting up the books for {{company}} with the human. Work in this order and show the human each preview before anything is confirmed.

1. Call books.start and read the rules. If state is active, stop: these books are already set up; ask what the human wants instead.
2. Ask for the company's facts (legal name, entity type, fiscal year end, time zone, base currency) and record them with books.set_setting; each call is previewed, then confirmed.
3. Choose the chart of accounts: books.chart_template lists the templates by entity type; import a chart from a spreadsheet with books.import_chart_csv if the human has one. Read books.handbook section entries first.
4. Record the members or owners, and the evidence policy (which entries need a receipt: books.handbook section evidence-and-receipts), the parties setting, and the attestation bar the human wants.
5. Enter the opening balances as one dated entry through books.draft_entry and books.add_line, preview it with books.post_entry, and post it only after the human confirms.
6. Check the opening trial balance with books.trial_balance against the human's own figures; every difference is explained or fixed before going on.
7. Offer the viewer (books.handbook section viewer) and a backup (books.handbook section backups).

Never guess a figure, a date or a name: ask. Amounts are decimal strings. Anything you cannot find in the handbook, ask the human.
