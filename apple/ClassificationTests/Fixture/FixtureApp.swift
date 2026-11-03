import FolioKit
import SwiftUI

@main struct ClassificationFixtureApp: App {
    var body: some Scene {
        WindowGroup { ClassificationFixture() }
            #if os(macOS)
                .defaultSize(width: 640, height: 760)
            #endif
    }
}

@MainActor private struct ClassificationFixture: View {
    @State private var service: ClassificationFixtureService
    private let readOnly: Bool
    private let largeText: Bool

    init() {
        let arguments = ProcessInfo.processInfo.arguments
        let readOnly = arguments.contains("--read-only")
        let largeText = arguments.contains("--large-text")
        self.readOnly = readOnly
        self.largeText = largeText
        _service = State(
            initialValue: ClassificationFixtureService(
                readOnly: readOnly,
                failInitialLoad: arguments.contains("--initial-load-error"),
                longNames: largeText
            ))
    }

    var body: some View {
        NavigationStack {
            VStack(spacing: FolioTheme.Spacing.space2) {
                TaxonomyView(service: service, canManage: !readOnly)
                Text(service.mutationNote)
                    .font(FolioTheme.Typography.meta)
                    .foregroundStyle(Color.inkMuted)
                    .padding(.horizontal, FolioTheme.Spacing.space4)
                    .padding(.bottom, FolioTheme.Spacing.space2)
                    .accessibilityIdentifier("fixture-state")
            }
            .background(Color.paper)
        }
        .dynamicTypeSize(largeText ? .accessibility3 : .large)
    }
}
