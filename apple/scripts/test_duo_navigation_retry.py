import unittest

from duo_navigation_retry import should_retry


BOOTSTRAP_FAILURE = """NavigationTests-Runner (38368) encountered an error (Early unexpected exit, operation never finished bootstrapping - no restart will be attempted. (Underlying Error: The test runner crashed while preparing to run tests: NavigationTests-Runner at -[XCTWaiter(StallHandling) handleStalledWait:]))
** TEST EXECUTE FAILED **
"""
LAUNCH_FAILURE = """/fixture/NavigationTests.swift:70: error: -[NavigationTests.NavigationTests testInboxOpensAnotherDocumentAfterReturning] : Failed to terminate org.folio.tests.inboxfixture:39833: Failed to terminate org.folio.tests.inboxfixture:0
Test Case '-[NavigationTests.NavigationTests testInboxOpensAnotherDocumentAfterReturning]' failed (134.722 seconds).
Executed 13 tests, with 3 tests skipped and 1 failure (0 unexpected)
** TEST EXECUTE FAILED **
"""
ASSERTION_FAILURE = """/fixture/NavigationTests.swift:74: error: -[NavigationTests.NavigationTests testInboxOpensAnotherDocumentAfterReturning] : XCTAssertTrue failed
Test Case '-[NavigationTests.NavigationTests testInboxOpensAnotherDocumentAfterReturning]' failed (12.000 seconds).
** TEST EXECUTE FAILED **
"""


class DuoNavigationRetryTests(unittest.TestCase):
    def test_bootstrap_crash_before_tests_qualifies(self):
        self.assertTrue(should_retry(BOOTSTRAP_FAILURE))

    def test_bootstrap_diagnostic_after_a_test_started_does_not_qualify(self):
        started = "Test Case '-[NavigationTests.NavigationTests testInbox]' started.\n"
        self.assertFalse(should_retry(started + BOOTSTRAP_FAILURE))

    def test_fixture_termination_as_sole_failure_qualifies(self):
        self.assertTrue(should_retry(LAUNCH_FAILURE))

    def test_assertion_failure_does_not_qualify(self):
        self.assertFalse(should_retry(ASSERTION_FAILURE))

    def test_fixture_termination_with_an_assertion_failure_does_not_qualify(self):
        self.assertFalse(should_retry(LAUNCH_FAILURE + ASSERTION_FAILURE))

    def test_a_multi_failure_summary_does_not_qualify(self):
        self.assertFalse(should_retry(LAUNCH_FAILURE.replace("1 failure", "2 failures")))

    def test_incomplete_failure_log_does_not_qualify(self):
        self.assertFalse(should_retry(BOOTSTRAP_FAILURE.replace("** TEST EXECUTE FAILED **", "")))


if __name__ == "__main__":
    unittest.main()
