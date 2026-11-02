import FolioKit
import SwiftUI

/// SwiftUI declares its own `Document`; the app means the archive document.
typealias Document = FolioKit.Document

@main
struct FolioApp: App {
    @State private var model = AppModel()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            RootView(model: model)
                .tint(.archive)
                .onChange(of: scenePhase) { _, phase in
                    switch phase {
                    case .active: FolioTelemetry.shared.lifecycle("active")
                    case .inactive: FolioTelemetry.shared.lifecycle("inactive")
                    case .background: FolioTelemetry.shared.lifecycle("background")
                    @unknown default: break
                    }
                }
        }
        .commands { FolioCommands() }
    }
}

@MainActor @Observable
final class AppModel {
    private(set) var serverURL: URL?
    private let store: any ServerConfigStore = UserDefaultsServerConfigStore.shared()

    init() {
        // Without a sign-in the stored address only prefills onboarding.
        serverURL = KeychainCredentialStore().load()?.isSignedIn == true ? store.load() : nil
        if serverURL != nil { FileProviderDomain.register() }
    }

    static func makeService(for url: URL) -> any FolioService {
        SharedConfiguration.makeService(for: url)
    }

    func connected(to url: URL) {
        serverURL = url
        FileProviderDomain.register()
    }

    static func signIn(to url: URL) async throws {
        try await OAuthSignIn(
            store: KeychainCredentialStore(), clients: UserDefaultsClientRegistry(),
            authenticator: SystemWebAuthenticator()
        ).signIn(to: url)
    }

    /// Clears the tokens and returns to onboarding; the address stays to prefill the next sign-in.
    func signOut() {
        FolioTelemetry.shared.stop()
        serverURL = nil
        Task { await SharedConfiguration.tokenProvider().signOut() }
        FileProviderDomain.unregister()
    }
}

struct RootView: View {
    let model: AppModel

    var body: some View {
        if let url = model.serverURL {
            WorkspaceView(serverURL: url, service: AppModel.makeService(for: url), onSignOut: model.signOut)
                .id(url)
        } else {
            OnboardingView(model: model)
        }
    }
}
