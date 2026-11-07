import Foundation
import Testing

@testable import FolioKit

@MainActor struct SourceOutcomesTests {
    private let vault = UUID()
    private let source = UUID()

    @Test func failureIsSanitizedAndRetryLoadsOutcomes() async {
        let service = OutcomeServiceStub()
        let model = SourceOutcomesViewModel(vaultID: vault, sourceID: source, service: service)
        await service.fail()
        await model.load()
        #expect(model.error == "Could not load source outcomes. Try again.")
        #expect(model.outcomes.isEmpty)
        await service.respond(SourceOutcomePage(vaultID: vault, sourceID: source, outcomes: [outcome(1)]))
        await model.load()
        #expect(model.error == nil)
        #expect(model.outcomes.map(\.id) == [1])
        #expect(model.hasLoaded)
    }

    @Test func failedContinuationPreservesRowsAndRetriesSameCursor() async {
        let service = OutcomeServiceStub()
        let model = SourceOutcomesViewModel(vaultID: vault, sourceID: source, service: service)
        await service.respond(
            SourceOutcomePage(vaultID: vault, sourceID: source, outcomes: [outcome(5)], nextBefore: 5))
        await model.load()
        await service.fail()
        await model.loadMore()
        #expect(model.outcomes.map(\.id) == [5])
        #expect(model.canLoadMore)
        await service.respond(SourceOutcomePage(vaultID: vault, sourceID: source, outcomes: [outcome(4)]))
        await model.loadMore()
        #expect(model.outcomes.map(\.id) == [5, 4])
        #expect(await service.cursors == [nil, 5, 5])
    }

    @Test func failedRefreshRetriesTheFirstPageInsteadOfContinuingOldHistory() async {
        let service = OutcomeServiceStub()
        let model = SourceOutcomesViewModel(vaultID: vault, sourceID: source, service: service)
        await service.respond(
            SourceOutcomePage(vaultID: vault, sourceID: source, outcomes: [outcome(5)], nextBefore: 5))
        await model.load()
        await service.fail()
        await model.load()
        await service.respond(SourceOutcomePage(vaultID: vault, sourceID: source, outcomes: [outcome(6)]))
        await model.retry()
        #expect(model.outcomes.map(\.id) == [6])
        #expect(await service.cursors == [nil, nil, nil])
    }

    @Test func responseForAnotherVaultDoesNotExposeRows() async {
        let service = OutcomeServiceStub()
        await service.respond(SourceOutcomePage(vaultID: UUID(), sourceID: source, outcomes: [outcome(1)]))
        let model = SourceOutcomesViewModel(vaultID: vault, sourceID: source, service: service)
        await model.load()
        #expect(model.outcomes.isEmpty)
        #expect(model.error != nil)
    }

    @Test func nonAdvancingCursorCannotLoop() async {
        let service = OutcomeServiceStub()
        let model = SourceOutcomesViewModel(vaultID: vault, sourceID: source, service: service)
        await service.respond(
            SourceOutcomePage(vaultID: vault, sourceID: source, outcomes: [outcome(5)], nextBefore: 5))
        await model.load()
        await service.respond(
            SourceOutcomePage(vaultID: vault, sourceID: source, outcomes: [outcome(4)], nextBefore: 5))
        await model.loadMore()
        #expect(model.outcomes.map(\.id) == [5])
        #expect(model.error != nil)
    }

    @Test func cancellationDoesNotBecomeAUserFacingFailure() async {
        let service = OutcomeServiceStub()
        await service.cancel()
        let model = SourceOutcomesViewModel(vaultID: vault, sourceID: source, service: service)
        await model.load()
        #expect(model.error == nil)
        #expect(!model.isLoading)
        #expect(!model.hasLoaded)
    }

    private func outcome(_ id: Int64) -> SourceOutcome {
        SourceOutcome(
            id: id, itemID: UUID(), attemptID: UUID(), occurredAt: Date(timeIntervalSince1970: 0), outcome: .failed)
    }
}

private actor OutcomeServiceStub: SourceOutcomeService {
    private enum Failure: Error { case privateProviderError }
    private var result: Result<SourceOutcomePage, any Error> = .failure(Failure.privateProviderError)
    private(set) var cursors: [Int64?] = []

    func respond(_ page: SourceOutcomePage) { result = .success(page) }
    func fail() { result = .failure(Failure.privateProviderError) }
    func cancel() { result = .failure(CancellationError()) }

    func sourceOutcomes(vaultID: UUID, sourceID: UUID, before: Int64?) async throws -> SourceOutcomePage {
        cursors.append(before)
        return try result.get()
    }
}
