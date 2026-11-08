import FolioKit
import SwiftUI

/// SwiftUI declares its own `Document`; the app means the archive document.
typealias Document = FolioKit.Document

@main
struct FolioApp: App {
    #if os(iOS)
        @UIApplicationDelegateAdaptor(FolioAppDelegate.self) private var appDelegate
    #else
        @NSApplicationDelegateAdaptor(FolioAppDelegate.self) private var appDelegate
    #endif
    @State private var model = AppModel()
    @State private var security = AppLockLifecycle()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            RootView(model: model, lock: security.lock)
                .onChange(of: security.lock.isEnabled) { _, _ in security.refreshCovers() }
                .tint(.archive)
                .onReceive(
                    NotificationCenter.default.publisher(for: Notification.Name("FolioSessionChanged")).receive(
                        on: DispatchQueue.main)
                ) { _ in
                    model.restore()
                }
                .onReceive(NotificationCenter.default.publisher(for: .folioUploadFinished)) { _ in
                    PushNotifications.afterUpload()
                }
                .onChange(of: scenePhase) { _, phase in
                    switch phase {
                    case .active:
                        FolioTelemetry.shared.lifecycle("active")
                        model.restore()
                        Task { await ShareSessionCoordinator.shared.recoverInterrupted() }
                    case .inactive: FolioTelemetry.shared.lifecycle("inactive")
                    case .background: FolioTelemetry.shared.lifecycle("background")
                    @unknown default: break
                    }
                }
        }
        .commands {
            FolioCommands(lock: security.lock)
            AppLockCommands(lock: security.lock)
        }
        #if os(macOS)
            Settings {
                Group {
                    if security.lock.isLocked {
                        AppLockView(lock: security.lock)
                    } else {
                        TabView {
                            AppLockSettingsView(lock: security.lock)
                                .tabItem { Label("Privacy", systemImage: "lock.shield") }
                            NotificationSettingsView()
                                .tabItem { Label("Notifications", systemImage: "bell") }
                        }
                    }
                }
                .navigationTitle("Folio Settings")
                .frame(minWidth: 400, idealWidth: 480, minHeight: 360, idealHeight: 420)
                .onChange(of: security.lock.isEnabled) { _, _ in security.refreshCovers() }
            }
            .windowResizability(.contentSize)
        #endif
    }
}

#if os(iOS)
    import UIKit

    /// Hands share-extension upload sessions the system relaunches the app for to the share coordinator, so
    /// their outcome is recorded after the extension has gone.
    @MainActor final class FolioAppDelegate: NSObject, UIApplicationDelegate {
        func application(
            _: UIApplication, handleEventsForBackgroundURLSession identifier: String,
            completionHandler: @escaping () -> Void
        ) {
            if !ShareSessionCoordinator.shared.handleEvents(for: identifier, completion: completionHandler) {
                completionHandler()
            }
        }
    }
#endif

@MainActor @Observable
final class AppModel {
    private(set) var session: SessionRestoration = .restoring {
        didSet {
            SpotlightIndex.shared.sessionChanged(to: session)
            if case let .signedIn(url) = session { PushNotifications.signedIn(to: url) }
        }
    }
    private(set) var isWorking = false
    var operationError: String?
    private var restorationTask: Task<Void, Never>?
    private var restorationGeneration = 0
    private let store: any ServerConfigStore = UserDefaultsServerConfigStore.shared()

    init() { restore() }

    func restore() {
        guard !isWorking else { return }
        let rememberedServer: URL?
        switch session {
        case let .signedIn(url), let .expired(url), let .interrupted(url): rememberedServer = url
        case let .unavailable(server, _): rememberedServer = server
        case .signedOut, .restoring: rememberedServer = nil
        }
        let server = rememberedServer ?? store.load()
        restorationTask?.cancel()
        restorationGeneration += 1
        let generation = restorationGeneration
        restorationTask = Task {
            let restored: SessionRestoration
            do {
                try await SharedConfiguration.tokenProvider().retryPendingSave()
                restored = try await SharedConfiguration.oauthRefreshLock.withLock {
                    SessionRestoration.restore(from: KeychainCredentialStore(), server: server)
                }
            } catch is CancellationError {
                return
            } catch {
                restored = .unavailable(server: server, message: error.localizedDescription)
            }
            guard !Task.isCancelled, generation == restorationGeneration, !isWorking else { return }
            if restored != session {
                session = restored
                if case .signedIn = restored { FileProviderDomain.register() }
            }
            if case .signedIn = restored {
                FolioTelemetry.shared.signedIn()
                FileProviderDomain.signalChanges()
            }
        }
    }

    static func makeService(for url: URL) -> any FolioService {
        SharedConfiguration.makeService(for: url)
    }

    func connected(to url: URL) {
        restorationTask?.cancel()
        restorationGeneration += 1
        SpotlightIndex.shared.signedIn(to: url)
        session = .signedIn(url)
        FolioTelemetry.shared.signedIn()
        FileProviderDomain.register()
    }

    static func signIn(to url: URL) async throws {
        try await OAuthSignIn(
            store: KeychainCredentialStore(), clients: UserDefaultsClientRegistry(),
            authenticator: SystemWebAuthenticator(), refreshLock: SharedConfiguration.oauthRefreshLock
        ).signIn(to: url)
    }

    func reconnect(to url: URL) {
        guard !isWorking else { return }
        isWorking = true
        restorationTask?.cancel()
        restorationGeneration += 1
        Task {
            defer { isWorking = false }
            do {
                try await Self.signIn(to: url)
                connected(to: url)
            } catch {
                operationError = error.localizedDescription
            }
        }
    }

    func signOut() {
        guard !isWorking else { return }
        isWorking = true
        restorationTask?.cancel()
        restorationGeneration += 1
        Task {
            defer { isWorking = false }
            do {
                // The server forgets this device while the credentials still work; revocation removes it too.
                await PushNotifications.signingOut()
                // Sent while the token still works; best effort, sign-out never waits more than briefly.
                FolioTelemetry.shared.action(.signOut)
                await FolioTelemetry.shared.endSession(.signOut)
                try await SharedConfiguration.tokenProvider().signOut()
                FolioTelemetry.shared.stop()
                session = .signedOut
                FileProviderDomain.unregister()
            } catch {
                // Still signed in, so keep recording against a session.
                FolioTelemetry.shared.signedIn()
                operationError = error.localizedDescription
            }
        }
    }

}

struct RootView: View {
    let model: AppModel
    let lock: AppLock
    #if os(iOS)
        @State private var showLockSettings = false
        @State private var showNotificationSettings = false
    #endif

    var body: some View {
        content
            .alert(
                "Couldn't complete sign-in change",
                isPresented: Binding(
                    get: { model.operationError != nil },
                    set: { if !$0 { model.operationError = nil } }
                )
            ) {
                Button("OK") { model.operationError = nil }
            } message: {
                Text(model.operationError ?? "")
            }
    }

    @ViewBuilder private var content: some View {
        if lock.isLocked {
            AppLockView(lock: lock)
        } else if case let .signedIn(url) = model.session {
            WorkspaceView(serverURL: url, service: AppModel.makeService(for: url), onSignOut: model.signOut)
                .id(url)
                #if os(iOS)
                    .toolbar {
                        ToolbarItem {
                            Menu {
                                Button("App lock settings…") { showLockSettings = true }
                                Button("Notification settings…") { showNotificationSettings = true }
                                if lock.isEnabled {
                                    Button("Lock now", systemImage: "lock") { lock.lockNow() }
                                }
                            } label: {
                                Label("Privacy", systemImage: "lock.shield")
                            }
                        }
                    }
                    .sheet(isPresented: $showLockSettings) { AppLockSettingsView(lock: lock) }
                    .sheet(isPresented: $showNotificationSettings) {
                        NavigationStack { NotificationSettingsView() }
                    }
                    .onDisappear {
                        showLockSettings = false
                        showNotificationSettings = false
                    }
                #endif
        } else {
            switch model.session {
            case .restoring:
                ProgressView("Restoring sign-in…")
            case .signedOut:
                OnboardingView(model: model)
            case let .expired(url):
                recovery(
                    title: "Sign in again", message: "Your session has expired or was revoked. Sign in again.",
                    server: url, retry: false)
            case let .interrupted(url):
                recovery(
                    title: "Sign in again", message: "Your sign-in could not be renewed safely. Sign in again.",
                    server: url, retry: true)
            case let .unavailable(server, message):
                recovery(title: "Couldn't access your saved sign-in", message: message, server: server, retry: true)
            case .signedIn:
                EmptyView()
            }
        }
    }

    private func recovery(title: String, message: String, server: URL?, retry: Bool) -> some View {
        ContentUnavailableView {
            Label(title, systemImage: "lock.circle")
        } description: {
            Text(message)
            if let server { Text(server.absoluteString) }
        } actions: {
            if retry { Button("Try again") { model.restore() } }
            if let server { Button("Sign in again") { model.reconnect(to: server) } }
            if model.isWorking { ProgressView() }
        }
        .disabled(model.isWorking)
    }
}
