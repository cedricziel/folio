import FolioKit
import SwiftUI
import UIKit

typealias Document = FolioKit.Document

@main struct FixtureApp: App {
    private static let documents = [
        Document(title: "First fixture", originalFilename: "first.pdf"),
        Document(title: "Second fixture", originalFilename: "second.pdf"),
    ]
    static let firstDocumentID = documents[0].id
    let service = FixtureApp.makeService(arguments: ProcessInfo.processInfo.arguments)

    var body: some Scene {
        WindowGroup {
            if ProcessInfo.processInfo.arguments.contains("--scan-review") {
                ScanReviewFixture(service: service)
            } else if ProcessInfo.processInfo.arguments.contains("--onboarding-layout") {
                OnboardingLayoutFixture()
            } else if ProcessInfo.processInfo.arguments.contains("--share-layout") {
                ShareLayoutFixture()
            } else if ProcessInfo.processInfo.arguments.contains("--handoff") {
                HandoffFixture(service: service, documentID: Self.firstDocumentID)
            } else if ProcessInfo.processInfo.arguments.contains("--processing-state") {
                ProcessingStateFixture()
            } else if ProcessInfo.processInfo.arguments.contains("--custom-metadata") {
                CustomMetadataFixture(
                    documents: service,
                    canEdit: !ProcessInfo.processInfo.arguments.contains("--read-only")
                )
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
        let correspondent = Correspondent(name: "City utilities")
        let type = DocumentType(name: "Invoice")
        let tag = FolioKit.Tag(name: "Taxes")
        let classified = arguments.contains("--classification-saved")
        // Processed but not reviewed, so the inbox holds it until it is marked reviewed.
        let letter = Document(
            title: "Unfiled letter", originalFilename: "letter.pdf",
            correspondentID: classified ? correspondent.id : nil, documentTypeID: classified ? type.id : nil,
            ocrStatus: .done, embedStatus: .done, tags: classified ? [tag] : [],
            canEditDocumentDate: true, reviewed: false, canReviewDocument: true)
        let kept = Document(
            title: "Second fixture", originalFilename: "second.pdf", ocrStatus: .done, embedStatus: .done,
            reviewed: false)
        return MockFolioService(
            documents: [letter, kept], correspondents: [correspondent], types: [type], tags: [tag],
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

/// Delivers a Handoff continuation to the first document on demand, as another device would.
private struct HandoffFixture: View {
    let service: any FolioService
    let documentID: Document.ID
    private static let server = URL(string: "https://unused.invalid")!
    @State private var inbox = ContinuationInbox()
    private let scene = UUID()

    var body: some View {
        WorkspaceView(
            serverURL: Self.server, service: service, onSignOut: {}, pendingContinuation: inbox.pending,
            takeContinuation: {
                inbox.take(scene: scene, isLocked: false, session: .signedIn(Self.server))?.route
            }
        )
        .overlay(alignment: .bottomTrailing) {
            Button("Simulate handoff") {
                let payload = HandoffPayload(
                    server: Self.server, vaultID: MockFolioService.defaultVault.id, target: .document(documentID))
                inbox.receive(.payload(payload), scene: scene)
            }
            .buttonStyle(.borderedProminent)
            .padding()
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
            if ProcessInfo.processInfo.arguments.contains("--slow-saved-view"), let mock = service as? MockFolioService
            {
                await mock.delaySavedViewResponses(by: .seconds(12))
            }
            await workspace.loadVaults()
            workspace.destination = .all
        }
    }
}

private struct ProcessingStateFixture: View {
    var body: some View {
        ScrollView {
            DocumentProcessingView(
                document: Document(
                    title: "Processing fixture", ocrStatus: .failed, embedStatus: .failed,
                    processingStages: [
                        .init(
                            runID: UUID(), stage: .ocr, attempt: 2, state: .waitingToRetry,
                            reason: .providerFailed, retryAt: Date(timeIntervalSince1970: 1_800_000_000)
                        ),
                        .init(runID: UUID(), stage: .embedding, attempt: 4, state: .failed, reason: .invalidResponse),
                        .init(
                            runID: UUID(), stage: .thumbnail, attempt: 1, state: .skipped, reason: .unsupportedFormat),
                    ]
                )
            )
            .padding(FolioTheme.Spacing.space4)
        }
    }
}
