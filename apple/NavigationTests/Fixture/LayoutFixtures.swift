import FolioKit
import SwiftUI

@MainActor final class AppModel {
    static func makeService(for _: URL) -> any FolioService {
        MockFolioService()
    }

    static func signIn(to _: URL) async throws {}
    func connected(to _: URL) {}
}

struct OnboardingLayoutFixture: View {
    var body: some View {
        LayoutViewport {
            OnboardingView(model: AppModel())
        }
    }
}

struct ShareLayoutFixture: View {
    @State private var model: ShareViewModel?

    var body: some View {
        LayoutViewport {
            if let model {
                ShareView(model: model, onFinish: {})
            } else {
                ProgressView()
            }
        }
        .task {
            let service = MockFolioService()
            await service.setTags((1...16).map { Tag(name: "Archive category \($0)") })
            model = ShareViewModel(
                files: [UploadFile(data: Data(), filename: "Fixture.pdf", mimeType: "application/pdf")],
                service: service, background: FixtureBackgroundUploader()
            )
        }
    }
}

private struct LayoutViewport<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        content
            .environment(\.dynamicTypeSize, .accessibility5)
            .frame(width: 520, height: 360)
            .clipped()
            .overlay { Rectangle().stroke(.red) }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("layout-viewport")
            .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

private struct FixtureBackgroundUploader: BackgroundUploading {
    func enqueue(_: UploadFile) throws {}
}
