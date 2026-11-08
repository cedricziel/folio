import FolioKit
import SwiftUI
import UserNotifications

#if os(iOS)
    import UIKit
#else
    import AppKit
#endif

/// Asking for permission, registering with APNs and reacting to delivered notifications.
@MainActor
enum PushNotifications {
    private static let askedKey = "push.askedAfterUpload"
    private static let responder = NotificationResponder()

    /// Called once at launch: categories and the delegate must be set before a notification is opened.
    static func configure() {
        let center = UNUserNotificationCenter.current()
        center.delegate = responder
        let open = UNNotificationAction(identifier: FolioNotification.openAction, title: "Open", options: [.foreground])
        let review = UNNotificationAction(
            identifier: FolioNotification.markReviewedAction, title: "Mark reviewed", options: [.authenticationRequired]
        )
        // Shown on the lock screen when previews are hidden; it never names a document.
        let placeholder = "Document update"
        center.setNotificationCategories([
            UNNotificationCategory(
                identifier: FolioNotification.documentCategory, actions: [open, review], intentIdentifiers: [],
                hiddenPreviewsBodyPlaceholder: placeholder, options: []
            ),
            UNNotificationCategory(
                identifier: FolioNotification.inboxCategory, actions: [], intentIdentifiers: [],
                hiddenPreviewsBodyPlaceholder: placeholder, options: []
            ),
            UNNotificationCategory(
                identifier: FolioNotification.testCategory, actions: [], intentIdentifiers: [],
                hiddenPreviewsBodyPlaceholder: placeholder, options: []
            ),
        ])
    }

    /// Signed in: deliver quietly until the person decides, and tell the server about this device.
    static func signedIn(to server: URL) {
        Task {
            let center = UNUserNotificationCenter.current()
            if await center.notificationSettings().authorizationStatus == .notDetermined {
                _ = try? await center.requestAuthorization(options: [.provisional, .alert, .sound])
            }
            registerForRemoteNotifications()
            await PushRegistration.shared.signedIn(to: server)
        }
    }

    /// After the first finished upload or scan, ask once for prominent notifications.
    static func afterUpload() {
        let defaults = UserDefaults.standard
        guard !defaults.bool(forKey: askedKey) else { return }
        defaults.set(true, forKey: askedKey)
        Task { await requestFullAuthorization() }
    }

    static func requestFullAuthorization() async {
        let center = UNUserNotificationCenter.current()
        let status = await center.notificationSettings().authorizationStatus
        guard status == .notDetermined || status == .provisional else { return }
        if (try? await center.requestAuthorization(options: [.alert, .sound])) == true {
            registerForRemoteNotifications()
        }
    }

    /// Forget this device on the server, then clear what it already shows.
    static func signingOut() async {
        await PushRegistration.shared.signingOut()
        UNUserNotificationCenter.current().removeAllDeliveredNotifications()
    }

    static func registerForRemoteNotifications() {
        #if os(iOS)
            UIApplication.shared.registerForRemoteNotifications()
        #else
            NSApplication.shared.registerForRemoteNotifications()
        #endif
    }

    /// Marks the document of a notification reviewed, through the signed-in server.
    static func markReviewed(_ note: FolioNotification, requestID: String) async {
        guard let documentID = note.documentID, let service = SharedConfiguration.makeService() else { return }
        let scoped: any FolioService
        if let vaultID = note.vaultID {
            scoped = service.scoped(to: vaultID)
        } else {
            scoped = service
        }
        if (try? await scoped.setDocumentReview(true, for: documentID)) != nil {
            UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: [requestID])
        }
    }
}

/// Routes taps and actions; delegate callbacks arrive off the main actor.
final class NotificationResponder: NSObject, UNUserNotificationCenterDelegate, Sendable {
    func userNotificationCenter(
        _: UNUserNotificationCenter, willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        // The open app refreshes its lists; keep the notification in the list without interrupting.
        FolioNotification(userInfo: notification.request.content.userInfo) == nil ? [] : [.list]
    }

    func userNotificationCenter(_: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        guard let note = FolioNotification(userInfo: response.notification.request.content.userInfo) else { return }
        let action = response.actionIdentifier
        let requestID = response.notification.request.identifier
        await MainActor.run {
            if action == FolioNotification.markReviewedAction {
                Task { await PushNotifications.markReviewed(note, requestID: requestID) }
            } else if action != UNNotificationDismissActionIdentifier, !note.isTest {
                IntentRouter.shared.route(note.route)
            }
        }
    }
}

#if os(iOS)
    final class FolioAppDelegate: NSObject, UIApplicationDelegate {
        func application(
            _: UIApplication, didFinishLaunchingWithOptions _: [UIApplication.LaunchOptionsKey: Any]?
        ) -> Bool {
            PushNotifications.configure()
            return true
        }

        func application(_: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
            Task { await PushRegistration.shared.received(token: deviceToken) }
        }

        func application(_: UIApplication, didFailToRegisterForRemoteNotificationsWithError _: any Error) {
            // Without a token the server is never told about this device; nothing else to do.
        }
    }
#else
    final class FolioAppDelegate: NSObject, NSApplicationDelegate {
        func applicationDidFinishLaunching(_: Notification) {
            PushNotifications.configure()
        }

        func application(_: NSApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
            Task { await PushRegistration.shared.received(token: deviceToken) }
        }

        func application(_: NSApplication, didFailToRegisterForRemoteNotificationsWithError _: any Error) {
            // Without a token the server is never told about this device; nothing else to do.
        }
    }
#endif

/// Settings → Notifications
struct NotificationSettingsView: View {
    @State private var push = PushRegistration.shared
    @State private var testStatus: String?

    var body: some View {
        Form {
            Section {
                Toggle("Documents that need review", isOn: binding(\.needsReview))
                Toggle("Documents that failed processing", isOn: binding(\.failed))
                Toggle("Documents that are ready", isOn: binding(\.processed))
                Picker("Documents", selection: binding(\.scope)) {
                    Text("Uploaded by me").tag(NotificationPreferences.Scope.mine)
                    Text("All I can edit").tag(NotificationPreferences.Scope.editable)
                }
            } header: {
                Text("Notify me about")
            }
            Section {
                Toggle("Show document titles", isOn: binding(\.showTitles))
            } footer: {
                Text(
                    "Titles are fetched by this device and never sent through Apple. "
                        + "App lock does not cover notifications, so titles can appear on the lock screen."
                )
            }
            Section {
                if push.pushAvailable == false {
                    Text("This server can't send notifications. Ask its administrator to set up push notifications.")
                        .foregroundStyle(.secondary)
                }
                Button("Allow notifications") {
                    Task { await PushNotifications.requestFullAuthorization() }
                }
                Button("Send test notification") {
                    Task {
                        do {
                            try await push.sendTest()
                            testStatus = "A test notification is on its way."
                        } catch {
                            testStatus = error.localizedDescription
                        }
                    }
                }
                .disabled(push.pushAvailable != true)
                if let testStatus {
                    Text(testStatus).foregroundStyle(.secondary)
                }
            }
        }
        .formStyle(.grouped)
        .navigationTitle("Notifications")
    }

    private func binding<Value>(_ keyPath: WritableKeyPath<NotificationPreferences, Value>) -> Binding<Value> {
        Binding(
            get: { push.preferences[keyPath: keyPath] },
            set: { value in
                var preferences = push.preferences
                preferences[keyPath: keyPath] = value
                Task { await push.update(preferences) }
            }
        )
    }
}
