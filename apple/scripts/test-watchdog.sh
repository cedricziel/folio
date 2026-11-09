#!/usr/bin/env bash
# Runs a test command and, if it outlives its time limit, records the stacks of every test process before stopping
# it, so a hung run says where it is stuck instead of only timing out.
#
#   test-watchdog.sh <command> [args...]
#
# TEST_WATCHDOG_LIMIT     seconds before the run counts as hung (default 900)
# TEST_WATCHDOG_DIR       where the stacks go (default ./build/test-hang)
# TEST_WATCHDOG_PATTERN   pgrep -f pattern for the processes to sample
#                         (default: the Swift Testing helper and XCTest runners)
set -uo pipefail

limit=${TEST_WATCHDOG_LIMIT:-900}
out=${TEST_WATCHDOG_DIR:-$PWD/build/test-hang}
pattern=${TEST_WATCHDOG_PATTERN:-swiftpm-testing-helper|xctest|PackageTests}

"$@" &
pid=$!
deadline=$((SECONDS + limit))

while kill -0 "$pid" 2>/dev/null; do
    if ((SECONDS >= deadline)); then
        echo "::error::'$*' ran longer than ${limit}s; recording the test processes' stacks in $out"
        mkdir -p "$out"
        ps -axo pid,ppid,stat,etime,command >"$out/processes.txt" 2>&1
        for target in $(pgrep -f "$pattern" | grep -vx "$$"); do
            if command -v sample >/dev/null; then
                sample "$target" 5 -file "$out/sample-$target.txt" >/dev/null 2>&1 ||
                    sudo -n sample "$target" 5 -file "$out/sample-$target.txt" >/dev/null 2>&1 ||
                    echo "could not sample $target" >>"$out/errors.txt"
            fi
            if command -v spindump >/dev/null; then
                sudo -n spindump "$target" 5 -file "$out/spindump-$target.txt" >/dev/null 2>&1 ||
                    echo "could not spindump $target" >>"$out/errors.txt"
            fi
        done
        pkill -TERM -P "$pid" 2>/dev/null
        kill -TERM "$pid" 2>/dev/null
        sleep 5
        pkill -KILL -P "$pid" 2>/dev/null
        kill -KILL "$pid" 2>/dev/null
        exit 124
    fi
    sleep 5
done

wait "$pid"
