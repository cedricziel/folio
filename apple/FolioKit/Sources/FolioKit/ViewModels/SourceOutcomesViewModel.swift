import Foundation
import Observation

@MainActor @Observable
public final class SourceOutcomesViewModel {
    public private(set) var outcomes: [SourceOutcome] = []
    public private(set) var isLoading = false
    public private(set) var hasLoaded = false
    public private(set) var error: String?
    public var canLoadMore: Bool { nextBefore != nil }
    private let vaultID: UUID
    private let sourceID: UUID
    private let service: any SourceOutcomeService
    private var nextBefore: Int64?
    private var requestID = UUID()
    private var failedBefore: Int64?

    public init(vaultID: UUID, sourceID: UUID, service: any SourceOutcomeService) {
        self.vaultID = vaultID
        self.sourceID = sourceID
        self.service = service
    }

    public func load() async {
        await fetch(before: nil)
    }

    public func loadMore() async {
        guard !isLoading, let nextBefore else { return }
        await fetch(before: nextBefore)
    }

    public func retry() async {
        guard !isLoading, error != nil else { return }
        await fetch(before: failedBefore)
    }

    private func fetch(before: Int64?) async {
        let request = UUID()
        requestID = request
        isLoading = true
        error = nil
        failedBefore = before
        defer {
            if requestID == request { isLoading = false }
        }
        do {
            let page = try await service.sourceOutcomes(vaultID: vaultID, sourceID: sourceID, before: before)
            guard requestID == request, !Task.isCancelled else { return }
            let validContinuation =
                page.nextBefore.map { cursor in
                    cursor > 0 && cursor == page.outcomes.map(\.id).min()
                        && (before.map { cursor < $0 } ?? true)
                } ?? true
            guard page.vaultID == vaultID, page.sourceID == sourceID, validContinuation,
                page.outcomes.allSatisfy({ outcome in
                    outcome.id > 0 && (before.map { outcome.id < $0 } ?? true)
                })
            else {
                error = "Could not load source outcomes. Try again."
                return
            }
            var seen = Set(before == nil ? [] : outcomes.map(\.id))
            let appended = page.outcomes.filter { seen.insert($0.id).inserted }
            outcomes = before == nil ? appended : outcomes + appended
            nextBefore = page.nextBefore
            hasLoaded = true
        } catch is CancellationError {
        } catch {
            guard requestID == request, !Task.isCancelled else { return }
            self.error = "Could not load source outcomes. Try again."
        }
    }
}
