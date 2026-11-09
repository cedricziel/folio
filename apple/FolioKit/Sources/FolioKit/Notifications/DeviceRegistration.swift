import Foundation

/// What a device wants to hear about; mirrors the server's notification preferences.
public struct NotificationPreferences: Codable, Equatable, Sendable {
    public enum Scope: String, Codable, Sendable {
        /// Documents this person uploaded
        case mine
        /// Every document this person may edit
        case editable
    }

    public enum Vaults: Equatable, Sendable {
        case all
        case only([UUID])
    }

    public var processed: Bool
    public var needsReview: Bool
    public var failed: Bool
    public var scope: Scope
    public var vaults: Vaults
    /// Let the device fetch and show document titles; App lock does not cover notifications.
    public var showTitles: Bool

    public init(
        processed: Bool = false, needsReview: Bool = true, failed: Bool = true, scope: Scope = .mine,
        vaults: Vaults = .all, showTitles: Bool = false
    ) {
        self.processed = processed
        self.needsReview = needsReview
        self.failed = failed
        self.scope = scope
        self.vaults = vaults
        self.showTitles = showTitles
    }

    private enum CodingKeys: String, CodingKey {
        case processed, failed, scope, vaults
        case needsReview = "needs_review"
        case showTitles = "show_titles"
    }

    public init(from decoder: any Decoder) throws {
        let defaults = NotificationPreferences()
        let container = try decoder.container(keyedBy: CodingKeys.self)
        processed = try container.decodeIfPresent(Bool.self, forKey: .processed) ?? defaults.processed
        needsReview = try container.decodeIfPresent(Bool.self, forKey: .needsReview) ?? defaults.needsReview
        failed = try container.decodeIfPresent(Bool.self, forKey: .failed) ?? defaults.failed
        scope = try container.decodeIfPresent(Scope.self, forKey: .scope) ?? defaults.scope
        showTitles = try container.decodeIfPresent(Bool.self, forKey: .showTitles) ?? defaults.showTitles
        if let ids = try? container.decode([UUID].self, forKey: .vaults) {
            vaults = .only(ids)
        } else {
            vaults = .all
        }
    }

    public func encode(to encoder: any Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(processed, forKey: .processed)
        try container.encode(needsReview, forKey: .needsReview)
        try container.encode(failed, forKey: .failed)
        try container.encode(scope, forKey: .scope)
        try container.encode(showTitles, forKey: .showTitles)
        switch vaults {
        case .all: try container.encode("all", forKey: .vaults)
        case let .only(ids): try container.encode(ids.map { $0.uuidString.lowercased() }, forKey: .vaults)
        }
    }
}

/// The APNs environment a token belongs to, decided by how the app was built.
public enum PushEnvironment: String, Codable, Sendable {
    case sandbox
    case production

    public static var current: PushEnvironment {
        #if DEBUG
            .sandbox
        #else
            .production
        #endif
    }
}

public enum DevicePlatform: String, Codable, Sendable {
    case ios
    case macos

    public static var current: DevicePlatform {
        #if os(macOS)
            .macos
        #else
            .ios
        #endif
    }
}

/// What the app tells the server about this installation.
public struct DeviceRegistration: Encodable, Equatable, Sendable {
    public var platform: DevicePlatform
    public var environment: PushEnvironment
    public var locale: String
    /// Absent keeps the token the server has.
    public var alertToken: String?
    public var widgetToken: String?
    public var preferences: NotificationPreferences

    public init(
        platform: DevicePlatform = .current, environment: PushEnvironment = .current, locale: String,
        alertToken: String?, widgetToken: String? = nil, preferences: NotificationPreferences
    ) {
        self.platform = platform
        self.environment = environment
        self.locale = locale
        self.alertToken = alertToken
        self.widgetToken = widgetToken
        self.preferences = preferences
    }

    private enum CodingKeys: String, CodingKey {
        case platform, environment, locale, preferences
        case alertToken = "alert_token"
        case widgetToken = "widget_token"
    }

    public func encode(to encoder: any Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(platform, forKey: .platform)
        try container.encode(environment, forKey: .environment)
        try container.encode(locale, forKey: .locale)
        try container.encodeIfPresent(alertToken, forKey: .alertToken)
        try container.encodeIfPresent(widgetToken, forKey: .widgetToken)
        try container.encode(preferences, forKey: .preferences)
    }
}

/// The server's answer to a registration.
public struct DeviceRegistrationResult: Equatable, Sendable {
    public var preferences: NotificationPreferences
    /// Whether the server can deliver notifications at all
    public var pushAvailable: Bool

    public init(preferences: NotificationPreferences, pushAvailable: Bool) {
        self.preferences = preferences
        self.pushAvailable = pushAvailable
    }
}

/// Server endpoints that register this installation for push notifications.
public protocol DeviceRegistrationService: Sendable {
    func registerDevice(installationID: String, _ registration: DeviceRegistration) async throws
        -> DeviceRegistrationResult
    /// Succeeds when the device is not registered (any more).
    func deleteDevice(installationID: String) async throws
    func sendTestNotification(installationID: String) async throws
}

public enum PushToken {
    /// APNs tokens travel as lowercase hexadecimal.
    public static func hex(_ token: Data) -> String {
        token.map { String(format: "%02x", $0) }.joined()
    }
}

/// Server wire format of a registration response.
struct DeviceRegistrationWire: Decodable {
    struct Device: Decodable {
        let preferences: NotificationPreferences
    }

    let device: Device
    let pushAvailable: Bool

    private enum CodingKeys: String, CodingKey {
        case device
        case pushAvailable = "push_available"
    }
}
