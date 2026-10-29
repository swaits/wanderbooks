Go through the year-end compliance work for {{year}} for {{company}}.

1. books.start, and books.open_items: what is already open, with its due dates.
2. Read books.handbook sections 1099, parties-and-tins and rules-that-change. Work out the calendar for this entity type and list the filings and dates that apply; where you are unsure of a rule, raise it with books.raise_item for the human or the CPA rather than guessing.
3. 1099 work: books.vendor_1099 for {{year}}; collect missing W-9s (books.set_w9; a full TIN goes only into the vault, never into your messages).
4. Fixed assets: books.asset_schedule and books.run_depreciation for the year, previewed.
5. Every month of {{year}} closed (the month_end_close prompt) and reconciled.
6. Record each decision the human makes with books.record_decision, and each follow-up with books.raise_item.
