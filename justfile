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
    # A day, not an hour: a working session in a parallel worktree keeps its
    # test roots (and a running test's data root) for hours, and sweeping
    # another engineer's roots mid-run fails their run for no reason.
    find "${TMPDIR:-/tmp}" /tmp -maxdepth 1 -user "$(id -u)" -type d -mmin +1440 \( -name 'wb-*' -o -name 'wanderbooks-*' -o -name 'ledger-*' \) -exec rm -rf {} + 2>/dev/null || true
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
# four jobs, a CI runner's. Every other harness in src/proofs/ runs in `prove`, a new one
# included; a name here that isn't a harness, or is shared by two, fails
# both (docs/verification.md).
kani_slow := "accounting_equation cumulative_close_never_drifts_a_unit_at_near_thirds cumulative_closing_entry_balances deductible_amount_rounds_once discount_max_is_bounded entry_flows_pure_are_movements entry_flows_sum_to_cash prorate_parts_sum_to_total sums_are_exact the_last_months_charge_is_within_half_a_cent"

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
    echo "Lean: $(wc -l <<<"$audit") theorems, standard axioms only"

# The slow Kani harnesses (kani_slow), each under its own 30-minute cap, $(nproc) at a time (CI runs it beside prove)
prove-slow: (_kani "slow")

# Every proof: prove, then prove-slow
prove-all: prove prove-slow

# Every Kani harness, its module path, and the set (fast/slow) it runs in:
# no verification, so the slow list is shown resolved without `prove-slow`.
prove-list: (_kani "list")

# Kani on one set of the harnesses: "fast" (all but kani_slow, the whole
# run under the cap) or "slow" (each harness under the cap); "list" prints
# the harnesses and the set each runs in, without running Kani (prove-list).
_kani set:
    #!/usr/bin/env bash
    set -euo pipefail
    dir=crates/ledger-core/src/proofs
    # Every harness, fully qualified: proofs/mod.rs is module `proofs`,
    # proofs/<unit>.rs is `proofs::<unit>`. Kani's --exact matches only a
    # fully qualified name, so a harness resolves wherever its file lives.
    files=("$dir/mod.rs")
    for f in "$dir"/*.rs; do [ "$f" = "$dir/mod.rs" ] || files+=("$f"); done
    all=$(
        for f in "${files[@]}"; do
            mod=proofs
            [ "$f" = "$dir/mod.rs" ] || mod="proofs::$(basename "$f" .rs)"
            awk -v mod="$mod" '/#\[kani::proof\]/ { p = 1; next } p && match($0, /^fn [a-z_0-9]+/) { print mod "::" substr($0, 4, RLENGTH - 3); p = 0 }' "$f"
        done | sort
    )
    bare=$(sed 's/.*:://' <<<"$all")
    slow=$(tr ' ' '\n' <<<"{{kani_slow}}" | sort)
    # Two harnesses with one name would make kani_slow (a list of bare
    # names) ambiguous.
    if dup=$(uniq -d <<<"$bare") && [ -n "$dup" ]; then
        echo "two harnesses share a name: $dup" >&2
        exit 1
    fi
    # A slow name that no harness has is a typo or a moved proof: fail
    # rather than silently verify one harness less than the list says.
    if missing=$(comm -13 <(echo "$bare") <(echo "$slow")) && [ -n "$missing" ]; then
        echo "kani_slow names no harness: $missing" >&2
        exit 1
    fi
    # The qualified name whose bare name is in `want` (a slow/fast file).
    picked() { # want-file
        awk 'NR==FNR { want[$1] = 1; next } { n = $0; sub(/.*::/, "", n); if (n in want) print }' "$1" <(printf '%s\n' "$all")
    }
    if [ "{{set}}" = list ]; then
        for h in $all; do
            n=${h##*::}
            if grep -qxF "$n" <<<"$slow"; then echo "slow  $h"; else echo "fast  $h"; fi
        done
        exit 0
    fi
    if [ "{{set}}" = slow ]; then names=$(picked <(echo "$slow")); else names=$(picked <(comm -23 <(echo "$bare") <(echo "$slow"))); fi
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
        for h in $names; do args+=(--harness "$h"); done
        timeout 1800 cargo kani -j "$(nproc)" --output-format terse --exact "${args[@]}" | tee "$logs/fast.log"
        checked "$logs/fast.log" "$(wc -l <<<"$names")"
    else
        cargo kani --only-codegen --output-format terse
        failed=0
        for h in $names; do
            log=${h##*::}
            while [ "$(jobs -rp | wc -l)" -ge "$(nproc)" ]; do wait -n || failed=1; done
            ( start=$(date +%s)
              timeout 1800 cargo kani --output-format terse --exact --harness "$h" > "$logs/$log.log" 2>&1 &&
                  checked "$logs/$log.log" 1 &&
                  echo "Kani: $log verified in $(( $(date +%s) - start )) s" ||
                  { echo "Kani: $log failed or ran past 1800 s ($logs/$log.log)" >&2; exit 1; } ) &
        done
        while [ "$(jobs -rp | wc -l)" -gt 0 ]; do wait -n || failed=1; done
        wait || failed=1
        [ "$failed" = 0 ] || exit 1
    fi
    rm -rf "$logs"
    echo "Kani ({{set}}): $(wc -l <<<"$names") harnesses verified"

# Random books through wanderbooks and GnuCash 5.10, compared exactly (Docker)
# The oracle's Dockerfile and runner live in bookspec (`gnucash/oracle`,
# image `bookspec-gnucash-oracle`): one source of truth. This builds that
# image from the pinned bookspec checkout (the rev in ledger-core's
# Cargo.toml), so no copy is kept here.
oracle:
    #!/usr/bin/env bash
    set -euo pipefail
    # The oracle's container lives in bookspec (one source of truth): find
    # the pinned checkout through cargo itself, whatever its directory name.
    cargo fetch --locked
    manifest=$(cargo metadata --format-version 1 --locked \
        | jq -r '.packages[] | select(.name == "bookspec-checker") | .manifest_path' | head -1)
    [ -n "$manifest" ] || { echo "oracle: bookspec-checker is not a dependency of this workspace" >&2; exit 1; }
    root=${manifest%/checker/Cargo.toml}
    [ -f "$root/gnucash/oracle/Dockerfile" ] || {
        echo "oracle: the pinned bookspec ($root) has no gnucash/oracle; move the pin to a bookspec rev that includes it (P14 unit S-K1)" >&2
        exit 1
    }
    docker build -q -t bookspec-gnucash-oracle "$root/gnucash/oracle"
    cargo test -p ledger-core --test oracle_gnucash -- --ignored --nocapture

# The Rust ledger against the Lean model of it, on random command sequences
# Each sequence is run through the engine, written as a bookspec example whose
# expected values are the engine's answers, and checked by the Lean model
# (`lake exe bookspec-check`, built from the pinned bookspec): a mismatch is the
# engine and the model disagreeing. `cases` sequences, one `lake` call for all of
# them (crates/ledger-core/tests/bookspec/lean_model.rs); fails without lake.
lean-model cases="64":
    WANDERBOOKS_REQUIRE_LEAN=1 PROPTEST_CASES={{cases}} cargo test -p ledger-core --test bookspec lean_model -- --nocapture

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

# Line, region and branch coverage of the core and the store, from their
# own test suites (nightly's -Zcoverage-options=branch, so every `if` has a
# taken/not-taken record too). HTML and a plain per-file table land in
# target/coverage/<crate>/; docs/coverage-floor.md records the totals and
# `coverage-check` guards them. Not a push gate (it is slow), but the
# weekly scheduled workflow runs it. `src/proofs.rs` is Kani-only (cfg(kani))
# and is excluded, as are test and bench files: this measures the library.
coverage crate="all":
    #!/usr/bin/env bash
    set -euo pipefail
    # The shared sccache wrapper (~/.cargo/config.toml) would hand rustc to
    # whatever toolchain it finds, and a `RUSTUP_TOOLCHAIN` in the caller's
    # environment (1.99.1 in a session here) would beat `+nightly` for the
    # rustc cargo-llvm-cov invokes. Both are overridden explicitly: branch
    # coverage needs the pinned nightly. The build is its own target dir
    # (`target/llvm-cov-target`), so sccache would not be shared anyway.
    export RUSTC_WRAPPER=
    export RUSTUP_TOOLCHAIN=nightly-2026-10-01
    # Name the compiler itself: the mise shim around cargo-llvm-cov can put
    # its pinned stable toolchain back, and a bare `rustc` then loses the
    # nightly flag. Call the binary directly, as `fuzz` does with cargo-fuzz.
    export RUSTC="$(rustup which --toolchain "$RUSTUP_TOOLCHAIN" rustc)"
    llvm_cov=$(mise which cargo-llvm-cov)
    mkdir -p target/coverage
    # Every workspace member but the crate under measurement: a dev-dependency
    # (the core's tests use the store, the store's use the core) is compiled
    # instrumented too, and its files must not enter this crate's totals.
    others() {
        local keep=$1 member
        local list=()
        for member in ledger-core ledger-store ledger-config ledger-mcp \
            ledger-import ledger-reader ledger-cli ledger-gui ledger-vault \
            ledger-host ledger-package; do
            [ "$member" = "$keep" ] || list+=("${member#ledger-}")
        done
        local IFS='|'
        echo "crates/ledger-(${list[*]})/"
    }
    run() { # crate
        local crate=$1 dir="target/coverage/$1"
        local ignore="$(others "$crate")|/(tests|benches|examples)/|/src/proofs\\.rs$"
        mkdir -p "$dir"
        # One instrumented test run: the HTML comes from it; the JSON and the
        # table are regenerated from the same profiles, cheaply.
        # `--failure-mode all`: the store's tests start and kill server
        # processes, which inherit LLVM_PROFILE_FILE; a SIGKILLed child leaves
        # a truncated profraw that llvm-profdata refuses. That file has no
        # test coverage in it, so it is tolerated; the test binaries' own
        # profiles merge normally.
        "$llvm_cov" llvm-cov -p "$crate" --branch --failure-mode all \
            --ignore-filename-regex "$ignore" \
            --html --output-dir "$dir"
        "$llvm_cov" llvm-cov report --failure-mode all \
            --ignore-filename-regex "$ignore" \
            --json --output-path "$dir/coverage.json"
        "$llvm_cov" llvm-cov report --failure-mode all \
            --ignore-filename-regex "$ignore" > "$dir/summary.txt"
        echo "coverage: $crate written to $dir/html and $dir/summary.txt"
        awk -v crate="$crate" '$1 == "TOTAL" { printf "coverage: %s lines %s, regions %s, branches %s\n", crate, $10, $4, $13 }' "$dir/summary.txt"
    }
    case "{{crate}}" in
        all) run ledger-core; run ledger-store ;;
        ledger-core | ledger-store) run "{{crate}}" ;;
        *) echo "coverage: unknown crate '{{crate}}' (all, ledger-core, ledger-store)" >&2; exit 2 ;;
    esac
    echo "coverage: floors in docs/coverage-floor.md; check them with \`just coverage-check\`"

# The coverage floor: fails when either crate's total line coverage is more
# than 0.5 percentage points below docs/coverage-floor.md's recorded number
# (reads the last `just coverage` report; it does not rerun the tests).
coverage-check:
    @scripts/coverage-floor.sh

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

# Forwarded to every mutation run: "--iterate" resumes the output directory a
# cut-off run left behind (skip caught and unviable mutants, retest the rest,
# see docs/testing.md "Running P12 step 2"). Empty for a fresh run.
mutants_iterate := env_var_or_default("P12_MUTANTS_ITERATE", "")

# Mutation testing of the core: do the tests catch planted bugs?
mutants shard="0/1": sweep-test-tmp
    # Core and store tests (the store's replay tests reach Ledger::replay_*);
    # the whole workspace would rebuild the viewer for every mutant.
    mkdir -p {{mutants_tmp}}
    PROPTEST_MAX_SHRINK_ITERS=0 TMPDIR={{mutants_tmp}} cargo mutants -p ledger-core --exclude 'crates/ledger-core/src/proofs/**' --test-package ledger-core --test-package ledger-store -j 4 --timeout-multiplier 3 --minimum-test-timeout 120 --shard {{shard}} {{mutants_iterate}}
    @just sweep-test-tmp

# Mutation testing of the store. Fewer random cases per mutant (each needs
# a run that catches it, not the deep sweep).
mutants-store shard="0/1": sweep-test-tmp
    mkdir -p {{mutants_tmp}}
    STORE_CASES=64 PROPTEST_MAX_SHRINK_ITERS=0 TMPDIR={{mutants_tmp}} cargo mutants -p ledger-store --test-package ledger-store -j 3 --timeout-multiplier 3 --minimum-test-timeout 300 --shard {{shard}} {{mutants_iterate}} -o mutants-store
    @just sweep-test-tmp

# Mutation testing of the importers and report renderers.
mutants-import shard="0/1": sweep-test-tmp
    mkdir -p {{mutants_tmp}}
    PROPTEST_MAX_SHRINK_ITERS=0 TMPDIR={{mutants_tmp}} cargo mutants -p ledger-import --test-package ledger-import -j 4 --timeout-multiplier 3 --minimum-test-timeout 300 --shard {{shard}} {{mutants_iterate}} -o mutants-import
    @just sweep-test-tmp

# Mutation testing of the settings parser and the receipt policy.
mutants-config shard="0/1": sweep-test-tmp
    mkdir -p {{mutants_tmp}}
    PROPTEST_MAX_SHRINK_ITERS=0 TMPDIR={{mutants_tmp}} cargo mutants -p ledger-config --test-package ledger-config -j 4 --timeout-multiplier 3 --minimum-test-timeout 120 --shard {{shard}} {{mutants_iterate}} -o mutants-config
    @just sweep-test-tmp

# Mutation testing of the MCP server: every mutant runs its unit tests and
# the suites that drive a real books server (ledger-cli's tests, D-44),
# built without the viewer; fewer random cases each, as for the store.
mutants-mcp shard="0/1": sweep-test-tmp
    mkdir -p {{mutants_tmp}}
    MCP_CASES=16 PROPTEST_MAX_SHRINK_ITERS=0 TMPDIR={{mutants_tmp}} cargo mutants -p ledger-mcp --test-package ledger-mcp --test-package ledger-cli --no-default-features -j 4 --timeout-multiplier 3 --minimum-test-timeout 600 --shard {{shard}} {{mutants_iterate}} -o mutants-mcp
    @just sweep-test-tmp

# Mutation testing of the host (the books home, the lease, the update channel, the bridge): every mutant runs its own tests and the
# suites that drive a real books server (ledger-cli's, D-44), built without
# the viewer, as for the MCP server. A local run, never CI (D-59).
mutants-host shard="0/1": sweep-test-tmp
    mkdir -p {{mutants_tmp}}
    PROPTEST_MAX_SHRINK_ITERS=0 TMPDIR={{mutants_tmp}} cargo mutants -p ledger-host --test-package ledger-host --test-package ledger-cli --no-default-features -j 4 --timeout-multiplier 3 --minimum-test-timeout 300 --shard {{shard}} {{mutants_iterate}} -o mutants-host
    @just sweep-test-tmp

# Mutation testing of the CPA package and the 1099 worksheet: every mutant runs its own tests and the
# suites that drive a real books server (ledger-cli's, D-44), built without
# the viewer, as for the MCP server. A local run, never CI (D-59).
mutants-package shard="0/1": sweep-test-tmp
    mkdir -p {{mutants_tmp}}
    PROPTEST_MAX_SHRINK_ITERS=0 TMPDIR={{mutants_tmp}} cargo mutants -p ledger-package --test-package ledger-package --test-package ledger-cli --no-default-features -j 4 --timeout-multiplier 3 --minimum-test-timeout 300 --shard {{shard}} {{mutants_iterate}} -o mutants-package
    @just sweep-test-tmp

# Mutation testing of the TIN vault: every mutant runs its own tests and the
# suites that drive a real books server (ledger-cli's, D-44), built without
# the viewer, as for the MCP server. A local run, never CI (D-59).
mutants-vault shard="0/1": sweep-test-tmp
    mkdir -p {{mutants_tmp}}
    PROPTEST_MAX_SHRINK_ITERS=0 TMPDIR={{mutants_tmp}} cargo mutants -p ledger-vault --test-package ledger-vault --test-package ledger-cli --no-default-features -j 4 --timeout-multiplier 3 --minimum-test-timeout 120 --shard {{shard}} {{mutants_iterate}} -o mutants-vault
    @just sweep-test-tmp

# Mutation testing of the one executable (its argument handling, the server and bridge modes): every mutant runs its own tests and the
# suites that drive a real books server (ledger-cli's, D-44), built without
# the viewer, as for the MCP server. A local run, never CI (D-59).
mutants-cli shard="0/1": sweep-test-tmp
    mkdir -p {{mutants_tmp}}
    PROPTEST_MAX_SHRINK_ITERS=0 TMPDIR={{mutants_tmp}} cargo mutants -p ledger-cli --test-package ledger-cli --no-default-features -j 4 --timeout-multiplier 3 --minimum-test-timeout 600 --shard {{shard}} {{mutants_iterate}} -o mutants-cli
    @just sweep-test-tmp

# P12 step 2: start the resumable overnight run, detached (D-66, docs/testing.md)
p12-night:
    @scripts/p12-night.sh start

# P12 step 2: print the summary and whether a runner is live
p12-status:
    @scripts/p12-night.sh status

# P12 step 2: stop a live runner the way the 07:00 deadline does
p12-stop:
    @scripts/p12-night.sh stop

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

# The Linux packages (P11a step 3), in target/packages/VERSION. Builds the
# release program first; needs the tools `mise install` pins. A VERSION other
# than the program's repackages the same program under it (the upgrade
# test's second release).
# Build the AppImage, .deb, pacman tarball and .rpm
package-linux version="" formats="deb,appimage,pacman":
    cargo build --release -p ledger-cli
    deploy/packaging/build-linux.sh {{version}} {{formats}}

# Build the unsigned macOS .dmg (on a Mac; CI does it on a tag or dispatch)
package-macos version="":
    cargo build --release -p ledger-cli
    deploy/packaging/build-macos.sh {{version}}

# A release folder on this machine signed with a throwaway key; then
# `wanderbooks update --feed file://DIR/manifest.json --trusted-key DIR/release.pub`
# Make a local release feed to try `wanderbooks update` against
local-release file version dir:
    deploy/packaging/local-release.sh {{file}} {{version}} {{dir}}

# A program linked here does not start on Debian (docs/packaging.md); this
# builds it in rust:1.99-trixie, into target/container. Needs docker.
# Build the release program in a Debian trixie container
release-trixie:
    mkdir -p target/container
    docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp -e CARGO_HOME=/tmp/cargo-home -e CARGO_TARGET_DIR=/target -v "$PWD:/src:ro" -v "$PWD/target/container:/target" -w /src rust:1.99-trixie cargo build --release --locked -p ledger-cli

# Install each Linux package in a clean Arch, Debian and Fedora container,
# open the books, upgrade to the next release, and check the books' head is
# unchanged and the open books restarted onto the new program. Builds the
# program in a trixie container, then releases 0.1.0 and 0.1.1 of it.
# Install and upgrade the Linux packages in Arch, Debian and Fedora containers
test-linux-packages: release-trixie
    WB_PROGRAM=target/container/release/wanderbooks deploy/packaging/build-linux.sh 0.1.0 deb,pacman
    WB_PROGRAM=target/container/release/wanderbooks deploy/packaging/build-linux.sh 0.1.1 deb,pacman
    deploy/packaging/test-linux-packages.sh
