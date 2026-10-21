# Entries

- The flow is draft (books.draft_entry) -> add lines (books.add_line) -> preview (books.post_entry without confirmed) -> show the human the preview -> post (books.post_entry with confirmed=true).
- Every entry must balance: debits equal credits, exactly. Every line needs a real account from books.chart; add a missing one with books.add_account first, never guess one.
- When unsure of the category, draft and ask. A draft is cheap; a wrong posting is a reversal.
- Name the entry's party when you draft it (books.draft_entry party), so the preview can show the split (the parties-and-tins section).
- An expense or purchase a member pays personally goes to the expense or asset against the members' financing account (Due to Members), the vendor named as the party; the cash flow statement shows it as borrowed from the member and spent. What the company owes a member for their own work or property (rent, a guaranteed payment) goes to a payable instead.
