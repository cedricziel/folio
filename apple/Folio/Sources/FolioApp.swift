import FolioKit
import SwiftUI

/// SwiftUI declares its own `Document`; the app means the archive document.
typealias Document = FolioKit.Document

@main
struct FolioApp: App {
    @State private var model = AppModel()

    var body: some Scene {
        WindowGroup {
            RootView(model: model)
                .tint(.archive)
        }
    }
}

@MainActor @Observable
final class AppModel {
    private(set) var serverURL: URL?
    private let store: any ServerConfigStore = UserDefaultsServerConfigStore.shared()

    init() {
        serverURL = store.load()
    }

    static func makeService(for url: URL) -> any FolioService {
        LiveFolioService(baseURL: url)
    }

    func connected(to url: URL) {
        serverURL = url
    }

    func changeServer() {
        store.save(nil)
        serverURL = nil
    }
}

struct RootView: View {
    let model: AppModel

    var body: some View {
        if let url = model.serverURL {
            InboxView(serverURL: url, service: AppModel.makeService(for: url), onChangeServer: model.changeServer)
                .id(url)
        } else {
            OnboardingView(model: model)
        }
    }
}
