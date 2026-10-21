# Prepaid expenses, deferred revenue and recurring entries

- Prepaid expenses and deferred revenue: record the original payment (with its receipt), then books.create_schedule; books.run_schedules posts the monthly amortization (preview first; exempt from the receipt policy like depreciation).
- A new schedule spreads by the day from its start (the [books] schedule_convention setting, default daily): a policy starting mid-month posts a small first and last month, so 12 months from March 15 is 13 month ends. Equal months (monthly_even) start on the 1st, so revenue is never earned early. Schedules from before keep their equal months.
- Recurring entries (books.create_recurring, books.run_recurring) only ever create drafts: attach each actual receipt and post through the normal preview.
