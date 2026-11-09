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

# Lists a process and everything below it: the test runner is a grandchild of the command (xcrun, swift test).
tree() {
    local child
    echo "$1"
    for child in $(pgrep -P "$1"); do
        tree "$child"
    done
}

# Stops the whole tree. The list is taken once, so a process that ignores TERM and loses its parent to it is still
# killed: it would no longer be found below the command.
stop_tree() {
    local processes
    processes=$(tree "$1")
    # shellcheck disable=SC2086
    kill -TERM $processes 2>/dev/null
    sleep "$2"
    # shellcheck disable=SC2086
    kill -KILL $processes 2>/dev/null
}

"$@" &
pid=$!
# A cancelled job stops the whole run too, so no test process outlives the step.
trap 'stop_tree "$pid" 2; exit 143' INT TERM
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
        stop_tree "$pid" 5
        exit 124
    fi
    sleep 5
done

wait "$pid"
