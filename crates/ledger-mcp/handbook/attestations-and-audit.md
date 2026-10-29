# Attestations and audit

An audit is the books read as an auditor reads them, through a day, in one run that gives the same findings for the same books. An attestation is a claim by a checker that it looked at something and found it right. The audit finds; the attestations stand behind it. The CPA package waits for both.

The receipt rule in force: {evidence_rule}

## Running an audit

- books.audit { through, scope } without confirmed lists the checks and changes nothing; show it to the human. Then call it again with confirmed: true and the preview. It files the workpaper (JSON and PDF, category workpaper) and journals the run. books.workpaper { id } gives it back: the JSON, and the PDF handed over (run its download command to save it).
- scope: month (a month-end), annual (a fiscal year), cpa_handoff (the year, for the CPA package), full (everything since the first entry).
- It checks the trial balance, the accounting equation, the profit and loss against equity, the cash flow's footing, the A/R and A/P control accounts, closes, schedules and depreciation, the asset register, every posted entry against the receipt rule, open drafts, tax categories, 1099 decisions and the settings; then the journal's chain, the stored files, the access log and the backups. A finding is a failure (the books are wrong or cannot be shown right), a warning (unfinished or a rule not met) or a note (listed so the workpaper is whole: a reversal rests on its original's receipt).
- Fix findings with journaled corrections (a reversal, a receipt attached, a decision recorded), never by editing: posted entries stay as they are. Then audit again.
- The same books, day and scope give the run that exists: asking again files nothing. The books are "the same" while no entry, document, close, settings or role change has been made; filing a receipt, an audit or an attestation does not change them.
- Books that have not declared audits refuse a confirmed audit and every attestation with format_upgrade_pending until the owner's upgrade (books.start says so). Nothing else about them changes.

## Attesting

Use separate verifier agents; none of them should be the one that made the entry. For each receipt:

1. books.get_receipt for the extracted text, then its page image, and read the total and the date off the receipt itself.
2. books.attest_receipt { attachment_id, entry_id, amount, date, method, note }. The books compare what you read with the entry's amounts and date. A difference is recorded as disagrees and never counts: do not adjust your reading to fit.
3. books.review_entry { entry_id, method } for each posted entry, once its lines, accounts, party and evidence have been read whole.
4. books.attest_period { period, method } last, on a stand-still: it is refused unless an audit through the period's end still stands, which it does only while nothing has been posted, created, closed or set since the audit. If it is refused, audit again.

method is your own claim: model, reasoning_effort (low, medium, high, xhigh, max), adversarial (true when the verifier was set to find what is wrong), verifier_agents (how many independent ones checked). The books record it and cannot verify it; say what is true.

## What counts

books.attestations lists every claim with counted: whether it meets policy.attestation_bar as the bar stands when you ask. The default bar is reasoning effort high; a year-level audit (annual, cpa_handoff, full) also needs the check to be adversarial and at least two independent verifier agents. A bar change changes what counts, earlier claims too: nothing is versioned. A claim whose receipt reading disagrees with the entry never counts.

## The CPA package

Under policy.cpa_package_requires_audit = refuse (the default once audits are declared), books.export_cpa_package is refused unless an audit for the handoff (scope cpa_handoff or full) stands at the current books with a period attested on it at the bar. Pass without_audit_reason only when the human says to hand it over anyway; the reason is journaled as a decision. Under warn the package is made and says so.
