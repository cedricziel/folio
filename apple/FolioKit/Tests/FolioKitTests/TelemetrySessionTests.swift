import Foundation
import OpenTelemetryApi
import Testing

@testable import FolioKit

private final class TestClock: @unchecked Sendable {
    private let lock = NSLock()
    private var value = Date(timeIntervalSince1970: 1_800_000_000)

    var now: Date { lock.withLock { value } }

    func advance(_ seconds: TimeInterval) {
        lock.withLock { value = value.addingTimeInterval(seconds) }
    }
}

private func sharedDirectory() throws -> URL {
    let url = FileManager.default.temporaryDirectory.appending(path: "folio-session-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    return url
}

// A hang in locking must fail this suite, not stall the whole test run.
@Suite(.timeLimit(.minutes(1))) struct TelemetrySessionTests {
    private let clock = TestClock()

    /// A store over `directory` (or memory), signed in unless the test is about signing out.
    private func store(_ directory: URL? = nil, signedIn: Bool = true) -> TelemetrySession {
        let clock = clock
        let sessions = TelemetrySession(directory: directory, now: { clock.now })
        if signedIn { sessions.signIn() }
        return sessions
    }

    @Test func activityKeepsOneSession() throws {
        let sessions = store()
        let first = sessions.touch()
        let id = try #require(first.id)
        #expect(first.boundaries == [.start(id: id, previousID: nil)])
        for _ in 0..<6 {
            clock.advance(10 * 60)
            let next = sessions.touch()
            #expect(next.id == id)
            #expect(next.boundaries.isEmpty)
        }
    }

    @Test func inactivityEndsTheSessionAndLinksTheNext() throws {
        let sessions = store()
        let first = try #require(sessions.touch().id)
        let lastActivity = clock.now
        clock.advance(TelemetrySession.inactivity + 1)
        let next = sessions.touch()
        let nextID = try #require(next.id)
        #expect(nextID != first)
        #expect(
            next.boundaries == [
                .end(id: first, reason: .timeout, at: lastActivity), .start(id: nextID, previousID: first),
            ])
    }

    @Test func fourHoursEndTheSessionDespiteActivity() throws {
        let sessions = store()
        let first = try #require(sessions.touch().id)
        for _ in 0..<11 {
            clock.advance(20 * 60)
            #expect(sessions.touch().id == first)
        }
        clock.advance(20 * 60)
        let next = sessions.touch()
        let nextID = try #require(next.id)
        #expect(next.boundaries.first == .end(id: first, reason: .maxDuration, at: clock.now - 20 * 60))
        #expect(next.boundaries.last == .start(id: nextID, previousID: first))
    }

    @Test func clockSetBackExpiresTheSession() throws {
        let sessions = store()
        let first = try #require(sessions.touch().id)
        clock.advance(-60 * 60)
        let next = sessions.touch()
        #expect(next.id != first)
        guard case .end(_, let reason, _)? = next.boundaries.first else {
            Issue.record("expected the old session to end")
            return
        }
        #expect(reason == .timeout)
    }

    @Test func readingNeverStartsOrExtendsASession() throws {
        let sessions = store()
        #expect(sessions.current() == nil)
        let first = try #require(sessions.touch().id)
        clock.advance(20 * 60)
        #expect(sessions.current() == first)
        clock.advance(11 * 60)
        #expect(sessions.current() == nil)
    }

    @Test(arguments: [SessionEndReason.signOut, .serverChange])
    func endingForgetsTheSessionWithoutLinkingTheNext(reason: SessionEndReason) throws {
        let sessions = store()
        let first = try #require(sessions.touch().id)
        #expect(sessions.end(reason) == [.end(id: first, reason: reason, at: clock.now)])
        #expect(sessions.current() == nil)
        sessions.signIn()
        let next = sessions.touch()
        let nextID = try #require(next.id)
        #expect(next.boundaries == [.start(id: nextID, previousID: nil)])
    }

    @Test func noSessionStartsBeforeTheAppSignsIn() throws {
        let directory = try sharedDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let share = store(directory, signedIn: false)
        #expect(share.touch().id == nil)
        #expect(share.current() == nil)
        _ = store(directory)
        #expect(share.touch().id != nil)
    }

    @Test func anotherProcessAdoptsTheLiveSession() throws {
        let directory = try sharedDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let app = store(directory)
        let share = store(directory, signedIn: false)
        let started = try #require(app.touch().id)
        clock.advance(5 * 60)
        let joined = share.touch()
        #expect(joined.id == started)
        #expect(joined.boundaries.isEmpty)
    }

    @Test func aRotationIsSeenByTheOtherProcess() throws {
        let directory = try sharedDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let app = store(directory)
        let fileProvider = store(directory, signedIn: false)
        let first = try #require(app.touch().id)
        #expect(fileProvider.current() == first)
        clock.advance(TelemetrySession.inactivity + 1)
        #expect(fileProvider.current() == nil)
        let next = try #require(app.touch().id)
        #expect(fileProvider.current() == next)
    }

    @Test func anExtensionStillRunningAfterSignOutCannotStartASessionTheNextAccountAdopts() throws {
        let directory = try sharedDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let app = store(directory)
        let share = store(directory, signedIn: false)
        let old = try #require(app.touch().id)
        #expect(share.touch().id == old)

        _ = app.end(.signOut)
        // Still running after sign-out: it neither keeps the old session nor writes a new one.
        #expect(share.touch().id == nil)
        #expect(share.current() == nil)

        app.signIn()
        let signedIn = app.touch()
        let next = try #require(signedIn.id)
        #expect(next != old)
        #expect(signedIn.boundaries == [.start(id: next, previousID: nil)])
        #expect(share.touch().id == next)
    }

    @Test func concurrentRotationsInTwoProcessesStartOneSession() throws {
        let directory = try sharedDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let app = store(directory)
        let share = store(directory, signedIn: false)
        _ = app.touch()
        clock.advance(TelemetrySession.inactivity + 1)
        // flock blocks its thread, so the contention runs on Dispatch threads, never the shared Swift
        // concurrency pool that every other suite needs.
        let collected = NSLock()
        nonisolated(unsafe) var results: [(String?, [SessionBoundary])] = []
        DispatchQueue.concurrentPerform(iterations: 40) { index in
            let result = (index.isMultiple(of: 2) ? app : share).touch()
            collected.withLock { results.append((result.id, result.boundaries)) }
        }
        #expect(Set(results.compactMap { $0.0 }).count == 1)
        #expect(results.allSatisfy { $0.0 != nil })
        let starts = results.flatMap { $0.1 }.filter {
            if case .start = $0 { return true }
            return false
        }
        #expect(starts.count == 1)
    }

    @Test func atMostSixtyUIEventsAMinute() {
        var limiter = UIEventLimiter()
        let start = ContinuousClock.now
        let allowed = (0..<100).filter { _ in limiter.allow(at: start) }.count
        #expect(allowed == 60)
        #expect(limiter.takeDropped() == 40)
        #expect(limiter.takeDropped() == 0)
        let allowedLater = limiter.allow(at: start + .seconds(61))
        #expect(allowedLater)
    }

    @Test func vocabularyMatchesTheSpecification() {
        #expect(
            Set(TelemetryScreen.allCases.map(\.rawValue)) == [
                "onboarding", "app_lock", "inbox", "documents", "saved_view", "saved_view.editor",
                "document.detail", "document.preview", "document.discussion", "document.processing",
                "scan.review", "upload.review", "taxonomy", "sources", "source.outcomes", "share",
            ])
        #expect(TelemetryAction.intentRun.rawValue == "intent.run")
        #expect(TelemetryAction.signOut.rawValue == "auth.sign_out")
        #expect(TelemetryAction.tagAdd.rawValue == "document.tag.add")
        #expect(
            Set(TelemetryIntent.allCases.map(\.rawValue)) == [
                "scan_document", "upload_file", "search_documents", "find_documents", "get_latest_document",
                "open_document", "add_tag", "remove_tag", "get_document_file", "tag_documents", "untag_documents",
            ])
        #expect(Set(BrowseScope.allCases.map(\.rawValue)) == ["inbox", "all", "saved_view", "intent"])
    }

    @Test func browseAttributesNameFilterKindsNotValues() {
        var filter = DocumentFilter()
        let tag = UUID()
        filter.tagIDs = [tag]
        filter.reviewed = false
        let query = DocumentBrowseQuery(text: "electricity bill", filter: filter, sort: .oldest, scope: .inbox)
        let attributes = query.telemetryAttributes(cursor: "opaque-cursor")
        #expect(attributes["folio.browse.scope"] == .string("inbox"))
        #expect(
            attributes["folio.browse.filters"]
                == .array(AttributeArray(values: [.string("reviewed"), .string("tag"), .string("text")])))
        #expect(attributes["folio.browse.sort"] == .string("oldest"))
        #expect(attributes["folio.browse.page"] == .string("next"))
        let exported = attributes.values.map { "\($0)" }.joined()
        #expect(!exported.contains(tag.uuidString.lowercased()))
        #expect(!exported.contains("electricity"))
        #expect(!exported.contains("opaque-cursor"))
        #expect(
            DocumentBrowseQuery().telemetryAttributes(cursor: nil)["folio.browse.filters"]
                == .array(AttributeArray(values: [])))
    }

    @MainActor @Test func listsNameTheirBrowseScope() {
        let list = DocumentListViewModel(service: MockFolioService())
        list.scope = .inbox
        #expect(list.query.scope == .inbox)
        list.scope = .all
        #expect(list.query.scope == .all)
        list.showsSavedView = true
        #expect(list.query.scope == .savedView)
    }

    @Test func browseScopeIsNotPartOfQueryEquality() {
        #expect(DocumentBrowseQuery(scope: .inbox) == DocumentBrowseQuery(scope: .savedView))
    }
}
