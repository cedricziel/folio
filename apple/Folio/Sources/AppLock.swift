import Foundation
import Observation

@MainActor @Observable
final class AppLock {
    enum Timeout: Int, CaseIterable, Identifiable {
        case immediately = 0
        case oneMinute = 60
        case fiveMinutes = 300

        var id: Int { rawValue }
        var title: String {
            switch self {
            case .immediately: "Immediately"
            case .oneMinute: "After 1 minute"
            case .fiveMinutes: "After 5 minutes"
            }
        }
    }

    private(set) var isEnabled: Bool
    private(set) var isLocked: Bool
    private(set) var timeout: Timeout
    private(set) var isAuthenticating = false
    private(set) var isActive = true
    private(set) var problem: String?
    var needsPrivacyCover: Bool { isEnabled && !isActive }

    private let defaults: UserDefaults
    private let now: () -> TimeInterval
    private let authenticate: (String) async -> Bool
    private var awaySince: TimeInterval?
    private var generation = 0

    init(
        defaults: UserDefaults,
        now: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime },
        authenticate: @escaping (String) async -> Bool
    ) {
        self.defaults = defaults
        self.now = now
        self.authenticate = authenticate
        let enabled = defaults.bool(forKey: "appLock.enabled")
        isEnabled = enabled
        isLocked = enabled
        timeout = (defaults.object(forKey: "appLock.timeout") as? Int).flatMap(Timeout.init) ?? .oneMinute
    }

    func setEnabled(_ enabled: Bool) async {
        guard enabled != isEnabled, await authorize("Change Folio's App lock setting.") else { return }
        isEnabled = enabled
        isLocked = false
        awaySince = nil
        defaults.set(enabled, forKey: "appLock.enabled")
    }

    func setTimeout(_ timeout: Timeout) async {
        guard timeout != self.timeout,
            await authorize("Change when Folio locks.")
        else { return }
        self.timeout = timeout
        defaults.set(timeout.rawValue, forKey: "appLock.timeout")
    }

    func unlock() async {
        guard isLocked, await authorize("Unlock your Folio documents.") else { return }
        isLocked = false
        awaySince = nil
    }

    func lockNow() {
        generation += 1
        guard isEnabled else { return }
        isLocked = true
        problem = nil
    }

    func deviceLocked() {
        lockNow()
    }

    func resignActive() {
        isActive = false
        guard !isAuthenticating, awaySince == nil else { return }
        awaySince = now()
        if timeout == .immediately { lockNow() }
    }

    func enterBackground() {
        if isAuthenticating { lockNow() }
        isActive = false
        if awaySince == nil { awaySince = now() }
    }

    func becomeActive() {
        if let awaySince, now() - awaySince >= Double(timeout.rawValue) { lockNow() }
        awaySince = nil
        isActive = true
    }

    private func authorize(_ reason: String) async -> Bool {
        guard !isAuthenticating, isActive else { return false }
        isAuthenticating = true
        problem = nil
        let requestGeneration = generation
        let success = await authenticate(reason)
        isAuthenticating = false
        guard requestGeneration == generation else { return false }
        guard success else {
            problem =
                "Authentication was cancelled or unavailable. Try again using biometrics or your device credential."
            return false
        }
        return true
    }
}
