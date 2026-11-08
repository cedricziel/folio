import CoreSpotlight
import FolioKit
import SwiftUI

/// SwiftUI declares its own `Document`; the app means the archive document.
typealias Document = FolioKit.Document

@main
struct FolioApp: App {
    @State private var model = AppModel()
    @State private var security = AppLockLifecycle()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            RootView(model: model, lock: security.lock)
                // Continuations go to an existing window, the frontmost one, instead of opening another.
                .handlesExternalEvents(preferring: ["*"], allowing: ["*"])
                .onChange(of: security.lock.isEnabled) { _, _ in security.refreshCovers() }
                .onChange(of: model.session, initial: true) { _, session in
                    DocumentActivityTracker.shared.sessionChanged(to: session)
                }
                .tint(.archive)
                .onReceive(
                    NotificationCenter.default.publisher(for: Notification.Name("FolioSessionChanged")).receive(
                        on: DispatchQueue.main)
                ) { _ in
                    model.restore()
                }
                .onChange(of: scenePhase) { _, phase in
                    switch phase {
                    case .active:
                        FolioTelemetry.shared.lifecycle("active")
                        model.restore()
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
                        AppLockSettingsView(lock: security.lock)
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

@MainActor @Observable
final class AppModel {
    private(set) var session: SessionRestoration = .restoring {
        didSet { SpotlightIndex.shared.sessionChanged(to: session) }
    }
    private(set) var isWorking = false
    var operationError: String?
    /// Handoff, universal-link and Spotlight continuations waiting for a window that can route them.
    let continuations = ContinuationInbox()
    private var restorationTask: Task<Void, Never>?
    private var restorationGeneration = 0
    private let store: any ServerConfigStore = UserDefaultsServerConfigStore.shared()

    init() { restore() }

    func restore() {
        guard !isWorking else { return }
        let server = session.serverURL ?? store.load()
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
            if case .signedIn = restored { FileProviderDomain.signalChanges() }
        }
    }

    static func makeService(for url: URL) -> any FolioService {
        SharedConfiguration.makeService(for: url)
    }

    func connected(to url: URL) {
        restorationTask?.cancel()
        restorationGeneration += 1
        SpotlightIndex.shared.signedIn(to: url)
        continuations.signedIn(to: url)
        session = .signedIn(url)
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
                try await SharedConfiguration.tokenProvider().signOut()
                FolioTelemetry.shared.stop()
                continuations.clear()
                session = .signedOut
                FileProviderDomain.unregister()
            } catch {
                operationError = error.localizedDescription
            }
        }
    }

}

struct RootView: View {
    let model: AppModel
    let lock: AppLock
    /// Identifies this window to the continuation inbox.
    @State private var sceneID = UUID()
    #if os(iOS)
        @State private var showLockSettings = false
    #endif

    private var continuations: ContinuationInbox { model.continuations }

    var body: some View {
        content
            .onContinueUserActivity(HandoffActivityType.document, perform: receiveHandoff)
            .onContinueUserActivity(HandoffActivityType.library, perform: receiveHandoff)
            .onContinueUserActivity(HandoffActivityType.savedView, perform: receiveHandoff)
            .onContinueUserActivity(NSUserActivityTypeBrowsingWeb) { activity in
                guard let url = activity.webpageURL else { return }
                continuations.receive(.universalLink(url), scene: sceneID)
            }
            .onContinueUserActivity(CSSearchableItemActionType) { activity in
                let identifier = activity.userInfo?[CSSearchableItemActivityIdentifier] as? String
                guard let link = identifier.flatMap(DocumentLink.init(spotlightIdentifier:)) else { return }
                continuations.receive(.spotlight(link), scene: sceneID)
            }
            .onDisappear { continuations.sceneClosed(sceneID) }
            .onChange(of: deliveryState, initial: true) { deliverContinuation() }
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
            WorkspaceView(
                serverURL: url, service: AppModel.makeService(for: url), onSignOut: model.signOut,
                pendingContinuation: continuations.pending?.id,
                takeContinuation: {
                    continuations.take(
                        scene: sceneID, isLocked: lock.isLocked, session: model.session, where: { $0.route != nil }
                    )?.route
                }
            )
            .id(url)
                #if os(iOS)
                    .toolbar {
                        ToolbarItem {
                            Menu {
                                Button("App lock settings…") { showLockSettings = true }
                                if lock.isEnabled {
                                    Button("Lock now", systemImage: "lock") { lock.lockNow() }
                                }
                            } label: {
                                Label("Privacy", systemImage: "lock.shield")
                            }
                        }
                    }
                    .sheet(isPresented: $showLockSettings) { AppLockSettingsView(lock: lock) }
                    .onDisappear { showLockSettings = false }
                #endif
        } else {
            switch model.session {
            case .restoring:
                ProgressView("Restoring sign-in…")
            case .signedOut:
                OnboardingView(model: model, prefill: signInPrefill)
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

    private struct DeliveryState: Equatable {
        let pending: UUID?
        let session: SessionRestoration
        let isLocked: Bool
    }

    private var deliveryState: DeliveryState {
        DeliveryState(pending: continuations.pending?.id, session: model.session, isLocked: lock.isLocked)
    }

    private var signInPrefill: URL? {
        if case let .signIn(server, _) = continuations.decision(isLocked: lock.isLocked, session: model.session) {
            server
        } else {
            nil
        }
    }

    private func receiveHandoff(_ activity: NSUserActivity) {
        continuations.receive(
            .activity(type: activity.activityType, userInfo: activity.userInfo, webpageURL: activity.webpageURL),
            scene: sceneID)
    }

    /// Handles everything except opening a target, which waits for the workspace to load its vaults.
    private func deliverContinuation() {
        guard
            let decision = continuations.take(
                scene: sceneID, isLocked: lock.isLocked, session: model.session, where: { $0.route == nil })
        else { return }
        present(HandoffPresentation.from(decision, currentServer: model.session.serverURL))
    }

    private func present(_ presentation: HandoffPresentation) {}

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

extension SessionRestoration {
    /// The server this session belongs to, if it names one.
    var serverURL: URL? {
        switch self {
        case let .signedIn(url), let .expired(url), let .interrupted(url): url
        case let .unavailable(server, _): server
        case .restoring, .signedOut: nil
        }
    }
}
