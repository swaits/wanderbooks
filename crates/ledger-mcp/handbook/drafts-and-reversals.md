# Drafts and reversals

Drafts
- A draft (books.draft_entry, books.add_line) is in no report until it posts. books.list_drafts shows the open ones; books.preview_entry shows what posting one would do and returns the preview to confirm with.
- A draft is either building or awaiting_evidence. When its lines are final and it only waits for its receipt, say so: books.set_draft_state with state awaiting_evidence and a note saying what it waits for ("Figma invoice due 11/3"). The state changes no figure; the receipt rule is checked when the draft posts. When the receipt arrives, attach it (books.attach_receipt), preview, and post.
- A draft that will never post (entered twice, never billed) is thrown away with books.discard_draft and the reason: previewed first, then confirmed with the preview. Nothing in any report moves. A period holding an open draft cannot close, so post or discard it first.
- Reports say what is still open as of their end (as_of_status); pass strict: true to refuse a report while a draft dated in it is open. The CPA package is refused while one is open unless you pass allow_open_drafts with the reason.

Every confirm takes the preview
- Every tool that takes confirmed returns a preview (a digest) when called without it, and needs that preview back with confirmed=true. If the books or an argument changed since (another session posted, a line was added), the confirm is refused as stale_preview: preview again, show the human, then confirm.

Reversals
- Posted entries are immutable: there is no edit and no delete.
- Corrections go through books.reverse_entry: without confirmed it previews the reversing entry and changes nothing; with confirmed=true and the preview it posts the reversal, linked to the original. Then draft and post the corrected entry through the normal flow.
- A reversal needs no receipt of its own: its evidence is its original's, which stays filed. A returned-item notice from the bank, if there is one, is attached to the reversal right after.
- Show the human the reversal preview before confirming, as for any posting.
