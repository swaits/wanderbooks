# CPA handoff

- books.export_cpa_package carries every member's and 1099 payee's full TIN from the vault (the K-1 worksheet and the 1099 worksheet), so it needs save_to: a new .zip or a folder outside the books.
- Tell the human where it was saved; never open, read, attach, or copy the file, and never echo its TINs.
- tin_gaps lists each member or payee whose TIN it lacks and why: collect them (books.set_tin, books.set_w9) and export again.
