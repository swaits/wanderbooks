# CPA handoff

- books.export_cpa_package carries every member's and 1099 payee's full TIN from the vault (the K-1 worksheet and the 1099 worksheet). It hands the zip over: run its download command with the folder the human wants it in (books served by `wanderbooks mcp`), then tell them where it is. The viewer's "Export for CPA" makes the same package.
- Never file the package, or a file from it, into the books.
- tin_gaps lists each member or payee whose TIN it lacks and why: collect them (books.set_tin, books.set_w9) and export again.
