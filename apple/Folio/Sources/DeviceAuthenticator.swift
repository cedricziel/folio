import LocalAuthentication

@MainActor
final class DeviceAuthenticator {
    private var context: LAContext?

    func authenticate(reason: String) async -> Bool {
        let context = LAContext()
        self.context = context
        defer { self.context = nil }
        guard context.canEvaluatePolicy(.deviceOwnerAuthentication, error: nil) else { return false }
        do {
            return try await context.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: reason)
        } catch {
            return false
        }
    }

    func cancel() {
        context?.invalidate()
    }
}
