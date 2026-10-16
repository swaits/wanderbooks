# Prepaid expenses, deferred revenue and recurring entries

- Prepaid expenses and deferred revenue: record the original payment (with its receipt), then books.create_schedule; books.run_schedules posts the monthly amortization (preview first; exempt from the receipt policy like depreciation).
- Recurring entries (books.create_recurring, books.run_recurring) only ever create drafts: attach each actual receipt and post through the normal preview.
