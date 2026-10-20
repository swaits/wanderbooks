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
# clippy and its tests, and links no window libraries
server-only:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo clippy -p ledger-cli --no-default-features --all-targets -- -D warnings
    cargo test -p ledger-cli --no-default-features
    tree=$(cargo tree -p ledger-cli --no-default-features -e normal)
    if grep -qiE '\b(eframe|egui|winit|wgpu|glow|ledger-gui)\b' <<<"$tree"; then
        echo "the server-only build links a window library" >&2
        exit 1
    fi
    echo "server-only build: no window libraries"

# Test books go in the system temp dir (often RAM). Every test's dir is
# removed when the test ends, pass or panic (a guard dropped on unwind;
# until 2026-10-17 many were removed only on pass, or never: 117,000 piled
# up). A run killed mid-test (a mutant's timeout, ^C) runs no destructor,
# so this sweep removes this user's test book dirs older than an hour; a
# run in progress elsewhere keeps its own.
sweep-test-tmp:
    @find "${TMPDIR:-/tmp}" /tmp -maxdepth 1 -user "$(id -u)" -type d -mmin +60 \( -name 'wb-*' -o -name 'wanderbooks-*' -o -name 'ledger-*' \) -exec rm -rf {} + 2>/dev/null || true

# Formal proofs: Kani on the accounting kernel, Lean for books of any size (docs/verification.md)
prove:
    #!/usr/bin/env bash
    set -euo pipefail
    # Capped: a harness that blows up should fail, not eat the machine.
    log=$(mktemp)
    (ulimit -v 12000000; cd crates/ledger-core &&
        timeout 1800 cargo kani -j "$(nproc)" --output-format terse | tee "$log")
    # A cover that can't be satisfied means a proof is vacuous.
    if grep -E "\*\* [0-9]+ of [0-9]+ cover properties satisfied" "$log" |
        awk '{ if ($2 != $4) bad = 1 } END { exit !bad }'; then
        echo "Kani: an unsatisfiable cover; some proof is vacuous" >&2
        exit 1
    fi
    rm -f "$log"
    cd verify/lean
    lake build
    audit=$(lake env lean Audit.lean)
    if grep -v -E "depends on axioms: \[(propext|Quot.sound|Classical.choice)(, (propext|Quot.sound|Classical.choice))*\]$|does not depend on any axioms" <<<"$audit"; then
        echo "Lean audit: a theorem rests on something beyond the standard axioms" >&2
        exit 1
    fi
    echo "Lean: $(wc -l <<<"$audit") headline theorems, standard axioms only"

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

# Mutation testing of the MCP server (every mutant runs the stdio suites;
# fewer random cases each, as for the store).
mutants-mcp shard="0/1": sweep-test-tmp
    mkdir -p {{mutants_tmp}}
    MCP_CASES=16 PROPTEST_MAX_SHRINK_ITERS=0 TMPDIR={{mutants_tmp}} cargo mutants -p ledger-mcp --test-package ledger-mcp -j 4 --timeout-multiplier 3 --minimum-test-timeout 600 --shard {{shard}} -o mutants-mcp
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

# Build the release binaries (CLI, MCP server, GUI)
build:
    cargo build --release -p ledger-cli -p ledger-mcp -p ledger-gui

# Install the binaries from this checkout into ~/.cargo/bin
install:
    for c in ledger-cli ledger-mcp ledger-gui; do cargo install --locked --path crates/$c; done

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
