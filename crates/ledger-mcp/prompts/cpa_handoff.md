Prepare the hand-off to the CPA for {{year}}.

1. books.start, then books.verify: the journal's chain, the replay and every stored file must be clean before anything leaves.
2. Check each of the year's months is closed (books.list_periods) and reconciled (books.reconciliations); list what is not and stop if any is missing.
3. books.control_check, and the 1099 work: books.vendor_1099 for the year, with W-9s in the vault (books.handbook section 1099).
4. Read books.handbook section cpa-handoff, then tell the human what the package holds and whether the full TINs go in it (they go in by default; the human decides).
5. books.export_cpa_package for the year. The package is handed over as a download: run the command the result gives, in a folder the human names, within 15 minutes.
6. Offer books.digest for the year.
