import Foundation
import Testing

@testable import FolioKit

@MainActor struct SourceActivityTests {
    @Test func failedLoadIsSanitizedAndRetryLoadsSources() async {
        let vault = UUID()
        let service = ActivityStub()
        let model = SourceActivityViewModel(vaultID: vault, service: service)
        await model.load()
        #expect(model.error == "Could not load sources. Try again.")
        await service.respond([
            SourceActivity(id: UUID(), vaultID: vault, name: "Mailbox", health: .unavailable, lastCheckedAt: nil)
        ])
        await model.load()
        #expect(model.error == nil)
        #expect(model.sources.first?.health == .unavailable)
        #expect(model.hasLoaded)
    }

    @Test func anotherVaultCannotSupplyTheSourceList() async {
        let service = ActivityStub()
        await service.respond([
            SourceActivity(id: UUID(), vaultID: UUID(), name: "Private", health: .available, lastCheckedAt: nil)
        ])
        let model = SourceActivityViewModel(vaultID: UUID(), service: service)
        await model.load()
        #expect(model.sources.isEmpty)
        #expect(model.error != nil)
    }
}

private actor ActivityStub: SourceActivityService {
    private enum Failure: Error { case privateProviderError }
    private var sources: [SourceActivity]?
    func respond(_ value: [SourceActivity]) { sources = value }
    func sourceActivity(vaultID: UUID) async throws -> [SourceActivity] {
        guard let sources else { throw Failure.privateProviderError }
        return sources
    }
    func sourceOutcomes(vaultID: UUID, sourceID: UUID, before: Int64?) async throws -> SourceOutcomePage {
        SourceOutcomePage(vaultID: vaultID, sourceID: sourceID, outcomes: [])
    }
}
