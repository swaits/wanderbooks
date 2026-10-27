# On a machine of the owner's own

These books may live on a machine the owner controls, not on the computer
an agent runs on. The server there runs as a separate account, and the
agent has no account on that machine: `books.start` says
`isolation: "service_user (enforced)"`. Locally it says
`same_user (advisory)`, which is honest: hidden, not locked.

What that means for you:

- Reach the books only through these tools. The machine they live on is
  not yours to inspect, and you have no way to reach it except through
  the server.
- The owner moves the books there with a verified copy or a backup, and
  the move is confirmed at the head the preview read. If `books.verify`
  or `books.start` reports a head the owner does not expect, say so and
  stop; do not touch the books until the owner has looked.
- Each client has its own token. Never ask the owner for one, never print
  one, and never put one in a note, an entry, a receipt or a file in the
  books. If a tool says the token was refused, ask the owner to make a
  new one for that client.
- Everything else is the same as on the owner's computer: the receipt
  rule, the preview before a post, the books' own settings and
  conventions. The machine changes where the books are, not how they are
  kept.

If the owner asks how the machine is set up, point them at the deployment
notes that came with the app; do not invent steps for them, and do not
try to run any yourself.
