# Settings, conventions, decisions and open items

The books keep their own record of how they are kept. Every agent, anywhere, reads it through these tools; none of it lives in local files.

## Settings

- books.settings lists every setting the books know: what it means and takes, its value (null: unset, its default applies), who set it and when, and problems (settings that disagree with the chart, each saying what to change).
- Change settings with books.set_setting: one or more {key, value} together, with who decided (decided_by) and when (decided_on, default today). A value is checked and kept in one spelling; null clears a key back to its default. Change a setting when the human tells you a company fact or decides a policy, when a tool refuses naming a setting, when books.settings or books.digest lists a problem, and after books.renumber_account lists config_mentions.
- The company's EIN is the one identity number the settings keep; it is shown masked. No other setting, note or convention ever holds a full TIN: those go only into the vault (the parties-and-tins section).
- company.timezone sets what "today" means for the books.
- policy.require_tax_category is warn (unset: a revenue or expense account without a tax category is created, and the result and Needs attention say so) or refuse (books.add_account, books.import_chart_csv and books.set_tax_category won't leave one without; the accounts invoices, bills and payroll create are never refused, and their results name them). Before setting refuse, categorize with the human every account books.digest lists in uncategorized_accounts.
- books.export_settings writes the settings out as a settings file (TOML), each with who decided it. It is a copy: editing it changes nothing.

## Conventions

How this company keeps its books, as the owners decided, is recorded as conventions (books.set_convention, read with books.conventions; books.start lists them):

- posting_template: how a kind of transaction posts, as lines with shares of the amount ({lines: [{account, side, share}]}, each side's shares summing to 1). An account may be a slot, a name in braces such as "{expense}", that each draft names (books.draft_from_template accounts: {"expense": "6300"}). books.draft_from_template drafts an entry from one, split to the cent; preview and post it as usual.
- approval_rule: who approves which entries ({approver, accounts?, amount_over?}). They block posting only when policy.approval_rules is enforce (off by default): then a post's preview lists approvals still needed, books.approve_entry records each once the approver says yes, and a tool that would post a covered entry at once (an invoice, a bill, a payment, payroll, a disposal) is refused: draft the entry instead. Reversals, closes, depreciation and amortization are exempt. An account a rule names can't be renumbered until the rule is revised.
- policy, close_step, calendar, mapping, note: in words.

Follow the company's conventions; when the human decides a new way of doing something, record it. A convention that no longer applies is retired (books.retire_convention), never deleted; its id is never reused.

## Decisions and open items

- books.record_decision records what the owners decided (a policy, a treatment, a cutover) with who decided, when, and the filed document behind it if there is one. That is the whole record: no signatures or memos are needed unless an outside party requires one.
- books.raise_item records something open (waiting on someone, due later); books.open_items lists them and books.resolve_item closes one with how. Raise open items with the human when they come due.
