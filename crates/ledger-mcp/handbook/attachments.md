# Attachments

- File a file once and link it wherever it is evidence (books.link_attachment, or link_to on books.attach_receipt and books.file_document): an entry (a file linked to an entry counts as its receipt), an account, an asset, a party, an invoice, a bill, a period, a payroll run, a schedule, a reconciliation, or the company.
- books.attachments lists a subject's files; books.unlink_attachment takes a link off with a reason.
- File documents under their kind (statement, contract, photo, warranty, import_source, formation, legal, tax, accounting, other).
- File a bank export before importing it and pass source_attachment_id to books.import_bank_csv.
