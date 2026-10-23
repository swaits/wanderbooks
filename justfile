# wanderbooks repo recipes. `just` lists them.

set shell := ["bash", "-cu"]

# The Netlify CLI, fetched on demand. Its packages' native build scripts
# (sharp, unix-dgram) fail on this machine and deploying needs neither, so
# they are skipped.
netlify := "npm_config_ignore_scripts=true npx --yes netlify-cli@27"

# List recipes
default:
    @just --list

# Everything CI should check: format, clippy and rustdoc (zero warnings), tests
check: sweep-test-tmp
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --quiet
    cargo test --workspace
    @just server-only
    @just sweep-test-tmp

# The server-only build (plan D-22, for another machine) builds, passes
# clippy and the one executable's own tests, and links no window libraries.
# The books server's tool suites (mcp_journeys, spec_*, oracle_mcp,
# hostile_args, ...: the same binary's server, D-44) run once, in `check`.
server-only:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo clippy -p ledger-cli --no-default-features --all-targets -- -D warnings
    cargo test -p ledger-cli --no-default-features --bins --test one_executable --test setup --test local_server --test open_while_held --test backups --test format_upgrade --test startup
    tree=$(cargo tree -p ledger-cli --no-default-features -e normal)
    if grep -qiE '\b(eframe|egui|winit|wgpu|glow|ledger-gui)\b' <<<"$tree"; then
        echo "the server-only build links a window library" >&2
        exit 1
    fi
    echo "server-only build: no window libraries"

# Test books go in the system temp dir: TMPDIR, else /tmp. Every test's dir is
# removed when the test ends, pass or panic (a guard dropped on unwind;
# until 2026-10-17 many were removed only on pass, or never: 117,000 piled
# up). A run killed mid-test (a mutant's timeout, ^C) runs no destructor,
# so this sweep removes this user's test book dirs older than an hour; a
# run in progress elsewhere keeps its own. Such a run can also leave its
# processes behind: a wanderbooks process whose `--root` is a test dir
# (a `wb-*` dir) that no longer exists is ended, woken first (a test may
# have stopped it with SIGSTOP; 2026-10-23, one sat stopped for a day).
# Only such a process: the live books' processes have no test `--root`.
sweep-test-tmp:
    #!/usr/bin/env bash
    find "${TMPDIR:-/tmp}" /tmp -maxdepth 1 -user "$(id -u)" -type d -mmin +60 \( -name 'wb-*' -o -name 'wanderbooks-*' -o -name 'ledger-*' \) -exec rm -rf {} + 2>/dev/null || true
    for pid in $(pgrep -u "$(id -u)" -x wanderbooks; pgrep -u "$(id -u)" -f '^wanderbooks (server|host|mcp|serve) '); do
        root=$(tr '\0' '\n' < "/proc/$pid/cmdline" 2>/dev/null | sed -n '/^--root$/{n;p;q}')
        case "$root" in
            */wb-*/*|*/wb-*) [ -e "$root" ] || { kill -CONT "$pid"; kill -KILL "$pid"; } 2>/dev/null ;;
        esac
    done
    true

# The Kani harnesses `prove` leaves to `prove-slow`: each takes four to
# twenty minutes alone on a busy machine, and with them the suite (about
# 100 minutes of CPU, 2026-10-21) can't finish inside the 30-minute cap at
# four jobs, a CI runner's. Every other harness in src/proofs.rs runs in `prove`, a new one
# included; a name here that isn't a harness fails both (docs/verification.md).
kani_slow := "accounting_equation cumulative_close_never_drifts_a_unit_at_near_thirds cumulative_closing_entry_balances entry_flows_pure_are_movements entry_flows_sum_to_cash prorate_parts_sum_to_total sums_are_exact the_last_months_charge_is_within_half_a_cent"

# Formal proofs: Kani on the accounting kernel (all but kani_slow), Lean for books of any size (docs/verification.md)
prove: (_kani "fast")
    #!/usr/bin/env bash
    set -euo pipefail
    cd verify/lean
    lake build
    audit=$(lake env lean Audit.lean)
    if grep -v -E "depends on axioms: \[(propext|Quot.sound|Classical.choice)(, (propext|Quot.sound|Classical.choice))*\]$|does not depend on any axioms" <<<"$audit"; then
        echo "Lean audit: a theorem rests on something beyond the standard axioms" >&2
        exit 1
    fi
    echo "Lean: $(wc -l <<<"$audit") headline theorems, standard axioms only"

# The slow Kani harnesses (kani_slow), each under its own 30-minute cap, $(nproc) at a time (CI runs it beside prove)
prove-slow: (_kani "slow")

# Every proof: prove, then prove-slow
prove-all: prove prove-slow

# Kani on one set of the harnesses: "fast" (all but kani_slow, the whole
# run under the cap) or "slow" (each harness under the cap)
_kani set:
    #!/usr/bin/env bash
    set -euo pipefail
    # Every harness: the fn after each #[kani::proof] in src/proofs.rs.
    all=$(awk '/#\[kani::proof\]/ { p = 1; next } p && match($0, /^fn [a-z_0-9]+/) { print substr($0, 4, RLENGTH - 3); p = 0 }' crates/ledger-core/src/proofs.rs | sort)
    slow=$(tr ' ' '\n' <<<"{{kani_slow}}" | sort)
    if missing=$(comm -13 <(echo "$all") <(echo "$slow")) && [ -n "$missing" ]; then
        echo "kani_slow names no harness: $missing" >&2
        exit 1
    fi
    if [ "{{set}}" = slow ]; then names=$slow; else names=$(comm -23 <(echo "$all") <(echo "$slow")); fi
    logs=$(mktemp -d)
    # What each run must show: every harness asked for verified, and no
    # cover unsatisfiable (a vacuous proof).
    checked() { # log, count
        if ! grep -qE "^Complete - $2 successfully verified harnesses, 0 failures, $2 total" "$1"; then
            echo "Kani: expected $2 harness(es) verified, see $1" >&2
            return 1
        fi
        if grep -E "\*\* [0-9]+ of [0-9]+ cover properties satisfied" "$1" |
            awk '{ if ($2 != $4) bad = 1 } END { exit !bad }'; then
            echo "Kani: an unsatisfiable cover; some proof is vacuous ($1)" >&2
            return 1
        fi
    }
    cd crates/ledger-core
    # Capped (timeout 1800, 12 GB each): a harness that blows up should
    # fail, not eat the machine.
    ulimit -v 12000000
    if [ "{{set}}" = fast ]; then
        args=()
        for h in $names; do args+=(--harness "proofs::$h"); done
        timeout 1800 cargo kani -j "$(nproc)" --output-format terse --exact "${args[@]}" | tee "$logs/fast.log"
        checked "$logs/fast.log" "$(wc -l <<<"$names")"
    else
        cargo kani --only-codegen --output-format terse
        failed=0
        for h in $names; do
            while [ "$(jobs -rp | wc -l)" -ge "$(nproc)" ]; do wait -n || failed=1; done
            ( start=$(date +%s)
              timeout 1800 cargo kani --output-format terse --exact --harness "proofs::$h" > "$logs/$h.log" 2>&1 &&
                  checked "$logs/$h.log" 1 &&
                  echo "Kani: $h verified in $(( $(date +%s) - start )) s" ||
                  { echo "Kani: $h failed or ran past 1800 s ($logs/$h.log)" >&2; exit 1; } ) &
        done
        while [ "$(jobs -rp | wc -l)" -gt 0 ]; do wait -n || failed=1; done
        wait || failed=1
        [ "$failed" = 0 ] || exit 1
    fi
    rm -rf "$logs"
    echo "Kani ({{set}}): $(wc -l <<<"$names") harnesses verified"

# Random books through wanderbooks and GnuCash 5.10, compared exactly (Docker)
oracle:
    docker build -q -t wanderbooks-gnucash-oracle oracle/gnucash
    cargo test -p ledger-core --test oracle_gnucash -- --ignored --nocapture

# Fuzz one target for some seconds (nightly; corpus in fuzz/corpus/<target>)
fuzz target="fuzz_op_sequence" seconds="300":
    #!/usr/bin/env bash
    set -euo pipefail
    # Call the binary directly: mise's shim would pin stable rustc.
    fuzz=$(mise which cargo-fuzz)
    cd fuzz && mkdir -p corpus/{{target}} seeds/{{target}}
    # corpus/ grows (git-ignored); seeds/ holds the checked-in minimal inputs.
    RUSTUP_TOOLCHAIN=nightly-2026-10-01 "$fuzz" fuzz run {{target}} corpus/{{target}} seeds/{{target}} -- \
        -max_total_time={{seconds}} -timeout=10 -rss_limit_mb=2048

# Undefined-behavior check of the core's tests under Miri (nightly)
miri:
    PROPTEST_CASES=4 MIRIFLAGS="-Zmiri-disable-isolation" cargo +nightly-2026-10-01 miri test -p ledger-core --lib --test kernel_bridge --test invariants

# Where cargo-mutants copies and builds the tree for each mutant (several
# GB per job): the user's cache on disk, never /tmp, which is often RAM.
# The tests it runs honor the same TMPDIR (every test's books go in
# std::env::temp_dir(), so TMPDIR); until 2026-10-22 a forced
# `[env] TMPDIR = "/tmp"` in .cargo/config.toml kept them in /tmp for
# speed, which also overrode a TMPDIR set for any cargo test.
# A mutant only needs some test to fail, so proptest doesn't shrink.
#
# Every mutation recipe takes a shard, "k/n" (default: all of it), so CI
# can spread a crate over parallel jobs. Test timeouts are three times the
# run's own baseline, never under the floor given: a slower machine gets
# longer ones instead of false timeouts.
mutants_tmp := env_var_or_default("XDG_CACHE_HOME", env_var("HOME") + "/.cache") + "/wanderbooks/mutants"

# Mutation testing of the core: do the tests catch planted bugs?
mutants shard="0/1": sweep-test-tmp
    # Core and store tests (the store's replay tests reach Ledger::replay_*);
    # the whole workspace would rebuild the viewer for every mutant.
    mkdir -p {{mutants_tmp}}
    PROPTEST_MAX_SHRINK_ITERS=0 TMPDIR={{mutants_tmp}} cargo mutants -p ledger-core --exclude crates/ledger-core/src/proofs.rs --test-package ledger-core --test-package ledger-store -j 4 --timeout-multiplier 3 --minimum-test-timeout 120 --shard {{shard}}
    @just sweep-test-tmp

# Mutation testing of the store. Fewer random cases per mutant (each needs
# a run that catches it, not the deep sweep).
mutants-store shard="0/1": sweep-test-tmp
    mkdir -p {{mutants_tmp}}
    STORE_CASES=64 PROPTEST_MAX_SHRINK_ITERS=0 TMPDIR={{mutants_tmp}} cargo mutants -p ledger-store --test-package ledger-store -j 3 --timeout-multiplier 3 --minimum-test-timeout 300 --shard {{shard}} -o mutants-store
    @just sweep-test-tmp

# Mutation testing of the importers and report renderers.
mutants-import shard="0/1": sweep-test-tmp
    mkdir -p {{mutants_tmp}}
    PROPTEST_MAX_SHRINK_ITERS=0 TMPDIR={{mutants_tmp}} cargo mutants -p ledger-import --test-package ledger-import -j 4 --timeout-multiplier 3 --minimum-test-timeout 300 --shard {{shard}} -o mutants-import
    @just sweep-test-tmp

# Mutation testing of the settings parser and the receipt policy.
mutants-config shard="0/1": sweep-test-tmp
    mkdir -p {{mutants_tmp}}
    PROPTEST_MAX_SHRINK_ITERS=0 TMPDIR={{mutants_tmp}} cargo mutants -p ledger-config --test-package ledger-config -j 4 --timeout-multiplier 3 --minimum-test-timeout 120 --shard {{shard}} -o mutants-config
    @just sweep-test-tmp

# Mutation testing of the MCP server: every mutant runs its unit tests and
# the suites that drive a real books server (ledger-cli's tests, D-44),
# built without the viewer; fewer random cases each, as for the store.
mutants-mcp shard="0/1": sweep-test-tmp
    mkdir -p {{mutants_tmp}}
    MCP_CASES=16 PROPTEST_MAX_SHRINK_ITERS=0 TMPDIR={{mutants_tmp}} cargo mutants -p ledger-mcp --test-package ledger-mcp --test-package ledger-cli --no-default-features -j 4 --timeout-multiplier 3 --minimum-test-timeout 600 --shard {{shard}} -o mutants-mcp
    @just sweep-test-tmp

# No mutation runs on the viewer (owner, 2026-10-18: they crushed the
# machine, and for the GUI it's "not as critical"). Its two musts are
# tested directly instead: it never writes the books (tests/read_only.rs)
# and every figure it shows is the engine's (tests/view_model.rs, against
# ledger-core on random books). Mutation recipes for other crates leave
# crates/ledger-gui out.

# Benchmarks of the core's hot paths (no budget gates them yet)
bench:
    cargo bench -p ledger-core --bench books
    cargo bench -p ledger-store --bench store

# Build the release binaries (`wanderbooks`, the one executable, and the
# viewer's own)
build:
    cargo build --release -p ledger-cli -p ledger-gui

# Install the binaries from this checkout into ~/.cargo/bin
install:
    for c in ledger-cli ledger-gui; do cargo install --locked --path crates/$c; done

# Open the GUI on the demo company
demo:
    cargo run --release -p ledger-gui -- --demo

# Regenerate the website's screenshots (dark) from the demo company
site-shots:
    #!/usr/bin/env bash
    set -euo pipefail
    tmp=$(mktemp -d)
    SHOT_DIR="$tmp" SHOT_DARK=1 SHOT_LIVE=1 SHOT_ONLY=overview,pnl,cash cargo test -p ledger-gui --test screens
    # The hero's viewer pane: taller, to sit beside the session.
    SHOT_DIR="$tmp" SHOT_DARK=1 SHOT_LIVE=1 SHOT_ONLY=entry SHOT_ENTRY=245 SHOT_HEIGHT=1150 cargo test -p ledger-gui --test screens
    for f in overview entry pnl cash; do cp "$tmp/$f.png" site/assets/; done
    rm -f site/assets/overview-dark.png
    rm -rf "$tmp"

# Assemble the website in site/dist (index.html, install.sh, assets)
site:
    rm -rf site/dist && mkdir -p site/dist
    cp site/index.html install.sh site/dist/
    cp -r site/assets site/dist/

# Deploy the website to wanderbooks.ai (Netlify; `netlify login` once first)
deploy: site
    {{netlify}} deploy --prod --dir site/dist --no-build

# Deploy a preview of the website and print its URL
deploy-preview: site
    {{netlify}} deploy --dir site/dist --no-build
