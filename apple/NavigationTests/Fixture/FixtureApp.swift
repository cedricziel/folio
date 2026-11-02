import FolioKit
import SwiftUI
import UIKit

typealias Document = FolioKit.Document

@main struct FixtureApp: App {
    let service = MockFolioService(documents: [
        Document(title: "First fixture", originalFilename: "first.pdf"),
        Document(title: "Second fixture", originalFilename: "second.pdf"),
    ])

    var body: some Scene {
        WindowGroup {
            if ProcessInfo.processInfo.arguments.contains("--scan-review") {
                ScanReviewFixture(service: service)
            } else if ProcessInfo.processInfo.arguments.contains("--onboarding-layout") {
                OnboardingLayoutFixture()
            } else if ProcessInfo.processInfo.arguments.contains("--share-layout") {
                ShareLayoutFixture()
            } else {
                WorkspaceView(
                    serverURL: URL(string: "https://unused.invalid")!,
                    service: service, onSignOut: {}
                )
            }
        }
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
