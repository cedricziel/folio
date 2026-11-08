import Foundation
import Observation

/// Keeps the server's record of this installation in step with the APNs token, the preferences and the
/// signed-in server, and removes it before sign-out.
@MainActor @Observable
public final class PushRegistration {
    public static let shared = PushRegistration(
        defaults: UserDefaults(suiteName: SharedConfiguration.appGroup) ?? .standard,
        installationID: {
            InstallationIdentity(store: KeychainInstallationIDStore()).resolve()?.uuidString.lowercased()
        },
        service: { url in SharedConfiguration.makeService(for: url) }
    )

    /// Whether the server can deliver notifications; nil until it answered.
    public private(set) var pushAvailable: Bool?
    public private(set) var lastError: String?
    public private(set) var preferences: NotificationPreferences

    private let defaults: UserDefaults
    private let installationID: () -> String?
    private let service: (URL) -> (any DeviceRegistrationService)?
    private var alertToken: String?
    private var server: URL?

    private static let preferencesKey = "push.preferences"

    public init(
        defaults: UserDefaults, installationID: @escaping () -> String?,
        service: @escaping (URL) -> (any DeviceRegistrationService)?
    ) {
        self.defaults = defaults
        self.installationID = installationID
        self.service = service
        preferences =
            defaults.data(forKey: Self.preferencesKey).flatMap {
                try? JSONDecoder().decode(NotificationPreferences.self, from: $0)
            } ?? NotificationPreferences()
    }

    /// The app is signed in to `server`; register if a token is known.
    public func signedIn(to server: URL) async {
        guard self.server != server else { return }
        self.server = server
        pushAvailable = nil
        await register()
    }

    /// APNs handed out a (possibly new) token.
    public func received(token: Data) async {
        let hex = PushToken.hex(token)
        guard hex != alertToken else { return }
        alertToken = hex
        await register()
    }

    public func update(_ preferences: NotificationPreferences) async {
        self.preferences = preferences
        if let data = try? JSONEncoder().encode(preferences) {
            defaults.set(data, forKey: Self.preferencesKey)
        }
        await register()
    }

    /// Call before revoking credentials: the server forgets the device while the token still works.
    /// Revocation removes it too, so a failure here is not an error.
    public func signingOut() async {
        defer {
            server = nil
            pushAvailable = nil
        }
        guard let server, let id = installationID(), let service = service(server) else { return }
        try? await service.deleteDevice(installationID: id)
    }

    public func sendTest() async throws {
        guard let server, let id = installationID(), let service = service(server) else {
            throw FolioError.signInRequired
        }
        try await service.sendTestNotification(installationID: id)
    }

    private func register() async {
        guard let server, let alertToken, let id = installationID(), let service = service(server) else { return }
        let registration = DeviceRegistration(
            locale: Locale.preferredLanguages.first ?? "en", alertToken: alertToken, preferences: preferences
        )
        do {
            let result = try await service.registerDevice(installationID: id, registration)
            // A sign-out or server switch during the request wins.
            guard self.server == server else { return }
            pushAvailable = result.pushAvailable
            lastError = nil
        } catch {
            lastError = error.localizedDescription
        }
    }
}
