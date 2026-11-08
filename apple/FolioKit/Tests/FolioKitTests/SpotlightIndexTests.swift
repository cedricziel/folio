import Foundation
import Testing

@testable import FolioKit

/// Records Spotlight calls; `holdNextIndex()` suspends the next index call until `release()`.
actor FakeSpotlightIndex: SpotlightIndexing {
    enum Call: Equatable {
        case index([String])
        case delete([String])
    }

    private(set) var items: [String: SpotlightItem] = [:]
    private(set) var calls: [Call] = []
    private(set) var heldIndexCalls = 0
    private var holding = false
    private var gate: CheckedContinuation<Void, Never>?

    func holdNextIndex() { holding = true }

    func release() {
        holding = false
        gate?.resume()
        gate = nil
    }

    func index(_ items: [SpotlightItem]) async throws {
        if holding {
            heldIndexCalls += 1
            await withCheckedContinuation { gate = $0 }
        }
        calls.append(.index(items.map(\.identifier)))
        for item in items { self.items[item.identifier] = item }
    }

    /// Domains are hierarchical, as in Core Spotlight: deleting "a.b" also deletes "a.b.c".
    func deleteDomains(_ domains: [String]) async throws {
        calls.append(.delete(domains))
        items = items.filter { _, item in
            !domains.contains { item.domain == $0 || item.domain.hasPrefix($0 + ".") }
        }
    }

    var documentIDs: Set<UUID> {
        Set(items.values.map(\.documentID))
    }
}

private let serverA = URL(string: "https://a.folio.example")!
private let serverB = URL(string: "https://b.folio.example")!
private let vault = UUID(uuidString: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee")!

private func summary(_ title: String, id: UUID = UUID()) -> DocumentSummary {
    DocumentSummary(id: id, title: title, date: Date(timeIntervalSince1970: 0))
}

private func scopeDefaults() -> UserDefaults {
    UserDefaults(suiteName: "SpotlightIndexTests-\(UUID().uuidString)")!
}

private func until(_ condition: @Sendable () async -> Bool) async throws {
    for _ in 0..<500 {
        if await condition() { return }
        try await Task.sleep(for: .milliseconds(5))
    }
    Issue.record("condition never became true")
}

struct SpotlightIndexTests {
    @Test func itemsCarryTheVaultLinkAndAServerScopedDocumentDomain() {
        let document = summary("Household letter", id: UUID(uuidString: "3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f")!)
        let item = SpotlightItem(document, vaultID: vault, server: serverA)

        #expect(
            DocumentLink(spotlightIdentifier: item.identifier)
                == DocumentLink(documentID: document.id, vaultID: vault))
        #expect(item.domain.hasPrefix(SpotlightIndex.domain(for: serverA) + "."))
        #expect(item.domain.hasSuffix(".3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f"))
        #expect(SpotlightIndex.domain(for: serverA).hasPrefix(SpotlightIndex.rootDomain + "."))
        #expect(SpotlightIndex.domain(for: serverA) != SpotlightIndex.domain(for: serverB))
        #expect(item.searchableItem.domainIdentifier == item.domain)
        #expect(item.searchableItem.attributeSet.title == "Household letter")
    }

    @Test func nothingIsDonatedWithoutASignedInSession() async {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())

        await spotlight.donate(summary("Before sign-in"), vaultID: vault)
        await spotlight.sessionChanged(to: .expired(serverA)).value
        await spotlight.donate(summary("Expired"), vaultID: vault)

        #expect(await fake.items.isEmpty)
        #expect(await !fake.calls.contains { if case .index = $0 { true } else { false } })
    }

    @Test func removingADocumentDeletesOnlyItsEntries() async {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        let kept = summary("Keep")
        let gone = summary("Gone")
        await spotlight.donate(kept, vaultID: vault)
        await spotlight.donate(gone, vaultID: vault)

        await spotlight.remove(documentID: gone.id).value

        #expect(await fake.documentIDs == [kept.id])
    }

    @Test func signingOutClearsEveryEntry() async {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        await spotlight.donate(summary("Private"), vaultID: vault)

        await spotlight.sessionChanged(to: .signedOut).value

        #expect(await fake.items.isEmpty)
        #expect(await fake.calls.last == .delete([SpotlightIndex.rootDomain]))
    }

    @Test func revokedSessionClearsEntriesButAnInterruptedOneKeepsThem() async {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        await spotlight.donate(summary("Private"), vaultID: vault)

        await spotlight.sessionChanged(to: .interrupted(serverA)).value
        await spotlight.donate(summary("While interrupted"), vaultID: vault)
        #expect(await fake.items.count == 1)

        await spotlight.sessionChanged(to: .expired(serverA)).value
        #expect(await fake.items.isEmpty)
    }

    @Test func relaunchOnTheSameServerKeepsEntriesAndAnotherServerClearsThem() async {
        let defaults = scopeDefaults()
        let fake = FakeSpotlightIndex()
        let first = SpotlightIndex(index: fake, defaults: defaults)
        await first.sessionChanged(to: .signedIn(serverA)).value
        await first.donate(summary("Private"), vaultID: vault)

        let relaunched = SpotlightIndex(index: fake, defaults: defaults)
        await relaunched.sessionChanged(to: .signedIn(serverA)).value
        #expect(await fake.items.count == 1)

        let switched = SpotlightIndex(index: fake, defaults: defaults)
        await switched.sessionChanged(to: .signedIn(serverB)).value
        #expect(await fake.items.isEmpty)
    }

    @Test func firstActivationPurgesEntriesFromBeforeScoping() async {
        let fake = FakeSpotlightIndex()
        let legacy = SpotlightItem(
            identifier: DocumentLink(documentID: UUID(), vaultID: vault).spotlightIdentifier,
            domain: SpotlightIndex.rootDomain, summary: summary("Legacy"))
        try? await fake.index([legacy])
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())

        await spotlight.sessionChanged(to: .signedIn(serverA)).value

        #expect(await fake.items.isEmpty)
    }

    @Test func signingInAgainOnTheSameServerClearsThePreviousAccount() async {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        await spotlight.donate(summary("Account X"), vaultID: vault)
        await spotlight.sessionChanged(to: .interrupted(serverA)).value

        await spotlight.signedIn(to: serverA).value
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        let fromY = summary("Account Y")
        await spotlight.donate(fromY, vaultID: vault)

        #expect(await fake.documentIDs == [fromY.id])
    }

    @Test func removalStillAppliesWhileTheSessionIsInterrupted() async {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        let gone = summary("Gone")
        await spotlight.donate(gone, vaultID: vault)

        await spotlight.sessionChanged(to: .unavailable(server: serverA, message: "keychain")).value
        await spotlight.remove(documentID: gone.id).value

        #expect(await fake.items.isEmpty)
    }

    @Test func donationInFlightDuringSignOutDoesNotSurviveIt() async throws {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        await fake.holdNextIndex()

        let donation = Task { await spotlight.donate(summary("Late"), vaultID: vault) }
        try await until { await fake.heldIndexCalls == 1 }
        let signOut = spotlight.sessionChanged(to: .signedOut)
        await fake.release()
        await donation.value
        await signOut.value

        #expect(await fake.items.isEmpty)
        #expect(await fake.calls.last == .delete([SpotlightIndex.rootDomain]))
    }

    @Test func donationRequestedAfterSignOutIsDropped() async {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        let staleScope = spotlight.currentScope

        spotlight.sessionChanged(to: .signedOut)
        await spotlight.donate(summary("Late"), vaultID: vault)
        if let staleScope {
            await spotlight.donate(summary("Stale scope"), vaultID: vault, scope: staleScope).value
        }

        #expect(staleScope != nil)
        #expect(await fake.items.isEmpty)
    }

    @Test func removalBeforeASessionIsRestoredUsesTheGivenServer() async {
        let fake = FakeSpotlightIndex()
        let first = SpotlightIndex(index: fake, defaults: scopeDefaults())
        await first.sessionChanged(to: .signedIn(serverA)).value
        let gone = summary("Gone")
        await first.donate(gone, vaultID: vault)

        let relaunched = SpotlightIndex(index: fake, defaults: scopeDefaults())
        await relaunched.remove(documentIDs: [gone.id]).value
        #expect(await fake.documentIDs == [gone.id])

        await relaunched.remove(documentIDs: [gone.id], fallbackServer: serverA).value
        #expect(await fake.items.isEmpty)
    }

    @Test func listedDocumentsAreIndexedOnceUntilTheyChange() async {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        let a = summary("A"), b = summary("B")

        await spotlight.index([a, b], vaultID: vault).value
        await spotlight.index([a, b], vaultID: vault).value
        var renamed = b
        renamed.title = "B, renamed"
        await spotlight.index([a, renamed], vaultID: vault).value

        #expect(await fake.documentIDs == [a.id, b.id])
        #expect(await fake.calls.filter { if case .index = $0 { true } else { false } }.count == 2)
        #expect(await fake.items.values.first { $0.documentID == b.id }?.summary.title == "B, renamed")
    }

    @Test func listIndexingStopsAddingDocumentsAtTheLimit() async {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults(), libraryLimit: 2)
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        let a = summary("A"), b = summary("B"), c = summary("C")

        await spotlight.index([a, b, c], vaultID: vault).value
        var renamed = a
        renamed.title = "A, renamed"
        await spotlight.index([renamed, c], vaultID: vault).value

        #expect(await fake.documentIDs == [a.id, b.id])
        #expect(await fake.items.values.first { $0.documentID == a.id }?.summary.title == "A, renamed")
    }

    @Test func theLimitHoldsAcrossLaunchesOnTheSameServer() async {
        let defaults = scopeDefaults()
        let fake = FakeSpotlightIndex()
        let first = SpotlightIndex(index: fake, defaults: defaults, libraryLimit: 2)
        await first.sessionChanged(to: .signedIn(serverA)).value
        let a = summary("A"), b = summary("B"), c = summary("C")
        await first.index([a, b], vaultID: vault).value

        let relaunched = SpotlightIndex(index: fake, defaults: defaults, libraryLimit: 2)
        await relaunched.sessionChanged(to: .signedIn(serverA)).value
        var renamed = a
        renamed.title = "A, renamed"
        await relaunched.index([c, renamed], vaultID: vault).value

        #expect(await fake.documentIDs == [a.id, b.id])
        #expect(await fake.items.values.first { $0.documentID == a.id }?.summary.title == "A, renamed")
    }

    @Test func removingOrClearingFreesRoomUnderTheLimit() async {
        let defaults = scopeDefaults()
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: defaults, libraryLimit: 1)
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        let a = summary("A"), b = summary("B"), c = summary("C")
        await spotlight.index([a], vaultID: vault).value

        await spotlight.remove(documentID: a.id).value
        await spotlight.index([b], vaultID: vault).value
        #expect(await fake.documentIDs == [b.id])

        await spotlight.sessionChanged(to: .signedOut).value
        let relaunched = SpotlightIndex(index: fake, defaults: defaults, libraryLimit: 1)
        await relaunched.signedIn(to: serverA).value
        await relaunched.sessionChanged(to: .signedIn(serverA)).value
        await relaunched.index([c], vaultID: vault).value
        #expect(await fake.documentIDs == [c.id])
    }

    @Test func anotherServerStartsWithAnEmptyLimit() async {
        let defaults = scopeDefaults()
        let fake = FakeSpotlightIndex()
        let first = SpotlightIndex(index: fake, defaults: defaults, libraryLimit: 1)
        await first.sessionChanged(to: .signedIn(serverA)).value
        await first.index([summary("On A")], vaultID: vault).value

        let switched = SpotlightIndex(index: fake, defaults: defaults, libraryLimit: 1)
        await switched.sessionChanged(to: .signedIn(serverB)).value
        let onB = summary("On B")
        await switched.index([onB], vaultID: vault).value

        #expect(await fake.documentIDs == [onB.id])
    }

    @Test func listIndexingUnderAStaleScopeIsDropped() async {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        let accountA = spotlight.currentScope

        await spotlight.signedIn(to: serverA).value
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        await spotlight.index([summary("From account A")], vaultID: vault, scope: accountA).value
        await spotlight.index([summary("No scope")], vaultID: vault, scope: nil).value

        #expect(accountA != nil)
        #expect(await fake.items.isEmpty)
    }

    @Test func listIndexingNeedsASignedInSessionAndRestartsAfterSignOut() async {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        let a = summary("A")

        await spotlight.index([a], vaultID: vault).value
        #expect(await fake.items.isEmpty)

        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        await spotlight.index([a], vaultID: vault).value
        await spotlight.sessionChanged(to: .signedOut).value
        await spotlight.index([a], vaultID: vault).value
        #expect(await fake.items.isEmpty)

        await spotlight.signedIn(to: serverA).value
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        await spotlight.index([a], vaultID: vault).value
        #expect(await fake.documentIDs == [a.id])
    }

    @Test func listIndexingInFlightDuringSignOutDoesNotSurviveIt() async throws {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        await fake.holdNextIndex()

        let indexing = spotlight.index([summary("Late")], vaultID: vault)
        try await until { await fake.heldIndexCalls == 1 }
        let signOut = spotlight.sessionChanged(to: .signedOut)
        await fake.release()
        await indexing.value
        await signOut.value

        #expect(await fake.items.isEmpty)
    }

    @Test func removedDocumentIsIndexedAgainWhenListedAgain() async {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        let restored = summary("Restored")

        await spotlight.index([restored], vaultID: vault).value
        await spotlight.remove(documentID: restored.id).value
        await spotlight.index([restored], vaultID: vault).value

        #expect(await fake.documentIDs == [restored.id])
    }

    @Test func cancelledDonationIsDropped() async {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        await spotlight.sessionChanged(to: .signedIn(serverA)).value

        let donation = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            await spotlight.donate(summary("Closed view"), vaultID: vault)
        }
        await donation.value

        #expect(await fake.items.isEmpty)
    }
}

/// Exercises the real delete, sign-out and server-switch flows against a fake index.
@MainActor struct SpotlightFlowTests {
    @Test func confirmedDeleteRemovesTheDocumentFromSpotlight() async {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        let kept = Document(title: "Keep")
        let gone = Document(title: "Gone")
        await spotlight.donate(summary(kept.title, id: kept.id), vaultID: vault)
        await spotlight.donate(summary(gone.title, id: gone.id), vaultID: vault)
        let service = MockFolioService(documents: [kept, gone])
        let model = DocumentListViewModel(service: service, searchDebounce: .milliseconds(1), spotlight: spotlight)
        await model.load()

        #expect(await model.delete(gone))
        await spotlight.idle()

        #expect(await fake.documentIDs == [kept.id])
    }

    @Test func failedDeleteKeepsTheDocumentInSpotlight() async {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        let doc = Document(title: "Keep")
        await spotlight.donate(summary(doc.title, id: doc.id), vaultID: vault)
        let service = MockFolioService(documents: [doc])
        let model = DocumentListViewModel(service: service, searchDebounce: .milliseconds(1), spotlight: spotlight)
        await model.load()
        await service.setFailure(.unreachable)

        #expect(await !model.delete(doc))
        await spotlight.idle()

        #expect(await fake.documentIDs == [doc.id])
    }

    @Test func deleteConfirmedAfterTheSessionDroppedStillRemovesTheEntry() async {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        let doc = Document(title: "Gone")
        await spotlight.donate(summary(doc.title, id: doc.id), vaultID: vault)
        let service = MockFolioService(documents: [doc])
        let model = DocumentListViewModel(service: service, searchDebounce: .milliseconds(1), spotlight: spotlight)
        await model.load()
        await spotlight.sessionChanged(to: .interrupted(serverA)).value

        #expect(await model.delete(doc))
        await spotlight.idle()

        #expect(await fake.items.isEmpty)
    }

    @Test func documentGoneFromTheServerIsRemovedWhenLookedUp() async {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        let goneElsewhere = summary("Deleted on the web")
        let trashed = Document(title: "Trashed", deletedAt: Date(timeIntervalSince1970: 0))
        await spotlight.donate(goneElsewhere, vaultID: vault)
        await spotlight.donate(summary(trashed.title, id: trashed.id), vaultID: vault)
        let model = DocumentListViewModel(
            service: MockFolioService(documents: [trashed]), searchDebounce: .milliseconds(1), spotlight: spotlight)

        await model.resolveDocument(goneElsewhere.id)
        #expect(model.documentError(for: goneElsewhere.id) != nil)
        await model.resolveDocument(trashed.id)
        await spotlight.idle()

        #expect(await fake.items.isEmpty)
    }

    @Test func unreachableServerKeepsTheEntryOnLookup() async {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        let doc = Document(title: "Kept")
        await spotlight.donate(summary(doc.title, id: doc.id), vaultID: vault)
        let service = MockFolioService(documents: [doc])
        await service.setFailure(.unreachable)
        let model = DocumentListViewModel(service: service, searchDebounce: .milliseconds(1), spotlight: spotlight)

        await model.resolveDocument(doc.id)
        await spotlight.idle()

        #expect(await fake.documentIDs == [doc.id])
    }

    @Test func loadingTheListIndexesItsDocumentsWithNames() async {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        let sender = Correspondent(name: "Stadtwerke")
        let first = Document(title: "Gas bill", correspondentID: sender.id)
        let second = Document(title: "Water bill")
        let service = MockFolioService(correspondents: [sender])
        let listVault = UUID()
        await service.setPages([
            "|": DocumentPage(vaultID: listVault, documents: [first], nextCursor: "next"),
            "|next": DocumentPage(vaultID: listVault, documents: [second]),
        ])
        let model = DocumentListViewModel(service: service, searchDebounce: .milliseconds(1), spotlight: spotlight)

        await model.load()
        await spotlight.idle()
        #expect(await fake.documentIDs == [first.id])
        let item = await fake.items.values.first
        #expect(item?.summary.correspondent == "Stadtwerke")
        #expect(item.flatMap { DocumentLink(spotlightIdentifier: $0.identifier) }?.vaultID == listVault)

        await model.loadNextPage()
        await spotlight.idle()
        #expect(await fake.documentIDs == [first.id, second.id])
    }

    @Test func listFetchedBeforeAnAccountSwitchIsNotIndexedForTheNewAccount() async {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        let service = MockFolioService(documents: [Document(title: "Account A's letter")])
        await service.suspendDocuments()
        let model = DocumentListViewModel(service: service, searchDebounce: .milliseconds(1), spotlight: spotlight)

        let load = Task { await model.load() }
        await service.waitForDocumentRequest()
        await spotlight.sessionChanged(to: .signedOut).value
        await spotlight.signedIn(to: serverA).value
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        await service.releaseDocuments()
        await load.value
        await spotlight.idle()

        #expect(model.rows.count == 1)
        #expect(await fake.items.isEmpty)
    }

    @Test func listIndexingIsDroppedWithoutASession() async {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        let model = DocumentListViewModel(
            service: MockFolioService(documents: [Document(title: "Private")]), searchDebounce: .milliseconds(1),
            spotlight: spotlight)

        await model.load()
        await spotlight.idle()

        #expect(await fake.items.isEmpty)
    }

    @Test func signOutThroughTheSessionStoreClearsSpotlight() async throws {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        let store = InMemoryCredentialStore(credentials(for: serverA))
        let oauth = OAuthSession(store: store)
        await spotlight.sessionChanged(to: SessionRestoration.restore(from: store, server: nil)).value
        await spotlight.donate(summary("Private"), vaultID: vault)
        #expect(await fake.items.count == 1)

        try await oauth.signOut()
        let restored = SessionRestoration.restore(from: store, server: serverA)
        await spotlight.sessionChanged(to: restored).value

        #expect(restored == .signedOut)
        #expect(await fake.items.isEmpty)
        await spotlight.donate(summary("After sign-out"), vaultID: vault)
        #expect(await fake.items.isEmpty)
    }

    @Test func connectingToAnotherServerClearsThePreviousServersEntries() async {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        await spotlight.donate(summary("From A"), vaultID: vault)
        let model = OnboardingViewModel(store: MemoryServerConfigStore(), makeService: { _ in MockFolioService() }) {
            _ = spotlight.signedIn(to: $0)
        }
        model.address = serverB.absoluteString

        await model.connect()
        await spotlight.idle()
        let fromB = summary("From B")
        await spotlight.donate(fromB, vaultID: vault)

        #expect(model.state == .idle)
        #expect(await fake.documentIDs == [fromB.id])
        #expect(await fake.items.values.allSatisfy { $0.domain.hasPrefix(SpotlightIndex.domain(for: serverB)) })
    }

    @Test func signingInAgainAsAnotherAccountOnTheSameServerClearsTheOldOne() async {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        await spotlight.sessionChanged(to: .signedIn(serverA)).value
        await spotlight.donate(summary("Account X"), vaultID: vault)
        await spotlight.sessionChanged(to: .interrupted(serverA)).value
        let model = OnboardingViewModel(store: MemoryServerConfigStore(), makeService: { _ in MockFolioService() }) {
            _ = spotlight.signedIn(to: $0)
        }
        model.address = serverA.absoluteString

        await model.connect()
        await spotlight.idle()

        #expect(model.state == .idle)
        #expect(await fake.items.isEmpty)
    }

    @Test func donationRacingSignOutThroughTheSessionStoreStaysCleared() async throws {
        let fake = FakeSpotlightIndex()
        let spotlight = SpotlightIndex(index: fake, defaults: scopeDefaults())
        let store = InMemoryCredentialStore(credentials(for: serverA))
        let oauth = OAuthSession(store: store)
        await spotlight.sessionChanged(to: SessionRestoration.restore(from: store, server: nil)).value
        await fake.holdNextIndex()

        let donation = Task { await spotlight.donate(summary("Opened just before sign-out"), vaultID: vault) }
        try await until { await fake.heldIndexCalls == 1 }
        try await oauth.signOut()
        let signedOut = spotlight.sessionChanged(to: SessionRestoration.restore(from: store, server: serverA))
        await fake.release()
        await donation.value
        await signedOut.value

        #expect(await fake.items.isEmpty)
    }

    private func credentials(for server: URL) -> OAuthCredentials {
        OAuthCredentials(
            serverURL: server, clientID: "client", tokenEndpoint: server.appending(path: "oauth/token"),
            revocationEndpoint: nil,
            tokens: OAuthTokens(accessToken: "access", refreshToken: nil, expiresAt: .distantFuture)
        )
    }
}
