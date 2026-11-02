import FolioKit
import SwiftUI

typealias Document = FolioKit.Document

@main struct FixtureApp: App {
    let service = MockFolioService(documents: [
        Document(title: "First fixture", originalFilename: "first.pdf"),
        Document(title: "Second fixture", originalFilename: "second.pdf"),
    ])

    var body: some Scene {
        WindowGroup {
            WorkspaceView(
                serverURL: URL(string: "https://unused.invalid")!,
                service: service, onSignOut: {}
            )
        }
    }
}
