# The books on a machine of your own

Plan 8.2 P9a, section 3.8. This is the recipe for the enforced deployment:
the books live on a machine you control, in a folder only a dedicated
account can read, and agents reach them through the server with a token.
Locally the books are `same_user (advisory)` — hidden from the agent, not
locked against it; here they are behind an account and a machine boundary.

Nothing here needs the owner's books: every step works on a copy.

The pieces:

- `wanderbooks-server.service` — the system unit (one account, one data
  root, loopback only).
- `wanderbooks` — the server-only build of the one executable
  (`cargo build --release -p ledger-cli --no-default-features`).
- Caddy (or Tailscale Funnel, or cloudflared) — TLS and the door from the
  internet.
- `crates/ledger-cli/tests/two_user.rs` — the two-account checks the CI job
  `.github/workflows/two-user.yml` runs, and which you can run on one
  machine (step 9).

## 1. The account and the folder

```sh
sudo useradd --system --create-home --home-dir /var/lib/wanderbooks \
             --shell /usr/sbin/nologin wanderbooks
sudo install -d -o wanderbooks -g wanderbooks -m 0700 /srv/wanderbooks
sudo install -m 0755 target/release/wanderbooks /usr/local/bin/wanderbooks
sudo install -D -m 0644 deploy/wanderbooks-server.service \
             /etc/systemd/system/wanderbooks-server.service
```

The executable is the server-only build: no viewer, no window libraries.
On a Linux package install (P11) this is the same binary and the package
manager writes the file; until then, copy it as above.

## 2. Make the data root, once

```sh
sudo -u wanderbooks -H /usr/local/bin/wanderbooks setup --root /srv/wanderbooks
```

`setup` makes the folder private (0700), records it for that account and
asks for the owner passphrase when a terminal is attached. Self-host has
no `/h/` pages (D-28, D-51); the passphrase guards a format upgrade only.
Set it at a terminal if the machine has none:

```sh
ssh -t you@server 'sudo -u wanderbooks -H /usr/local/bin/wanderbooks passphrase'
```

## 3. Bring the books over

The transport is a backup clone or a copied data folder (D-31: there is no
`fetch`, and no separate archive verifier). On the machine the books are
on now:

```sh
wanderbooks close                 # nothing writes while it is copied
wanderbooks books list            # note the id, e.g. acme
cp -a "$XDG_DATA_HOME/wanderbooks/books/acme" /media/usb/acme
```

The copy must hold the journal, the stored files, the vault and its key;
`cp -a` of the book's folder holds all four. If you would rather move a
verified backup, make one (`wanderbooks books back-up acme`) and clone the
repository it wrote; both are the same books.

Bring that folder to the server, start the service, and adopt it:

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now wanderbooks-server
sudo -u wanderbooks -H /usr/local/bin/wanderbooks books adopt /media/usb/acme \
     --root /srv/wanderbooks --id acme
```

The preview names the source's head (`Head: event N (short hash)`); with
`--yes` (or a `y` at the prompt) the move is confirmed against exactly
that head, so a source that changed after the preview is refused. The
original is left as it was; nothing is deleted.

**The head check.** Ask the agent on the server for `books.verify` (or
`books.start`): both report the head. Compare it with `books.verify` on
the machine the books came from, taken before the move. They are the same
event count, sequence number and hash: the move loses nothing. (The copy
is written in this version's journal format; a source written by an older
version gains one format event at adoption, and then the counts differ by
exactly that event, which the preview says: "The copy is in this version's
journal format".)

## 4. Keep the machine's hardening

The unit says `--isolation enforced`, so `books.start` reports
`service_user (enforced)`: the server is told, it never guesses. The unit
also sets `ProtectSystem=strict`, `NoNewPrivileges=yes`, `ProtectHome=yes`
and `IPAddressDeny=any` with `IPAddressAllow=localhost`, and no seccomp or
Landlock lines (D-32, D-51).

The loopback-only rule is deliberate: the server reaches nothing out. A
book whose backup target is a **remote** git repository needs that host
allowed, or its backups fail:

```ini
# /etc/systemd/system/wanderbooks-server.service.d/backup.conf
[Service]
IPAddressAllow=203.0.113.10
```

A backup to a folder on this machine needs nothing. The tunnel and Caddy
run as their own services and reach 127.0.0.1:7411, which the rule allows.

## 5. A token for each client

The token is a static bearer (`wb_` and 32 random bytes); the server keeps
only its SHA-256. Mint one per client, named for it:

```sh
sudo -u wanderbooks -H /usr/local/bin/wanderbooks token create \
     --root /srv/wanderbooks --book acme --name laptop
sudo -u wanderbooks -H /usr/local/bin/wanderbooks token print \
     --root /srv/wanderbooks --book acme --name laptop
```

`token create` keeps the text in a file in the server account's private
data root (mode 0600, readable by that account alone) and `token print`
shows it once; copy it to that client and nowhere else. It works on a
headless machine as it is. The server checks the token's SHA-256 from the
books. Revoke it with `token revoke --book acme --name laptop` when a
client is lost.

## 6. TLS and the door

The server listens on 127.0.0.1:7411 for the tunnel, and on its own Unix
sockets for anything running as the same account. **The TCP door is
loopback only, on purpose**: it is plaintext with a static bearer token,
so `--http` refuses `0.0.0.0`, a LAN address or `[::]` before it binds
anything; the TLS proxy or tunnel is what faces the network. Put TLS in
front of the loopback port. Two ways that need no open inbound port:

**Tailscale Funnel** (a tailnet name, TLS included):

```sh
sudo tailscale funnel --bg 7411
```

The books are then at `https://<machine>.<tailnet>.ts.net/b/acme/mcp`.

**Caddy** (a domain you own, automatic certificates):

```caddyfile
books.example.com {
    reverse_proxy 127.0.0.1:7411
}
```

with `cloudflared tunnel --url http://127.0.0.1:7411` instead of Caddy if
you use a Cloudflare tunnel. Either way the path is passed through
unchanged: the MCP endpoint is `https://<host>/b/acme/mcp`.

Nothing else listens. Do not forward 7411 on the router, and do not put
the Unix sockets behind the proxy: they check the caller's account (3.9)
and a proxy would hide it.

## 7. Point the agent at it

**Claude Code** (a client that sends headers):

```sh
claude mcp add --transport http wanderbooks-acme https://books.example.com/b/acme/mcp \
  --header "Authorization: Bearer <the token from step 5>"
```

**Codex** (`~/.codex/config.toml`; the token stays in the environment, not
the file):

```toml
[mcp_servers.wanderbooks-acme]
url = "https://books.example.com/b/acme/mcp"
bearer_token_env_var = "WANDERBOOKS_ACME_TOKEN"
```

```sh
export WANDERBOOKS_ACME_TOKEN='<the token from step 5>'
```

The plan's local rule is that no client configuration holds a token (the
bridge reads its token file). For a remote machine there is no command yet
that keeps a remote token in the client's own token file, so this recipe puts it
in Claude's configuration (mode 0600) or in Codex's environment. That
gap is recorded in `plan/served.md` 8.2 P9a's As-built block.

## 8. First start

Start a session and ask for `books.start`. It answers
`isolation: "service_user (enforced)"`, and every tool works over the TLS
door with the token. Logs:

```sh
journalctl -u wanderbooks-server -f
```

The server runs no jobs in the background: `serve` holds the books open
until systemd stops it. On a stop it finishes the call in progress, makes
a last backup and closes (3.9), so `TimeoutStopSec=600` is generous rather
than needed.

Backups still run on their own, from the books' settings, as on the
desktop (3.7, D-30, D-31); this machine runs git itself.

## 9. Prove the boundary (what CI runs)

`.github/workflows/two-user.yml` runs the same job on every change:

1. makes two accounts, `wanderbooks` (server) and the runner (agent);
2. makes books with this version in a desktop root, copies them, and
   adopts the copy into `/srv/wanderbooks`;
3. serves them as `wanderbooks` with `--isolation enforced --http
   127.0.0.1:17411`, and mints a token;
4. runs `crates/ledger-cli/tests/two_user.rs` as the runner with
   `WB_TWO_USER=1` and the deployment's settings. The test proves the
   runner cannot open `/srv/wanderbooks` or its journal, then reads,
   writes, uploads, files, exports and verifies over the HTTP door (in a
   second book, `tunnel`, so the moved books' head is never advanced), and
   compares the moved books' head with the source's.

`agent_uid_cannot_read_tenant` and `self_host_adopt_same_head` skip
themselves when `WB_TWO_USER` is unset, so an ordinary `cargo test` on a
laptop stays green; `isolation_label_is_enforced_under_the_unit` needs no
second account and always runs.

On one machine without a second account, run the client side in a user
namespace (a different user ID than the server's):

```sh
WB_TWO_USER=1 WB_TENANT_ROOT=... WB_SERVER_URL=http://127.0.0.1:17411 \
WB_BOOK=acme WB_TOKEN=... WB_WRITE_BOOK=tunnel WB_WRITE_TOKEN=... \
WB_SOURCE_DIR=... \
unshare --user --map-auto --setuid 0 --setgid 0 -- target/debug/deps/two_user-<hash> --nocapture
```

The two books and their tokens are made by the job (or by hand as in
steps 2 to 5): `acme` is the moved books, and `tunnel` is empty, so the
write checks never move `acme`'s head. `WB_SOURCE_DIR` is the journal the
books were moved from, readable by the agent account.
