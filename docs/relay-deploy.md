# Running the relay on wanderbooks.link

This guide brings the relay up on a rented machine, step by step. The relay
is the program that lets a web assistant (Claude, ChatGPT) reach a tester's
desktop app. It has only ever run inside the test suite, so the first real
run is also its first test. The design is in `plan/p15-app-and-relay.md`
(section 5) and the rules in `docs/spec/claims/relay.md`.

Commands marked "desktop" are for your own Linux computer and work in fish.
Commands marked "server" are typed on the rented machine after `ssh`, where
the shell is bash.

## What the relay does, in one paragraph

One program does two jobs. It is the DNS server for `wanderbooks.link`: every
name under the domain points at the machine, so there is no zone file to
maintain. It also listens on port 443, reads the server name in the first
message of each connection, and passes the connection down the line the
right app holds open, without reading it. The only certificate the relay
holds is its own, for `relay.wanderbooks.link`: apps dial that name and check
it against the public roots. Apps get their own certificates; the relay holds
none of them.

## 1. What to rent on Vultr

Rent one machine:

- Product: Cloud Compute, the regular (shared CPU) type, the smallest plan:
  1 vCPU, 1 GB of memory, about $5 to $6 a month. Check the price on the page.
- Operating system: Debian 13 (the current stable release). If the list does
  not offer 13, take Debian 12. Why Debian: it ships with nothing listening
  on port 53 (Ubuntu runs a small resolver there, and the relay needs the
  port), it is plain and long supported, and it has the packages we use
  (`certbot`, `ufw`, `unattended-upgrades`).
- Location: any region. Pick the one nearest the testers.
- Turn on IPv4 (it is on) and IPv6 ("Enable IPv6" in the deploy form). The
  relay serves both.
- Add your SSH public key in the deploy form. You will log in as `root` with
  the key, and the relay itself runs as an account with no powers.

How many testers fit: the relay measured 15.4 KiB of memory for each idle
app (`crates/relay/tests/memory.rs`). After the system's own share, a 1 GB
machine has room for tens of thousands of idle apps. The settings file
sets `max_lines = 20000` (about 310 MiB), which is hundreds of times more
than a test release needs. Memory is not the limit here; the certificate
authority's 50 new certificates a week is (see section 11).

When the machine is up, copy two things from the Vultr page: the IPv4
address and the IPv6 address. The rest of this guide calls them
`<server IPv4>` and `<server IPv6>`.

## 2. DNS: hand the domain to the relay

The relay answers all DNS for `wanderbooks.link` itself, so you do not add
an A record, a wildcard or a CAA record anywhere. You tell the registrar
where the domain's name servers are. The name servers are named inside the
domain (`ns1.wanderbooks.link`), so the registrar must also be told their
addresses. These are called glue or host records.

Where the domain is registered, add these. Remove any other name servers
and any DNSSEC (DS) record, because the relay does not sign its answers.

| What | Name | Type | Value |
|---|---|---|---|
| Host (glue) record | `ns1.wanderbooks.link` | A | `<server IPv4>` |
| Host (glue) record | `ns1.wanderbooks.link` | AAAA | `<server IPv6>` |
| Host (glue) record | `ns2.wanderbooks.link` | A | `<server IPv4>` |
| Host (glue) record | `ns2.wanderbooks.link` | AAAA | `<server IPv6>` |
| Name servers of the domain | `wanderbooks.link` | NS | `ns1.wanderbooks.link` |
| Name servers of the domain | `wanderbooks.link` | NS | `ns2.wanderbooks.link` |

Two names point at one machine because a domain registry wants two name
servers. Some registrars refuse two names with the same address. If yours
does, tell the person running the project before going on, and an extra
Vultr address can be added.

The relay then answers these, which you can see with `dig` in section 9. You
do not enter them anywhere:

| Name | Type | What the relay answers |
|---|---|---|
| `wanderbooks.link` and every name beneath it, including `relay.wanderbooks.link` and `*.<anything>.wanderbooks.link` | A, AAAA | `<server IPv4>`, `<server IPv6>` |
| `wanderbooks.link` | NS, SOA | the two name servers above |
| `wanderbooks.link` | CAA | `0 issue "letsencrypt.org; validationmethods=dns-01"` (made from `authorities.toml`) |
| `_acme-challenge.<four-words>.wanderbooks.link` | TXT | the proof value an app has just handed the relay, for a few minutes |

The CAA record matters for the relay's own certificate; section 7 explains.

## 3. First login and the machine's basics

Desktop:

```fish
ssh root@<server IPv4>
```

Server (bash), as root. Update, install what is needed, and turn on the
automatic security updates:

```bash
apt-get update && apt-get -y upgrade
apt-get -y install ufw unattended-upgrades certbot build-essential git curl sqlite3 dnsutils
printf 'APT::Periodic::Update-Package-Lists "1";\nAPT::Periodic::Unattended-Upgrade "1";\n' > /etc/apt/apt.conf.d/20auto-upgrades
```

The firewall. Port 22 is for you, 53 (both kinds) and 443 are the relay, and
80 is used only by certbot for the relay's certificate (section 7):

```bash
ufw default deny incoming
ufw default allow outgoing
ufw allow 22/tcp
ufw allow 53/tcp
ufw allow 53/udp
ufw allow 80/tcp
ufw allow 443/tcp
ufw enable
```

Say `y` when it warns about ssh. Keep this terminal open, and in another
desktop terminal check `ssh root@<server IPv4>` still works before closing
it.

Turn off password logins (the key is the way in):

```bash
sed -i 's/^#\?PasswordAuthentication.*/PasswordAuthentication no/' /etc/ssh/sshd_config
systemctl reload ssh
```

A 1 GB machine can run out of memory while it compiles the relay, so add
some swap space:

```bash
fallocate -l 2G /swapfile && chmod 600 /swapfile && mkswap /swapfile && swapon /swapfile
echo '/swapfile none swap sw 0 0' >> /etc/fstab
```

Nothing may already listen on port 53. This must print nothing:

```bash
ss -lunp | grep ':53 '
```

## 4. Getting the relay onto the machine

Recommended: build it on the server. The relay's program is called `relay`
and is built from this repository. Building on the server needs no
cross-compiler and cannot hit a mismatch between your desktop's system
libraries and the server's, which a program built on a newer Linux desktop
can (it then refuses to start with an error about `GLIBC`).

Copying a binary built on your desktop is possible for a statically linked
`x86_64-unknown-linux-musl` build. The relay's dependencies are pure Rust
apart from two that compile C (`ring` and bundled SQLite, and no OpenSSL), so
musl should work, but nobody has tried it here, and it needs the musl target
and `musl-gcc` on your desktop. This guide does not use it.

Server (bash). Install Rust, fetch the code and build. The build takes
several minutes on this machine:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
. "$HOME/.cargo/env"
git clone https://github.com/swaits/wanderbooks /root/wanderbooks
cd /root/wanderbooks
cargo build --release --locked -p relay
```

When it ends, `/root/wanderbooks/target/release/relay` exists.

## 5. The account, the folders and the settings

Server (bash), from `/root/wanderbooks`:

```bash
cd /root/wanderbooks
useradd --system --home-dir /var/lib/wanderbooks-relay --no-create-home \
        --shell /usr/sbin/nologin wanderbooks-relay
install -m 0755 target/release/relay /usr/local/bin/wanderbooks-relay
install -d -o root -g wanderbooks-relay -m 0750 /etc/wanderbooks-relay
install -o root -g wanderbooks-relay -m 0640 deploy/relay/relay.toml /etc/wanderbooks-relay/relay.toml
install -o root -g wanderbooks-relay -m 0640 deploy/relay/authorities.toml /etc/wanderbooks-relay/authorities.toml
install -m 0644 deploy/relay/wanderbooks-relay.service /etc/systemd/system/wanderbooks-relay.service
install -D -m 0755 deploy/relay/install-relay-cert.sh /etc/letsencrypt/renewal-hooks/deploy/wanderbooks-relay.sh
systemctl daemon-reload
```

Open the settings and replace the placeholders (`<server IPv4>` three times
and `<server IPv6>` three times, with the real addresses, no angle
brackets). If the machine has no IPv6, set `address_v6 = []` and delete the
`v6` lines:

```bash
nano /etc/wanderbooks-relay/relay.toml
```

Every setting is explained in that file. The state folder
`/var/lib/wanderbooks-relay` is made by the service unit; the registry (the
durable table of names and keys) lives there as `registry.sqlite3`.

## 6. The first start, with a stand-in certificate

The relay will not start without certificate files, and certbot can only
prove the name `relay.wanderbooks.link` once the relay is running and
answering DNS for it. So there is one first-start step: give the relay a
two-day self-signed stand-in so it can start. Certbot replaces it in the
next section.

Server (bash):

```bash
cd /etc/wanderbooks-relay
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
  -days 2 -subj '/CN=relay.wanderbooks.link' \
  -keyout relay-key.pem -out relay-cert.pem
chown root:wanderbooks-relay relay-key.pem relay-cert.pem
chmod 0640 relay-key.pem relay-cert.pem
systemctl enable --now wanderbooks-relay
journalctl -u wanderbooks-relay -n 20 --no-pager
```

The log should say `router on ... DNS on ... (udp) and ... (tcp)`. If it
says `could not start`, read the reason: a typo in `relay.toml`, or
something already on port 53 or 443.

Now check that the domain is delegated to it. Desktop (fish):

```fish
dig +short NS wanderbooks.link
dig +short A relay.wanderbooks.link
```

The first should print the two `ns` names and the second `<server IPv4>`.
Registrar changes can take from minutes to a day to reach everyone. Do not
go on until both answer.

## 7. The relay's own certificate

The relay needs a publicly trusted certificate for `relay.wanderbooks.link`,
because every app checks the relay against the public roots. The relay does
not obtain it; certbot does, with the HTTP method on port 80 (certbot
listens there itself for a moment; the relay does not use port 80). The
relay's DNS publishes a CAA record for this one name that lets Let's Encrypt
issue by any method, while every app's name still has the stricter
DNS-only rule from the apex. That is why the authority list is on from the
start.

Server (bash):

```bash
certbot certonly --standalone --preferred-challenges http \
  -d relay.wanderbooks.link --agree-tos --register-unsafely-without-email \
  --deploy-hook /etc/letsencrypt/renewal-hooks/deploy/wanderbooks-relay.sh
```

(Use `-m <your address>` in place of `--register-unsafely-without-email` if
you want expiry mail from Let's Encrypt.) The deploy hook copies the new
files over the stand-in and restarts the relay. Check the result:

```bash
journalctl -u wanderbooks-relay -n 20 --no-pager
certbot certificates
```

## 8. Renewal

Nothing to do. The `certbot` package runs its own timer twice a day and
renews the certificate when it is about 30 days from its end. The deploy
hook then copies the files and restarts the relay, because the relay reads
its certificate only when it starts. The restart drops every app's line
for a second or two. The apps redial by themselves, with a short random wait
that starts at one second (`crates/ledger-link`), so they come back without
anyone doing anything.

To see that renewal will work, without changing anything. Server (bash):

```bash
certbot renew --dry-run
```

Port 80 must stay open in the firewall for this to work.

## 9. Checking it works, from your desktop

None of this needs any wanderbooks program. Desktop (fish):

```fish
# The name servers and the delegation (what the registrar tells the world).
dig +short NS wanderbooks.link

# Any name under the domain is at the relay, including made-up ones.
dig +short A relay.wanderbooks.link
dig +short AAAA relay.wanderbooks.link
dig +short A anything.at-all.wanderbooks.link

# The relay's own answers, bypassing every cache.
dig @<server IPv4> CAA wanderbooks.link +noall +answer
dig @<server IPv4> +tcp A relay.wanderbooks.link +short

# It is not an open resolver: a name outside the domain is refused.
dig @<server IPv4> example.com +noall +comments | grep status
```

Expected: two `ns` names, `<server IPv4>` (and `<server IPv6>` for AAAA),
the CAA line `0 issue "letsencrypt.org; validationmethods=dns-01"`, and
`status: REFUSED` for `example.com`. Also check the relay's own name: `dig @<server IPv4> CAA relay.wanderbooks.link +noall +answer` should print
`0 issue "letsencrypt.org"`, with no `validationmethods`.

The TLS door. The first command should show a certificate for
`relay.wanderbooks.link` from Let's Encrypt and `Verify return code: 0 (ok)`.
The second, with a name nobody has claimed, should end at once with an
`unrecognized_name` alert:

```fish
openssl s_client -connect <server IPv4>:443 -servername relay.wanderbooks.link -tls1_3 </dev/null 2>&1 | grep -E 'subject=|issuer=|Verify return'
openssl s_client -connect <server IPv4>:443 -servername nothing.here.wanderbooks.link -tls1_3 </dev/null 2>&1 | grep -i -E 'alert|unrecognized'
```

The relay's log, which carries a line of counters every minute. Desktop:

```fish
ssh root@<server IPv4> journalctl -u wanderbooks-relay -n 50 --no-pager
ssh root@<server IPv4> journalctl -u wanderbooks-relay -f
```

A line like `lines 0 routed 0 unknown_or_offline 1 shed 0 hello_timeouts 0
bad_hello 0 claims 0` is healthy and idle. `lines` is apps connected now,
`claims` is names registered since the start, and `shed` counts connections
turned away because the relay was full.

Last, the real test: on a desktop with the app, turn on web assistants for a
company and watch `lines` become 1 in the log.

## 10. Looking after it

Restart, status and stop. Server (bash):

```bash
systemctl status wanderbooks-relay
systemctl restart wanderbooks-relay   # drops every line; apps reconnect
```

Update to a new version of the relay. Server (bash):

```bash
cd /root/wanderbooks && git pull
. "$HOME/.cargo/env"
cargo build --release --locked -p relay
install -m 0755 target/release/relay /usr/local/bin/wanderbooks-relay
systemctl restart wanderbooks-relay
```

Changing the authority list needs no restart: edit
`/etc/wanderbooks-relay/authorities.toml` and the relay reads it within half
a minute. The settings file (`relay.toml`) and the certificate are read
only at start.

Block or unblock a name (a name is four words joined by hyphens). Server
(bash):

```bash
sudo -u wanderbooks-relay /usr/local/bin/wanderbooks-relay /etc/wanderbooks-relay/relay.toml block <four-word-name>
sudo -u wanderbooks-relay /usr/local/bin/wanderbooks-relay /etc/wanderbooks-relay/relay.toml unblock <four-word-name>
```

A block closes that app's line within about ten seconds.

Back up the registry. It is the one thing that cannot be rebuilt: it records
which key owns which name. If it is lost, every app's name is up for grabs.
Make a consistent copy while the relay runs, and bring it to your desktop.
Server (bash):

```bash
install -d -m 0700 /root/relay-backups
sqlite3 /var/lib/wanderbooks-relay/registry.sqlite3 ".backup /root/relay-backups/registry-$(date +%F).sqlite3"
```

Desktop (fish):

```fish
mkdir -p ~/relay-backups
scp 'root@<server IPv4>:/root/relay-backups/*' ~/relay-backups/
```

Do this after any day with new testers. To restore, stop the relay, put the
file back as `/var/lib/wanderbooks-relay/registry.sqlite3`, run
`chown wanderbooks-relay:wanderbooks-relay` on it, and start the relay. Also
consider Vultr's paid automatic backups for the whole machine.

## 11. What this does not do yet

Each item is checked against the code.

- **A renewal restarts the relay.** The relay reads its certificate only
  when it starts (`crates/relay/src/tls.rs`), so each renewal, about every
  60 days, drops every line for a moment. Apps redial on their own, so no
  one has to act, but an assistant mid-call fails. Reloading the certificate
  without a restart is not built.
- **No watching of the certificate logs.** The plan says the app should warn
  the owner of a certificate issued for their name that they did not ask for.
  Nothing in the code does this. A relay that misused its DNS to get a
  certificate for a tester's name would not be noticed.
- **No higher Let's Encrypt limit.** Until Let's Encrypt grants a raised
  limit, only 50 new apps a week can get certificates, and the relay enforces
  that ceiling itself (`cert_publishes_per_window`). The request is a human
  task, made for the registered domain by one account, and the plan says to
  send it as soon as the relay runs. It has not been sent.
- **No monitoring.** The relay has no health page or metrics port. The only
  signals are the minute counters in the log and the checks in section 9.
  Nothing tells you if the machine is down. A free external uptime check on
  `https://relay.wanderbooks.link` and on a `dig` of the domain would do.
- **One machine, one point of failure.** Both name servers are the same
  machine. If it is down, the domain does not resolve (for five minutes after
  its answers expire) and no assistant reaches any app.
- **Authorities that need an account binding do not work.** Only Let's
  Encrypt is in the sample list. ZeroSSL and similar are listed as not
  usable, because the per-app source of bindings is not built.
- **Untried against the real thing.** The relay has never run outside tests,
  against a real certificate authority or a real assistant. The largest soak
  was 20,000 lines; the 100,000-line soak has not been run.
- **IPv6 and DNSSEC.** The settings listen on `[::]` for both stacks, which
  has been checked in the code and not on a live machine. The DNS does not
  sign its answers, so the domain must have no DS record at the registrar.
- **No backup schedule.** Section 10 is a manual routine.
