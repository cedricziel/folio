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

struct TelemetrySessionTests {
    private let clock = TestClock()

    private func store(_ directory: URL? = nil) -> TelemetrySession {
        let clock = clock
        return TelemetrySession(directory: directory, now: { clock.now })
    }

    @Test func activityKeepsOneSession() {
        let sessions = store()
        let first = sessions.touch()
        #expect(first.boundaries == [.start(id: first.id, previousID: nil)])
        for _ in 0..<6 {
            clock.advance(10 * 60)
            let next = sessions.touch()
            #expect(next.id == first.id)
            #expect(next.boundaries.isEmpty)
        }
    }

    @Test func inactivityEndsTheSessionAndLinksTheNext() {
        let sessions = store()
        let first = sessions.touch()
        let lastActivity = clock.now
        clock.advance(TelemetrySession.inactivity + 1)
        let next = sessions.touch()
        #expect(next.id != first.id)
        #expect(
            next.boundaries == [
                .end(id: first.id, reason: .timeout, at: lastActivity), .start(id: next.id, previousID: first.id),
            ])
    }

    @Test func fourHoursEndTheSessionDespiteActivity() {
        let sessions = store()
        let first = sessions.touch()
        for _ in 0..<11 {
            clock.advance(20 * 60)
            #expect(sessions.touch().id == first.id)
        }
        clock.advance(20 * 60)
        let next = sessions.touch()
        #expect(next.boundaries.first == .end(id: first.id, reason: .maxDuration, at: clock.now - 20 * 60))
        #expect(next.boundaries.last == .start(id: next.id, previousID: first.id))
    }

    @Test func clockSetBackExpiresTheSession() {
        let sessions = store()
        let first = sessions.touch()
        clock.advance(-60 * 60)
        let next = sessions.touch()
        #expect(next.id != first.id)
        guard case .end(_, let reason, _)? = next.boundaries.first else {
            Issue.record("expected the old session to end")
            return
        }
        #expect(reason == .timeout)
    }

    @Test func readingNeverStartsOrExtendsASession() {
        let sessions = store()
        #expect(sessions.current() == nil)
        let first = sessions.touch()
        clock.advance(20 * 60)
        #expect(sessions.current() == first.id)
        clock.advance(11 * 60)
        #expect(sessions.current() == nil)
    }

    @Test(arguments: [SessionEndReason.signOut, .serverChange])
    func endingForgetsTheSessionWithoutLinkingTheNext(reason: SessionEndReason) {
        let sessions = store()
        let first = sessions.touch()
        #expect(sessions.end(reason) == [.end(id: first.id, reason: reason, at: clock.now)])
        #expect(sessions.current() == nil)
        let next = sessions.touch()
        #expect(next.boundaries == [.start(id: next.id, previousID: nil)])
    }

    @Test func anotherProcessAdoptsTheLiveSession() throws {
        let directory = try sharedDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let app = store(directory)
        let share = store(directory)
        let started = app.touch()
        clock.advance(5 * 60)
        let joined = share.touch()
        #expect(joined.id == started.id)
        #expect(joined.boundaries.isEmpty)
    }

    @Test func aRotationIsSeenByTheOtherProcess() throws {
        let directory = try sharedDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let app = store(directory)
        let fileProvider = store(directory)
        let first = app.touch()
        #expect(fileProvider.current() == first.id)
        clock.advance(TelemetrySession.inactivity + 1)
        #expect(fileProvider.current() == nil)
        let next = app.touch()
        #expect(fileProvider.current() == next.id)
    }

    @Test func anExtensionRunningAcrossSignOutDoesNotCarryTheOldSession() throws {
        let directory = try sharedDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let app = store(directory)
        let share = store(directory)
        let old = app.touch()
        #expect(share.touch().id == old.id)
        _ = app.end(.signOut)
        let signedIn = app.touch()
        let next = share.touch()
        #expect(next.id == signedIn.id)
        #expect(next.id != old.id)
        #expect(signedIn.boundaries == [.start(id: signedIn.id, previousID: nil)])
    }

    @Test func concurrentRotationsInTwoProcessesStartOneSession() async throws {
        let directory = try sharedDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let app = store(directory)
        let share = store(directory)
        _ = app.touch()
        clock.advance(TelemetrySession.inactivity + 1)
        let results = await withTaskGroup(of: (String, [SessionBoundary]).self) { group in
            for index in 0..<40 {
                let sessions = index.isMultiple(of: 2) ? app : share
                group.addTask {
                    let result = sessions.touch()
                    return (result.id, result.boundaries)
                }
            }
            var all: [(String, [SessionBoundary])] = []
            for await result in group { all.append(result) }
            return all
        }
        #expect(Set(results.map { $0.0 }).count == 1)
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
        #expect(limiter.allow(at: start + .seconds(61)))
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
                "open_document", "add_tag", "remove_tag", "get_document_file",
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
