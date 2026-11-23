import re
import sys
from pathlib import Path


def should_retry(log):
    if "** TEST EXECUTE FAILED **" not in log:
        return False

    diagnostics = re.findall(r"^.*:\d+(?::\d+)?: error: (.+)$", log, re.MULTILINE)
    failed = re.findall(r"^Test Case '([^'\n]+)' failed \(", log, re.MULTILINE)
    runner_errors = re.findall(r"^\s*NavigationTests-Runner .* encountered an error (.+)$", log, re.MULTILINE)
    if runner_errors:
        return (
            len(runner_errors) == 1
            and "operation never finished bootstrapping" in runner_errors[0]
            and "XCTWaiter(StallHandling) handleStalledWait" in runner_errors[0]
            and not diagnostics
            and not re.search(r"^Test Case '.*' (started|passed|failed|skipped)", log, re.MULTILINE)
        )

    totals = {int(count) for count in re.findall(r"\b(\d+) failures? \(\d+ unexpected\)", log)}
    return (
        len(failed) == len(diagnostics) == 1
        and totals == {1}
        and re.fullmatch(
            re.escape(failed[0])
            + r" : Failed to terminate org\.folio\.tests\.inboxfixture:\d+: "
            + r"Failed to terminate org\.folio\.tests\.inboxfixture:\d+",
            diagnostics[0],
        ) is not None
    )


if __name__ == "__main__":
    log = Path(sys.argv[1]).read_text(errors="replace")
    raise SystemExit(0 if should_retry(log) else 1)
