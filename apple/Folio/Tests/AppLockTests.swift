import Foundation
import XCTest

@MainActor
final class AppLockTests: XCTestCase {
    private var preferences: UserDefaults!
    private var suite: String!

    override func setUp() async throws {
        suite = "folio-lock-test-\(UUID().uuidString)"
        preferences = UserDefaults(suiteName: suite)!
    }

    override func tearDown() async throws {
        preferences.removePersistentDomain(forName: suite)
    }

    func testDisabledByDefaultAndEnabledLockSurvivesRelaunch() async {
        let lock = AppLock(defaults: preferences, authenticate: { _ in true })
        XCTAssertFalse(lock.isEnabled)
        XCTAssertFalse(lock.isLocked)
        XCTAssertEqual(lock.timeout, .oneMinute)
        await lock.setEnabled(true)
        XCTAssertFalse(lock.isLocked)
        let relaunched = AppLock(defaults: preferences, authenticate: { _ in true })
        XCTAssertTrue(relaunched.isEnabled)
        XCTAssertTrue(relaunched.isLocked)
    }

    func testFailedEnableDoesNotPersist() async {
        let lock = AppLock(defaults: preferences, authenticate: { _ in false })
        await lock.setEnabled(true)
        XCTAssertFalse(lock.isEnabled)
        XCTAssertNotNil(lock.problem)
    }

    func testFailedUnlockAndDisableLeaveLockEnabled() async {
        let lock = AppLock(defaults: preferences, authenticate: { _ in true })
        await lock.setEnabled(true)
        let relaunched = AppLock(defaults: preferences, authenticate: { _ in false })
        await relaunched.unlock()
        await relaunched.setEnabled(false)
        XCTAssertTrue(relaunched.isLocked)
        XCTAssertTrue(relaunched.isEnabled)
    }

    func testSuccessfulDisablePersists() async {
        let lock = AppLock(defaults: preferences, authenticate: { _ in true })
        await lock.setEnabled(true)
        await lock.setEnabled(false)
        XCTAssertFalse(AppLock(defaults: preferences, authenticate: { _ in false }).isEnabled)
    }

    func testPrivacyCoverDuringGraceAndUnlockBeforeDeadline() async {
        var time = 0.0
        let lock = AppLock(defaults: preferences, now: { time }, authenticate: { _ in true })
        await lock.setEnabled(true)
        lock.resignActive()
        XCTAssertTrue(lock.needsPrivacyCover)
        time = 59
        lock.becomeActive()
        XCTAssertFalse(lock.isLocked)
        XCTAssertFalse(lock.needsPrivacyCover)
    }

    func testReturnAtDeadlineLocks() async {
        var time = 0.0
        let lock = AppLock(defaults: preferences, now: { time }, authenticate: { _ in true })
        await lock.setEnabled(true)
        lock.resignActive()
        time = 60
        lock.becomeActive()
        XCTAssertTrue(lock.isLocked)
    }

    func testFiveMinuteTimeoutPersistsAndLocksAtBoundary() async {
        var time = 0.0
        let lock = AppLock(defaults: preferences, now: { time }, authenticate: { _ in true })
        await lock.setEnabled(true)
        await lock.setTimeout(.fiveMinutes)
        XCTAssertEqual(AppLock(defaults: preferences, authenticate: { _ in true }).timeout, .fiveMinutes)
        lock.resignActive()
        time = 299
        lock.becomeActive()
        XCTAssertFalse(lock.isLocked)
        lock.resignActive()
        time = 599
        lock.becomeActive()
        XCTAssertTrue(lock.isLocked)
    }

    func testImmediateTimeoutLocksOnResignation() async {
        let lock = AppLock(defaults: preferences, authenticate: { _ in true })
        await lock.setEnabled(true)
        await lock.setTimeout(.immediately)
        lock.resignActive()
        XCTAssertTrue(lock.isLocked)
    }

    func testManualAndDeviceLockIgnoreGrace() async {
        let lock = AppLock(defaults: preferences, authenticate: { _ in true })
        await lock.setEnabled(true)
        lock.lockNow()
        XCTAssertTrue(lock.isLocked)
        await lock.unlock()
        lock.deviceLocked()
        XCTAssertTrue(lock.isLocked)
    }

    func testDeviceLockWithoutFocusChangeCanBeUnlocked() async {
        let lock = AppLock(defaults: preferences, authenticate: { _ in true })
        await lock.setEnabled(true)
        lock.deviceLocked()
        await lock.unlock()
        XCTAssertFalse(lock.isLocked)
    }

    func testAuthenticationInactivityDoesNotRelock() async {
        var lock: AppLock!
        lock = AppLock(
            defaults: preferences,
            authenticate: { _ in
                lock.resignActive()
                lock.becomeActive()
                return true
            })
        await lock.setEnabled(true)
        await lock.setTimeout(.immediately)
        lock.lockNow()
        await lock.unlock()
        XCTAssertFalse(lock.isLocked)
    }

    func testDeviceLockRejectsSuccessfulPendingAuthentication() async {
        let initial = AppLock(defaults: preferences, authenticate: { _ in true })
        await initial.setEnabled(true)
        var lock: AppLock!
        lock = AppLock(
            defaults: preferences,
            authenticate: { _ in
                lock.deviceLocked()
                return true
            })
        await lock.unlock()
        XCTAssertTrue(lock.isLocked)
    }

    func testAuthenticationCanFinishBeforeApplicationReactivates() async {
        let initial = AppLock(defaults: preferences, authenticate: { _ in true })
        await initial.setEnabled(true)
        await initial.setTimeout(.immediately)
        var lock: AppLock!
        lock = AppLock(
            defaults: preferences,
            authenticate: { _ in
                lock.resignActive()
                return true
            })
        await lock.unlock()
        XCTAssertTrue(lock.needsPrivacyCover)
        lock.becomeActive()
        XCTAssertFalse(lock.isLocked)
        XCTAssertFalse(lock.needsPrivacyCover)
    }

    func testRepeatedInactiveNotificationsDoNotExtendGrace() async {
        var time = 0.0
        let lock = AppLock(defaults: preferences, now: { time }, authenticate: { _ in true })
        await lock.setEnabled(true)
        lock.resignActive()
        time = 30
        lock.resignActive()
        time = 60
        lock.becomeActive()
        XCTAssertTrue(lock.isLocked)
    }

    func testDisabledLockDoesNotCoverOrBlockDocuments() {
        let lock = AppLock(defaults: preferences, authenticate: { _ in false })
        lock.resignActive()
        lock.deviceLocked()
        lock.enterBackground()
        lock.becomeActive()
        XCTAssertFalse(lock.isLocked)
        XCTAssertFalse(lock.needsPrivacyCover)
    }

    func testFailedTimeoutChangeKeepsPreviousValue() async {
        let lock = AppLock(defaults: preferences, authenticate: { _ in false })
        await lock.setTimeout(.fiveMinutes)
        XCTAssertEqual(lock.timeout, .oneMinute)
        XCTAssertEqual(AppLock(defaults: preferences, authenticate: { _ in true }).timeout, .oneMinute)
    }

    func testBackgroundRejectsPendingDisable() async {
        let initial = AppLock(defaults: preferences, authenticate: { _ in true })
        await initial.setEnabled(true)
        var lock: AppLock!
        lock = AppLock(
            defaults: preferences,
            authenticate: { _ in
                lock.enterBackground()
                return true
            })
        await lock.setEnabled(false)
        XCTAssertTrue(lock.isEnabled)
        XCTAssertTrue(lock.isLocked)
    }
}
