import CoreSpotlight
import FolioKit
import SwiftUI

#if os(macOS)
    import AppKit
#endif

/// SwiftUI declares its own `Document`; the app means the archive document.
typealias Document = FolioKit.Document

@main
struct FolioApp: App {
    #if os(iOS)
        @UIApplicationDelegateAdaptor(FolioAppDelegate.self) private var appDelegate
    #endif
    @State private var model = AppModel()
    @State private var security = AppLockLifecycle()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            RootView(model: model, lock: security.lock)
                // Continuations go to an existing window, the frontmost one, instead of opening another.
                .handlesExternalEvents(preferring: ["*"], allowing: ["*"])
                .onChange(of: security.lock.isEnabled) { _, _ in security.refreshCovers() }
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
            TransferFileCache.shared.sessionChanged(to: session)
        }
    }
    /// Counts sign-ins through the browser, which may be another account even on the same server.
    private(set) var interactiveSignIns = 0
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
        TransferFileCache.shared.signedIn(to: url)
        continuations.signedIn(to: url)
        interactiveSignIns += 1
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
    @Environment(\.scenePhase) private var scenePhase
    /// This window's open document, as the system sees it and as Handoff offers it.
    @State private var activity = DocumentActivityTracker(isEligibleForHandoff: true)
    @State private var handoffAlert: HandoffAlert?
    #if os(iOS)
        @State private var browserPage: BrowserPage?
        @State private var showLockSettings = false
    #endif

    private var continuations: ContinuationInbox { model.continuations }

    var body: some View {
        content
            .environment(activity)
            .onChange(of: model.session, initial: true) { _, session in activity.sessionChanged(to: session) }
            .onChange(of: model.interactiveSignIns) {
                if case let .signedIn(url) = model.session { activity.signedIn(to: url) }
            }
            .environment(\.handoffContext, HandoffContext(session: model.session, isLocked: lock.isLocked))
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
            // A destroyed iPad scene may never call onDisappear; it always reaches the background first,
            // and a backgrounded window is no longer the one to show the continuation in.
            .onChange(of: scenePhase) { _, phase in
                if phase == .background { continuations.sceneClosed(sceneID) }
            }
            .onChange(of: deliveryState, initial: true) { deliverContinuation() }
            .alert(
                handoffAlert?.title ?? "",
                isPresented: Binding(get: { handoffAlert != nil }, set: { if !$0 { handoffAlert = nil } }),
                presenting: handoffAlert
            ) { alert in
                ForEach(alert.actions, id: \.title) { action in
                    switch action {
                    case let .openInBrowser(url):
                        Button(action.title) { openInBrowser(url) }
                    case .cancel:
                        Button(action.title, role: .cancel) {}
                    }
                }
            } message: { alert in
                Text(alert.message)
            }
            #if os(iOS)
                .sheet(item: $browserPage) { page in
                    SafariView(url: page.url).ignoresSafeArea()
                }
            #endif
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
                pendingContinuation: continuations.pending,
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
        /// The whole item, so closing the window that received it lets this window try again.
        let pending: ContinuationInbox.Item?
        let session: SessionRestoration
        let isLocked: Bool
    }

    private var deliveryState: DeliveryState {
        DeliveryState(pending: continuations.pending, session: model.session, isLocked: lock.isLocked)
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

    private func present(_ presentation: HandoffPresentation) {
        switch presentation {
        case let .alert(title, message, _):
            handoffAlert = HandoffAlert(title: title, message: message, actions: presentation.actions)
        case let .browser(url):
            openInBrowser(url)
        case .navigate, .prefillSignIn, .nothing:
            break
        }
    }

    /// The web page in a browser that never hands the link back to Folio.
    private func openInBrowser(_ url: URL) {
        guard let scheme = url.scheme?.lowercased(), scheme == "https" || scheme == "http" else { return }
        #if os(iOS)
            browserPage = BrowserPage(url: url)
        #else
            // Open the default browser by name: a self-build with associated domains would otherwise
            // receive its own link back.
            let workspace = NSWorkspace.shared
            guard let browser = workspace.urlForApplication(toOpen: URL(string: "https://example.com")!) else {
                return
            }
            workspace.open([url], withApplicationAt: browser, configuration: NSWorkspace.OpenConfiguration())
        #endif
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
