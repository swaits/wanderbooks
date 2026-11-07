#!/usr/bin/env bash
# P12 step 2: the resumable overnight test run (D-66, docs/testing.md).
#
# `just p12-night` starts this detached. It walks one queue of steps, skips
# those whose done marker exists, rewrites the summary after each step, and
# stops at P12_STOP so the machine is free in the morning. Every step's
# log and status live under ${XDG_CACHE_HOME:-$HOME/.cache}/wanderbooks/p12.
set -euo pipefail

self=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")
repo=$(cd "$(dirname "$self")/.." && pwd)
state=${XDG_CACHE_HOME:-$HOME/.cache}/wanderbooks/p12
logs=$state/logs
steps_dir=$state/steps
done_dir=$state/done
pidfile=$state/pid
step_pidfile=$state/step.pid

fuzz_seconds=${P12_FUZZ_SECONDS:-3600}
dry_sleep=${P12_DRY_SLEEP:-2}

queue=(
    fuzz
    prove-all
    oracle
    props-max
    mutants
    mutants-store
    mutants-import
    mutants-config
    mutants-mcp
    mutants-host
    mutants-package
    mutants-vault
    mutants-cli
)

# The mutation recipes and the output directory each writes (the core
# recipe uses cargo-mutants' default; the rest pass -o <recipe name>).
mutation_out() {
    case "$1" in
        mutants) echo mutants.out ;;
        *) echo "$1" ;;
    esac
}

iso() { date '+%Y-%m-%d %H:%M:%S'; }

ensure_dirs() {
    mkdir -p "$logs" "$steps_dir" "$done_dir" "$state/mutants"
}

read_field() { # file key
    sed -n "s/^$2=//p" "$1" 2>/dev/null | head -1
}

record_result() { # step text
    printf '%s\n' "$2" > "$steps_dir/$1.result"
}

record_details() { # step (stdin)
    cat > "$steps_dir/$1.details"
}

write_status() { # step status start end secs result
    {
        printf 'status=%s\n' "$2"
        printf 'start=%s\n' "$3"
        printf 'end=%s\n' "$4"
        printf 'secs=%s\n' "$5"
        printf 'result=%s\n' "$6"
    } > "$steps_dir/$1.status"
}

runner_alive() {
    [ -f "$pidfile" ] || return 1
    local pid args
    pid=$(cat "$pidfile" 2>/dev/null || true)
    [ -n "$pid" ] || return 1
    kill -0 "$pid" 2>/dev/null || return 1
    args=$(ps -o args= -p "$pid" 2>/dev/null || true)
    [[ $args == *p12-night.sh* ]] || return 1
    return 0
}

validate_selection() {
    if [ -z "${P12_STEPS:-}" ]; then
        return 0
    fi
    local step
    for step in ${P12_STEPS}; do
        case " ${queue[*]} " in
            *" $step "*) ;;
            *)
                echo "P12: unknown step '$step' (P12_STEPS); known: ${queue[*]}" >&2
                return 2
                ;;
        esac
    done
}

selected_steps() {
    if [ -n "${P12_STEPS:-}" ]; then
        # Word splitting is the hook's contract: a space-separated list.
        # shellcheck disable=SC2206
        local picked=(${P12_STEPS})
        printf '%s\n' "${picked[@]}"
    else
        printf '%s\n' "${queue[@]}"
    fi
}

compute_deadline() {
    local stop=${P12_STOP:-07:00}
    local target now
    now=$(date +%s)
    if ! target=$(date -d "today $stop" +%s 2>/dev/null); then
        echo "P12: P12_STOP '$stop' is not a time this date understands" >&2
        return 2
    fi
    if [ "$target" -le "$now" ]; then
        target=$(date -d "tomorrow $stop" +%s)
    fi
    printf '%s\n' "$target"
}

write_summary() {
    local state_word started deadline step status start end secs result
    state_word=$(cat "$state/run.state" 2>/dev/null || echo "not started")
    started=$(cat "$state/started" 2>/dev/null || echo "-")
    deadline=$(cat "$state/deadline.iso" 2>/dev/null || echo "-")
    {
        echo "# P12 night"
        echo
        echo "- state: $state_word"
        echo "- started: $started"
        echo "- deadline: $deadline"
        echo "- state dir: \`$state\`"
        echo
        echo "| step | status | start | end | duration | result |"
        echo "|---|---|---|---|---|---|"
        for step in "${queue[@]}"; do
            if [ -f "$done_dir/$step" ]; then
                status="done"
            elif [ -f "$steps_dir/$step.status" ]; then
                status=$(read_field "$steps_dir/$step.status" status)
            else
                status="not started"
            fi
            if [ -f "$steps_dir/$step.status" ]; then
                start=$(read_field "$steps_dir/$step.status" start)
                end=$(read_field "$steps_dir/$step.status" end)
                secs=$(read_field "$steps_dir/$step.status" secs)
                result=$(read_field "$steps_dir/$step.status" result)
            else
                start="-"
                end="-"
                secs=""
                result=""
            fi
            [ -n "$start" ] || start="-"
            [ -n "$end" ] || end="-"
            if [ -n "$secs" ]; then
                secs="${secs}s"
            else
                secs="-"
            fi
            result=${result//|/\\|}
            printf '| %s | %s | %s | %s | %s | %s |\n' \
                "$step" "$status" "$start" "$end" "$secs" "$result"
        done
        if [ -f "$steps_dir/fuzz.details" ]; then
            echo
            echo "## Fuzzing, per target"
            echo
            cat "$steps_dir/fuzz.details"
        fi
        if [ -f "$steps_dir/props-max.details" ]; then
            echo
            echo "## Property maxima"
            echo
            cat "$steps_dir/props-max.details"
        fi
    } > "$state/summary.md.tmp"
    mv "$state/summary.md.tmp" "$state/summary.md"
}

# ---------------------------------------------------------------- step bodies

dry_result() {
    local step=$1 target
    case "$step" in
        fuzz)
            {
                for f in "$repo"/fuzz/fuzz_targets/*.rs; do
                    target=$(basename "$f" .rs)
                    echo "- $target: dry run, 0 executions, no crashes"
                done
            } | record_details fuzz
            record_result fuzz "dry run: every target ran for a fake moment"
            ;;
        mutants|mutants-store|mutants-import|mutants-config|mutants-mcp|mutants-host|mutants-package|mutants-vault|mutants-cli)
            record_result "$step" "dry run: caught 0, missed 0, timeout 0, unviable 0"
            ;;
        props-max)
            echo "- dry run: the property commands were not run" | record_details props-max
            record_result props-max "dry run: property maxima"
            ;;
        *)
            record_result "$step" "dry run: $step ok"
            ;;
    esac
}

run_fuzz() {
    local build_log=$logs/fuzz-build.log
    local fuzz_bin target
    if ! fuzz_bin=$(mise which cargo-fuzz 2>>"$build_log"); then
        record_result fuzz "failed: cargo-fuzz not found (see logs/fuzz-build.log)"
        return 1
    fi
    echo "building every fuzz target (one build)" >&2
    if ! (cd "$repo/fuzz" && RUSTUP_TOOLCHAIN=nightly-2026-10-01 "$fuzz_bin" build) >>"$build_log" 2>&1; then
        record_result fuzz "failed: the fuzz-target build failed (see logs/fuzz-build.log)"
        return 1
    fi

    local -a names=() pids=()
    local f
    for f in "$repo"/fuzz/fuzz_targets/*.rs; do
        target=$(basename "$f" .rs)
        names+=("$target")
        # Each target through the existing recipe, all side by side.
        just --justfile "$repo/justfile" fuzz "$target" "$fuzz_seconds" \
            > "$logs/fuzz-$target.log" 2>&1 &
        pids+=("$!")
    done
    if [ "${#names[@]}" -eq 0 ]; then
        record_result fuzz "failed: no fuzz targets found"
        return 1
    fi

    local i rc runs crash_list crash_count total=0 crashes=0 failed=0
    local details=$state/steps/fuzz.details.tmp
    : > "$details"
    for i in "${!pids[@]}"; do
        target=${names[$i]}
        rc=0
        wait "${pids[$i]}" || rc=$?
        runs=$(grep -oE 'stat::number_of_executed_units: *[0-9]+' "$logs/fuzz-$target.log" |
            tail -1 | grep -oE '[0-9]+' || true)
        if [ -z "$runs" ]; then
            runs=$(grep -oE 'Done [0-9]+ runs' "$logs/fuzz-$target.log" |
                tail -1 | grep -oE '[0-9]+' || true)
        fi
        runs=${runs:-0}
        total=$((total + runs))
        crash_list=$(find "$repo/fuzz/artifacts/$target" -mindepth 1 -maxdepth 1 -type f 2>/dev/null |
            sort || true)
        crash_count=$(printf '%s\n' "$crash_list" | grep -c . || true)
        crashes=$((crashes + crash_count))
        {
            echo "- $target: $runs executions"
            if [ "$crash_count" -gt 0 ]; then
                echo "  - crash files:"
                while IFS= read -r crash; do
                    [ -n "$crash" ] || continue
                    echo "    - \`$crash\`"
                done <<<"$crash_list"
            fi
        } >> "$details"
        if [ "$rc" -ne 0 ] || [ "$crash_count" -gt 0 ]; then
            failed=1
        fi
    done
    mv "$details" "$state/steps/fuzz.details"

    if [ "$failed" -eq 0 ]; then
        record_result fuzz "${#names[@]} targets, $total executions, no crashes"
        return 0
    fi
    record_result fuzz "${#names[@]} targets, $total executions, $crashes crash files (see the details below)"
    return 1
}

run_oracle() {
    if ! docker info >/dev/null 2>&1; then
        record_result oracle "failed: docker info failed; Docker is not available to this user"
        return 1
    fi
    if ! just --justfile "$repo/justfile" oracle; then
        record_result oracle "failed: just oracle failed (see the log)"
        return 1
    fi
    record_result oracle "just oracle passed"
}

run_props_max() {
    # The maxima docs/testing.md names, with the tests those maxima name.
    local -a cmds=(
        "STORE_CASES=5000 cargo test -p ledger-store --test store refused_commands_never_reach_the_journal"
        "WB_ORDERS_DEPTH=6 cargo test -p ledger-store --test asset_orders every_order_of_runs_reversals_disposals_and_closes_keeps_the_books_right"
        "CF_CASES=5000 cargo test -p ledger-core --test spec_cash_flow"
        "PROPTEST_CASES=5000 cargo test -p ledger-core --test spec_party_totals"
        "PROPTEST_CASES=2000 cargo test -p ledger-store --test settings settings_replay_is_pure"
        "PROPTEST_CASES=2000 cargo test -p ledger-core --test form1099 party_totals_are_the_1099s_buckets"
    )
    local details=$state/steps/props-max.details.tmp
    local cmd rc failed=0
    : > "$details"
    for cmd in "${cmds[@]}"; do
        echo "\$ $cmd" >> "$details"
        rc=0
        eval "$cmd" || rc=$?
        if [ "$rc" -eq 0 ]; then
            echo "  ok" >> "$details"
        else
            echo "  FAILED" >> "$details"
            failed=1
        fi
    done
    mv "$details" "$state/steps/props-max.details"
    if [ "$failed" -eq 0 ]; then
        record_result props-max "property tests at their documented maxima passed"
        return 0
    fi
    record_result props-max "failed: a property-maximum command failed (see the details below)"
    return 1
}

run_mutants() {
    local step=$1 out dir dest rc=0
    out=$(mutation_out "$step")
    # Never let an empty name turn the rm below into one of the repo.
    [ -n "$out" ] || { echo "P12: no output directory for $step" >&2; return 1; }
    dir=$repo/$out
    dest=$state/mutants/$step
    # No resume: the recipes do not pass cargo-mutants' --iterate, so a
    # cut-off crate starts over rather than mixing a half-finished run.
    rm -rf "$dir"
    just --justfile "$repo/justfile" "$step" || rc=$?
    rm -rf "$dest"
    mkdir -p "$dest"
    if [ -d "$dir" ]; then
        cp -a "$dir/." "$dest/"
    fi
    local caught missed timeout unviable
    count_file() { if [ -f "$1" ]; then wc -l < "$1" | tr -d ' '; else echo 0; fi; }
    caught=$(count_file "$dest/caught.txt")
    missed=$(count_file "$dest/missed.txt")
    timeout=$(count_file "$dest/timeout.txt")
    unviable=$(count_file "$dest/unviable.txt")
    if [ "$rc" -eq 0 ]; then
        record_result "$step" "caught $caught, missed $missed, timeout $timeout, unviable $unviable"
        return 0
    fi
    record_result "$step" "failed: caught $caught, missed $missed, timeout $timeout, unviable $unviable (see the log)"
    return 1
}

run_one_step() {
    local step=$1
    ensure_dirs
    cd "$repo"
    if [ "${P12_DRY_RUN:-0}" = 1 ]; then
        sleep "$dry_sleep"
        dry_result "$step"
        return 0
    fi
    case "$step" in
        fuzz) run_fuzz ;;
        prove-all)
            if just --justfile "$repo/justfile" prove-all; then
                record_result prove-all "just prove-all passed"
            else
                record_result prove-all "failed: just prove-all failed (see the log)"
                return 1
            fi
            ;;
        oracle) run_oracle ;;
        props-max) run_props_max ;;
        mutants|mutants-store|mutants-import|mutants-config|mutants-mcp|mutants-host|mutants-package|mutants-vault|mutants-cli)
            run_mutants "$step"
            ;;
        *)
            record_result "$step" "failed: unknown step"
            return 1
            ;;
    esac
}

# ------------------------------------------------------------------ the runner

current_step=""
step_pid=""
watchdog_pid=""
step_start_iso=""
step_start_epoch=0
deadline_epoch=0

cleanup() {
    local now now_epoch
    now=$(iso)
    now_epoch=$(date +%s)
    if [ -n "$watchdog_pid" ]; then
        kill "$watchdog_pid" 2>/dev/null || true
    fi
    if [ -n "$current_step" ] && [ -n "$step_pid" ] && kill -0 "$step_pid" 2>/dev/null; then
        kill -TERM -- "-$step_pid" 2>/dev/null || true
        sleep 2
        kill -KILL -- "-$step_pid" 2>/dev/null || true
        write_status "$current_step" partial "$step_start_iso" "$now" \
            "$((now_epoch - step_start_epoch))" "stopped by p12-stop"
    fi
    echo "stopped" > "$state/run.state"
    write_summary
    rm -f "$pidfile" "$step_pidfile"
    echo "P12: stopped." >&2
    exit 143
}

run_step() {
    local step=$1
    local log=$logs/$step.log
    current_step=$step
    step_start_iso=$(iso)
    step_start_epoch=$(date +%s)
    : > "$log"
    rm -f "$steps_dir/$step.details"
    write_status "$step" running "$step_start_iso" "" "" "running"
    write_summary

    setsid "$self" --run-step "$step" >>"$log" 2>&1 </dev/null &
    step_pid=$!
    printf '%s\n' "$step_pid" > "$step_pidfile"

    (
        while kill -0 "$step_pid" 2>/dev/null; do
            if [ "$(date +%s)" -ge "$deadline_epoch" ]; then
                touch "$state/deadline.reached"
                kill -TERM -- "-$step_pid" 2>/dev/null || true
                sleep 3
                kill -KILL -- "-$step_pid" 2>/dev/null || true
                exit 0
            fi
            sleep 2
        done
    ) &
    watchdog_pid=$!

    local rc=0
    wait "$step_pid" || rc=$?
    kill "$watchdog_pid" 2>/dev/null || true
    wait "$watchdog_pid" 2>/dev/null || true
    watchdog_pid=""

    local end end_epoch secs result
    end=$(iso)
    end_epoch=$(date +%s)
    secs=$((end_epoch - step_start_epoch))
    result=$(cat "$steps_dir/$step.result" 2>/dev/null || echo "no result recorded")

    if [ "$rc" -eq 0 ]; then
        : > "$done_dir/$step"
        write_status "$step" "done" "$step_start_iso" "$end" "$secs" "$result"
    elif [ -f "$state/deadline.reached" ]; then
        write_status "$step" partial "$step_start_iso" "$end" "$secs" \
            "stopped at the deadline (${P12_STOP:-07:00}); reruns next night"
    else
        write_status "$step" failed "$step_start_iso" "$end" "$secs" "$result"
    fi
    current_step=""
    step_pid=""
    return 0
}

run_runner() {
    ensure_dirs
    cd "$repo"
    validate_selection
    # A runner killed with KILL leaves its step marked running. No runner is
    # live now, so record that step as partial (it has no done marker and
    # reruns below).
    local old_step old_start
    for old_step in "${queue[@]}"; do
        if [ ! -f "$done_dir/$old_step" ] &&
            [ "$(read_field "$steps_dir/$old_step.status" status)" = running ]; then
            old_start=$(read_field "$steps_dir/$old_step.status" start)
            write_status "$old_step" partial "${old_start:--}" "$(iso)" 0 \
                "a previous runner stopped without finishing; reruns now"
        fi
    done
    printf '%s\n' "$$" > "$pidfile"
    rm -f "$state/deadline.reached" "$step_pidfile"
    local started_iso
    started_iso=$(iso)
    printf '%s\n' "$started_iso" > "$state/started"
    if ! deadline_epoch=$(compute_deadline); then
        rm -f "$pidfile"
        exit 2
    fi
    date -d "@$deadline_epoch" '+%Y-%m-%d %H:%M:%S' > "$state/deadline.iso"
    echo "running" > "$state/run.state"
    write_summary
    trap cleanup TERM INT

    local step
    while IFS= read -r step; do
        [ -n "$step" ] || continue
        if [ -f "$done_dir/$step" ]; then
            continue
        fi
        if [ "$(date +%s)" -ge "$deadline_epoch" ]; then
            echo "deadline" > "$state/run.state"
            write_summary
            rm -f "$pidfile" "$step_pidfile"
            return 0
        fi
        run_step "$step"
        write_summary
        if [ -f "$state/deadline.reached" ]; then
            echo "deadline" > "$state/run.state"
            write_summary
            rm -f "$pidfile" "$step_pidfile"
            return 0
        fi
    done < <(selected_steps)

    echo "finished" > "$state/run.state"
    write_summary
    rm -f "$pidfile" "$step_pidfile"
}

start_runner() {
    ensure_dirs
    validate_selection
    if runner_alive; then
        echo "P12: a runner is already live (pid $(cat "$pidfile")); not starting another."
        echo "Summary: $state/summary.md"
        return 0
    fi
    rm -f "$pidfile" "$step_pidfile"
    setsid nohup "$self" run >>"$logs/runner.log" 2>&1 </dev/null &
    local i
    for i in $(seq 1 30); do
        [ -s "$pidfile" ] && break
        sleep 0.1
    done
    echo "P12: started the overnight run, detached."
    echo "Summary: $state/summary.md"
    echo "Logs:    $logs/"
    echo "Check:   just p12-status"
    echo "Stop:    just p12-stop"
}

show_status() {
    ensure_dirs
    if [ -f "$state/summary.md" ]; then
        cat "$state/summary.md"
    else
        echo "P12: no summary yet; the runner has not started a step."
    fi
    echo
    if runner_alive; then
        echo "Runner: live (pid $(cat "$pidfile"))."
    else
        echo "Runner: not running."
    fi
}

stop_runner() {
    ensure_dirs
    if ! runner_alive; then
        echo "P12: no runner is live."
        rm -f "$pidfile" "$step_pidfile"
        return 0
    fi
    local pid spid i
    pid=$(cat "$pidfile")
    echo "P12: stopping runner $pid ..."
    kill -TERM "$pid" 2>/dev/null || true
    for i in $(seq 1 50); do
        kill -0 "$pid" 2>/dev/null || break
        sleep 0.1
    done
    if kill -0 "$pid" 2>/dev/null; then
        kill -KILL "$pid" 2>/dev/null || true
    fi
    if [ -f "$step_pidfile" ]; then
        spid=$(cat "$step_pidfile" 2>/dev/null || true)
        if [ -n "$spid" ] && kill -0 "$spid" 2>/dev/null; then
            kill -TERM -- "-$spid" 2>/dev/null || true
            sleep 1
            kill -KILL -- "-$spid" 2>/dev/null || true
        fi
    fi
    rm -f "$pidfile" "$step_pidfile"
    echo "P12: stopped."
}

usage() {
    cat >&2 <<'EOF'
usage: scripts/p12-night.sh start|status|stop
       scripts/p12-night.sh --run-step STEP   (internal)
EOF
    exit 2
}

case "${1:-}" in
    start) start_runner ;;
    status) show_status ;;
    stop) stop_runner ;;
    run) run_runner ;;
    --run-step)
        [ -n "${2:-}" ] || usage
        run_one_step "$2"
        ;;
    *) usage ;;
esac
