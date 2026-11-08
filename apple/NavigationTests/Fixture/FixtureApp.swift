import FolioKit
import SwiftUI
import UIKit

typealias Document = FolioKit.Document

@main struct FixtureApp: App {
    private static let documents = [
        Document(title: "First fixture", originalFilename: "first.pdf"),
        Document(title: "Second fixture", originalFilename: "second.pdf"),
    ]
    let service = FixtureApp.makeService(arguments: ProcessInfo.processInfo.arguments)

    var body: some Scene {
        WindowGroup {
            if ProcessInfo.processInfo.arguments.contains("--scan-review") {
                ScanReviewFixture(service: service)
            } else if ProcessInfo.processInfo.arguments.contains("--onboarding-layout") {
                OnboardingLayoutFixture()
            } else if ProcessInfo.processInfo.arguments.contains("--share-layout") {
                ShareLayoutFixture()
            } else if ProcessInfo.processInfo.arguments.contains("--saved-views") {
                SavedViewsFixture(service: service)
            } else {
                WorkspaceView(
                    serverURL: URL(string: "https://unused.invalid")!,
                    service: service, onSignOut: {}
                )
                .overlay(alignment: .bottomTrailing) {
                    if ProcessInfo.processInfo.arguments.contains("--remote-edit") {
                        Button("Simulate remote edit") { simulateRemoteEdit() }
                            .buttonStyle(.borderedProminent)
                            .padding()
                    }
                }
            }
        }
    }
}

extension FixtureApp {
    private static func makeService(arguments: [String]) -> MockFolioService {
        guard arguments.contains("--classification") else {
            return MockFolioService(
                documents: documents, loseFirstEntryResponse: arguments.contains("--discussion-save-failure"))
        }
        // Processed but not reviewed, so the inbox holds it until it is marked reviewed.
        let letter = Document(
            title: "Unfiled letter", originalFilename: "letter.pdf", ocrStatus: .done, embedStatus: .done,
            canEditDocumentDate: true, reviewed: false, canReviewDocument: true)
        let kept = Document(
            title: "Second fixture", originalFilename: "second.pdf", ocrStatus: .done, embedStatus: .done,
            reviewed: false)
        return MockFolioService(
            documents: [letter, kept], correspondents: [Correspondent(name: "City utilities")],
            types: [DocumentType(name: "Invoice")], tags: [FolioKit.Tag(name: "Taxes")],
            failFirstClassificationSave: arguments.contains("--classification-save-failure"))
    }

    /// Changes the first document on the server, as finished processing would, without telling the app.
    private func simulateRemoteEdit() {
        var documents = Self.documents
        documents[0].title = "Electricity bill"
        documents[0].pageCount = 3
        documents[0].updatedAt = Date()
        Task { await service.setDocuments(documents) }
    }
}

private struct ScanReviewFixture: View {
    let service: any FolioService
    @State private var presented = false

    var body: some View {
        Button("Review fixture scan") { presented = true }
            .sheet(isPresented: $presented) {
                ScanReviewView(
                    model: ScanReviewViewModel(
                        pages: [
                            UIGraphicsImageRenderer(size: CGSize(width: 100, height: 140))
                                .image { context in
                                    UIColor.white.setFill()
                                    context.fill(CGRect(x: 0, y: 0, width: 100, height: 140))
                                }.cgImage!
                        ],
                        service: service
                    ),
                    onFinished: { _ in presented = false }
                )
            }
    }
}

private struct SavedViewsFixture: View {
    let service: any FolioService
    @State private var workspace: WorkspaceViewModel

    init(service: any FolioService) {
        self.service = service
        _workspace = State(initialValue: WorkspaceViewModel(service: service))
    }

    var body: some View {
        WorkspaceView(
            serverURL: URL(string: "https://unused.invalid")!, service: service, onSignOut: {}, workspace: workspace
        )
        .task {
            await workspace.loadVaults()
            workspace.destination = .all
        }
    }
}
