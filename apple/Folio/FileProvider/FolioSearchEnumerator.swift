#if os(macOS)
    import FileProvider
    import Synchronization

    final class FolioSearchEnumerator: NSObject, NSFileProviderSearchEnumerator, Sendable {
        typealias SearchResult = any NSFileProviderSearchResult & Sendable

        private enum State {
            case idle
            case running(Task<Void, Never>)
            case ready([SearchResult], offset: Int)
            case finished
            case invalidated
        }

        private let state = Mutex<State>(.idle)
        private let desiredCount: Int
        private let search: @Sendable (Int) async throws -> [SearchResult]

        init(desiredCount: Int, search: @escaping @Sendable (Int) async throws -> [SearchResult]) {
            self.desiredCount = desiredCount
            self.search = search
        }

        func invalidate() {
            let task = state.withLock { state -> Task<Void, Never>? in
                defer { state = .invalidated }
                guard case let .running(task) = state else { return nil }
                return task
            }
            task?.cancel()
        }

        private func completeSearch(with next: State) -> Bool {
            state.withLock { state in
                guard case .running = state else { return false }
                state = next
                return true
            }
        }

        private func deliverPage(to observer: any NSFileProviderSearchEnumerationObserver, token: NSFileProviderPage?) {
            let pageSize = min(max(0, observer.maximumNumberOfResultsPerPage), 50)
            let batch = state.withLock { state -> ([SearchResult], NSFileProviderPage?) in
                guard case let .ready(results, offset) = state else { return ([], nil) }
                let expectedToken = offset == 0 ? nil : NSFileProviderPage(Data([UInt8(offset)]))
                guard token == expectedToken, pageSize > 0 else { return ([], nil) }
                let end = min(offset + pageSize, results.count)
                let next = end < results.count ? NSFileProviderPage(Data([UInt8(end)])) : nil
                state = next == nil ? .finished : .ready(results, offset: end)
                return (Array(results[offset..<end]), next)
            }
            if !batch.0.isEmpty { observer.didEnumerate(batch.0) }
            observer.finishEnumerating(upTo: batch.1)
        }

        func enumerateSearchResults(
            for observer: any NSFileProviderSearchEnumerationObserver, startingAt page: NSFileProviderPage?
        ) {
            if page != nil {
                deliverPage(to: observer, token: page)
                return
            }
            let limit = min(desiredCount, 50)
            guard limit > 0, observer.maximumNumberOfResultsPerPage > 0 else {
                observer.finishEnumerating(upTo: nil)
                return
            }
            let reply = SearchReply(observer: observer)
            let started = state.withLock { state in
                guard case .idle = state else { return false }
                state = .running(
                    Task { [weak self, search] in
                        do {
                            try Task.checkCancellation()
                            let results = try await search(limit)
                            guard let self,
                                completeSearch(with: .ready(Array(results.prefix(limit)), offset: 0))
                            else { return }
                            deliverPage(to: reply.observer, token: nil)
                        } catch {
                            guard self?.completeSearch(with: .finished) == true else { return }
                            reply.observer.finishEnumeratingWithError(error.providerError)
                        }
                    })
                return true
            }
            if !started {
                observer.finishEnumerating(upTo: nil)
            }
        }
    }

    private struct SearchReply: @unchecked Sendable {
        let observer: any NSFileProviderSearchEnumerationObserver
    }
#endif
