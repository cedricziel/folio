import Foundation
import Testing

@testable import FolioKit

struct WidgetSnapshotTests {
    private let vaultID = UUID(uuidString: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee")!
    private let documentID = UUID(uuidString: "3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f")!
    private let now = Date(timeIntervalSince1970: 1_800_000_000)

    private func makeStore() -> WidgetSnapshotStore {
        WidgetSnapshotStore(defaults: UserDefaults(suiteName: "WidgetSnapshotTests-\(UUID().uuidString)")!)
    }

    private func snapshot(title: String? = "Gas bill") -> WidgetSnapshot {
        WidgetSnapshot(
            isSignedIn: true, inboxCount: 3, inboxCountIsPartial: false,
            latest: .init(documentID: documentID, vaultID: vaultID, title: title, createdAt: now),
            hidesTitles: title == nil, updatedAt: now)
    }

    private func unreviewed(_ title: String, at offset: TimeInterval = 0) -> Document {
        Document(
            title: title, ocrStatus: .done, embedStatus: .done, reviewed: false,
            createdAt: now.addingTimeInterval(offset))
    }

    private func filed(_ title: String, at offset: TimeInterval = 0) -> Document {
        Document(
            title: title, ocrStatus: .done, embedStatus: .done, reviewed: true,
            createdAt: now.addingTimeInterval(offset))
    }

    // MARK: Store

    @Test func storeReadsBackWhatTheAppWrote() {
        let store = makeStore()
        #expect(store.load() == nil)

        store.save(snapshot())

        #expect(store.load() == snapshot())
    }

    @Test func storeIgnoresUnreadableData() {
        let defaults = UserDefaults(suiteName: "WidgetSnapshotTests-\(UUID().uuidString)")!
        defaults.set(Data("not json".utf8), forKey: WidgetSnapshotStore.key)

        #expect(WidgetSnapshotStore(defaults: defaults).load() == nil)
    }

    @Test func signingOutReplacesTheSnapshotWithoutDocuments() {
        let store = makeStore()
        store.save(snapshot())

        store.save(.signedOut(at: now))

        let saved = store.load()
        #expect(saved?.isSignedIn == false)
        #expect(saved?.latest == nil)
        #expect(saved?.inboxCount == 0)
    }

    // MARK: Privacy

    @Test func appLockHidesTheLatestTitle() {
        let hidden = snapshot().hidingTitles()

        #expect(hidden.latest?.title == nil)
        #expect(hidden.latest?.documentID == documentID)
        #expect(hidden.hidesTitles)
        #expect(hidden.inboxCount == 3)
    }

    @Test func turningOnAppLockRedactsTheStoredSnapshot() {
        let store = makeStore()
        store.save(snapshot())

        store.hideTitles()

        #expect(store.load()?.latest?.title == nil)
        #expect(store.load()?.hidesTitles == true)
    }

    @Test func hidingTitlesWithoutASnapshotWritesNothing() {
        let store = makeStore()

        store.hideTitles()

        #expect(store.load() == nil)
    }

    // MARK: Building from the server

    @Test func countsDocumentsThatNeedReviewAndKeepsTheNewest() async throws {
        let service = MockFolioService()
        let newest = unreviewed("Electricity bill", at: 60)
        await service.setPages([
            "|": DocumentPage(
                vaultID: vaultID, documents: [newest, filed("Lease"), unreviewed("Payslip", at: -60)])
        ])

        let snapshot = try await WidgetSnapshotBuilder.make(from: service, revealsTitles: true, now: now)

        #expect(snapshot.isSignedIn)
        #expect(snapshot.inboxCount == 2)
        #expect(!snapshot.inboxCountIsPartial)
        #expect(
            snapshot.latest
                == .init(
                    documentID: newest.id, vaultID: vaultID, title: "Electricity bill", createdAt: newest.createdAt))
        #expect(!snapshot.hidesTitles)
        #expect(snapshot.updatedAt == now)
    }

    @Test func appLockKeepsTitlesOutOfTheBuiltSnapshot() async throws {
        let service = MockFolioService()
        await service.setPages(["|": DocumentPage(vaultID: vaultID, documents: [unreviewed("Medical report")])])

        let snapshot = try await WidgetSnapshotBuilder.make(from: service, revealsTitles: false, now: now)

        #expect(snapshot.latest?.title == nil)
        #expect(snapshot.hidesTitles)
        #expect(snapshot.inboxCount == 1)
    }

    @Test func followsPagesUpToTheLimitThenMarksTheCountPartial() async throws {
        let service = MockFolioService()
        var pages: [String: DocumentPage] = [:]
        for index in 0..<WidgetSnapshotBuilder.maxPages {
            let key = index == 0 ? "|" : "|page\(index)"
            pages[key] = DocumentPage(
                vaultID: vaultID, documents: [unreviewed("Letter \(index)", at: -Double(index))],
                nextCursor: "page\(index + 1)")
        }
        await service.setPages(pages)

        let snapshot = try await WidgetSnapshotBuilder.make(from: service, revealsTitles: true, now: now)

        #expect(snapshot.inboxCount == WidgetSnapshotBuilder.maxPages)
        #expect(snapshot.inboxCountIsPartial)
        #expect(snapshot.latest?.title == "Letter 0")
    }

    @Test func anEmptyLibraryHasNoLatestDocument() async throws {
        let service = MockFolioService()
        await service.setPages(["|": DocumentPage(vaultID: vaultID, documents: [])])

        let snapshot = try await WidgetSnapshotBuilder.make(from: service, revealsTitles: true, now: now)

        #expect(snapshot.inboxCount == 0)
        #expect(snapshot.latest == nil)
    }

    // MARK: Timeline

    @Test func withoutASnapshotTheWidgetAsksToOpenFolio() {
        let entries = FolioWidgetEntry.timeline(for: nil, now: now)

        #expect(entries == [FolioWidgetEntry(date: now, content: .notSetUp)])
    }

    @Test func signedOutSnapshotShowsTheSignedOutState() {
        let entries = FolioWidgetEntry.timeline(for: .signedOut(at: now), now: now)

        #expect(entries.map(\.content) == [.signedOut])
    }

    @Test func freshSnapshotTurnsStaleLater() {
        let entries = FolioWidgetEntry.timeline(for: snapshot(), now: now.addingTimeInterval(60))

        #expect(entries.count == 2)
        #expect(entries[0].date == now.addingTimeInterval(60))
        #expect(entries[0].content == .documents(snapshot(), isStale: false))
        #expect(entries[1].date == now.addingTimeInterval(FolioWidgetEntry.staleAfter))
        #expect(entries[1].content == .documents(snapshot(), isStale: true))
    }

    @Test func oldSnapshotIsStaleRightAway() {
        let later = now.addingTimeInterval(FolioWidgetEntry.staleAfter + 1)

        let entries = FolioWidgetEntry.timeline(for: snapshot(), now: later)

        #expect(entries == [FolioWidgetEntry(date: later, content: .documents(snapshot(), isStale: true))])
    }

    @Test func entryNeverRevealsATitleTheSnapshotHid() {
        let entries = FolioWidgetEntry.timeline(for: snapshot().hidingTitles(), now: now)

        for entry in entries {
            guard case let .documents(shown, _) = entry.content else {
                Issue.record("Expected documents")
                continue
            }
            #expect(shown.latest?.title == nil)
        }
    }

    @Test(arguments: [(0, false, "0"), (7, false, "7"), (300, true, "300+")])
    func inboxCountText(count: Int, partial: Bool, text: String) {
        var shown = snapshot()
        shown.inboxCount = count
        shown.inboxCountIsPartial = partial

        #expect(shown.inboxCountText == text)
    }
}

struct WidgetLinkTests {
    private let vaultID = UUID(uuidString: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee")!
    private let documentID = UUID(uuidString: "3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f")!

    @Test func linksSurviveTheRoundTrip() {
        for link in [WidgetLink.inbox, .scan, .document(documentID, vaultID: vaultID)] {
            #expect(WidgetLink(url: link.url) == link)
        }
    }

    @Test func linksBecomeAppRequests() {
        #expect(WidgetLink.inbox.request == .inbox)
        #expect(WidgetLink.scan.request == .scan)
        #expect(WidgetLink.document(documentID, vaultID: vaultID).request == .document(documentID, vaultID: vaultID))
    }

    @Test func schemeMatchesWhateverCaseTheSystemHandsBack() {
        let upper = URL(string: WidgetLink.inbox.url.absoluteString.replacingOccurrences(
            of: WidgetLink.scheme, with: WidgetLink.scheme.uppercased()))!

        #expect(WidgetLink(url: upper) == .inbox)
    }

    @Test(arguments: [
        "https://folio.example/inbox",
        "com.cedricziel.folio.widgets://unknown",
        "com.cedricziel.folio.widgets://document/not-a-uuid/3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f",
        "com.cedricziel.folio.widgets://document/3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f",
        "com.cedricziel.folio:/oauth/callback",
    ])
    func foreignOrMalformedURLsAreIgnored(text: String) {
        #expect(WidgetLink(url: URL(string: text)!) == nil)
    }

    /// `ShortcutsTests` already checks that performing the intent leaves `.scan` for the app.
    @Test func scanControlIntentOpensTheApp() {
        #expect(ScanDocumentIntent.openAppWhenRun)
    }
}
