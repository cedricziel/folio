import Foundation
import Testing

@testable import FolioKit

private let stadtwerke = Correspondent(name: "Stadtwerke Köln")
private let invoice = DocumentType(name: "Invoice")

private func document(
    _ title: String, content: String? = "text", correspondent: Correspondent? = nil,
    type: DocumentType? = nil, pages: Int? = 3
) -> Document {
    Document(
        title: title, content: content, correspondentID: correspondent?.id,
        documentTypeID: type?.id, pageCount: pages,
        createdAt: Date(timeIntervalSince1970: 1_741_950_000)
    )
}

struct DocumentStatusTests {
    @Test func documentWithoutTextIsStillBeingRead() {
        #expect(document("a", content: nil, pages: 3).status == .reading(pages: 3))
        #expect(document("a", content: "  \n", pages: nil).status == .reading(pages: nil))
    }

    @Test func readDocumentWithoutCorrespondentOrTypeNeedsAGlance() {
        #expect(document("a").status == .needsReview)
    }

    @Test func documentWithAnyClassificationIsFiled() {
        #expect(document("a", correspondent: stadtwerke).status == .filed)
        #expect(document("a", type: invoice).status == .filed)
    }

    @Test func tagTintIsStablePerName() {
        #expect(TagTint.tint(forTagNamed: "Insurance") == TagTint.tint(forTagNamed: "insurance"))
    }
}

@MainActor struct DocumentListViewModelTests {
    private func makeModel(
        _ service: MockFolioService, debounce: Duration = .milliseconds(1)
    ) -> DocumentListViewModel {
        DocumentListViewModel(service: service, searchDebounce: debounce)
    }

    @Test func inboxScopeKeepsOnlyDocumentsWaitingForALook() async {
        let service = MockFolioService(
            documents: [
                document("Unfiled"), document("Filed", correspondent: stadtwerke),
                document("Reading", content: nil),
            ], correspondents: [stadtwerke])
        let model = makeModel(service)
        await model.load()
        #expect(model.rows.count == 3)

        model.scope = .inbox
        #expect(model.rows.map(\.title) == ["Unfiled", "Reading"])

        model.scope = .all
        #expect(model.rows.count == 3)
    }

    @Test func loadFillsRowsWithCorrespondentTypeAndStatus() async {
        let service = MockFolioService(
            documents: [
                document("Jahresabrechnung", correspondent: stadtwerke, type: invoice),
                document("Scan", content: nil, pages: 3),
                document("Letter"),
            ],
            correspondents: [stadtwerke], types: [invoice]
        )
        let model = makeModel(service)
        await model.load()

        #expect(model.phase == .loaded)
        let rows = model.rows
        #expect(rows[0].subtitle == "Stadtwerke Köln · Invoice")
        #expect(rows[0].statusText == nil)
        #expect(rows[1].statusText == "Reading 3 pages…")
        #expect(rows[2].statusText == "Needs a look")
        #expect(rows[2].subtitle == nil)
        #expect(rows[0].dateText.count == 10)
    }

    @Test func singlePageReadingUsesSingular() async {
        let model = makeModel(MockFolioService(documents: [document("x", content: nil, pages: 1)]))
        await model.load()
        #expect(model.rows[0].statusText == "Reading 1 page…")
    }

    @Test func failedLoadKeepsAFriendlyMessage() async {
        let service = MockFolioService()
        await service.setFailure(.unreachable)
        let model = makeModel(service)
        await model.load()
        #expect(model.phase == .failed("Couldn't reach the server."))
    }

    @Test func lookupFailureDoesNotBlockTheList() async {
        let service = MockFolioService(documents: [document("Letter")])
        await service.setFailLookups(true)
        let model = makeModel(service)
        await model.load()
        #expect(model.phase == .loaded)
        #expect(model.rows.count == 1)
    }

    @Test func searchQueriesTheServerAfterDebounce() async {
        let service = MockFolioService(documents: [document("Strom")])
        let model = makeModel(service)
        await model.search("  strom ")
        #expect(await service.queries == ["strom"])
        #expect(model.rows.count == 1)
    }

    @Test func supersededSearchNeverReachesTheServer() async {
        let service = MockFolioService(documents: [document("Strom")])
        let model = makeModel(service, debounce: .milliseconds(200))
        let first = Task { await model.search("st") }
        try? await Task.sleep(for: .milliseconds(20))
        first.cancel()
        await model.search("strom")
        await first.value
        #expect(await service.queries == ["strom"])
    }

    @Test func deleteRemovesTheRowAfterTheServerConfirms() async {
        let kept = document("Keep")
        let gone = document("Gone")
        let service = MockFolioService(documents: [kept, gone])
        let model = makeModel(service)
        await model.load()
        await model.delete(gone)
        #expect(model.rows.map(\.title) == ["Keep"])
        #expect(await service.deleted == [gone.id])
    }
}

@MainActor struct DocumentListFailureTests {
    @Test func failedDeleteKeepsTheListAndReportsTransiently() async {
        let doc = Document(title: "Keep", content: "text")
        let service = MockFolioService(documents: [doc])
        let model = DocumentListViewModel(service: service, searchDebounce: .milliseconds(1))
        await model.load()
        await service.setFailure(.unreachable)
        await model.delete(doc)
        #expect(model.phase == .loaded)
        #expect(model.rows.count == 1)
        #expect(model.actionError == "Couldn't reach the server.")
        model.dismissActionError()
        #expect(model.actionError == nil)
    }

    @Test func lookupsAreRetriedAfterAFailure() async {
        let person = Correspondent(name: "Stadtwerke")
        let service = MockFolioService(documents: [Document(title: "A")], correspondents: [person])
        await service.setFailLookups(true)
        let model = DocumentListViewModel(service: service, searchDebounce: .milliseconds(1))
        await model.search("")
        #expect(model.correspondents.isEmpty)
        await service.setFailLookups(false)
        await model.search("")
        #expect(model.correspondents[person.id] == "Stadtwerke")
    }
}

@MainActor struct DocumentDetailViewModelTests {
    @Test func metadataListsRecordsInMono() {
        let model = DocumentDetailViewModel(
            document: Document(
                title: "Bill", originalFilename: "bill.pdf", pageCount: 2,
                createdAt: Date(timeIntervalSince1970: 1_741_950_000)
            ),
            correspondent: "Stadtwerke Köln", type: "Invoice", service: MockFolioService()
        )
        let byLabel = Dictionary(uniqueKeysWithValues: model.metadata.map { ($0.label, $0) })
        #expect(byLabel["From"]?.value == "Stadtwerke Köln")
        #expect(byLabel["From"]?.isRecord == false)
        #expect(byLabel["Pages"]?.value == "2")
        #expect(byLabel["Pages"]?.isRecord == true)
        #expect(byLabel["File"]?.value == "bill.pdf")
    }

    @Test func missingClassificationReadsAsNotSet() {
        let model = DocumentDetailViewModel(
            document: Document(title: "Bill"), correspondent: nil, type: nil,
            service: MockFolioService()
        )
        #expect(model.metadata.first { $0.label == "From" }?.value == "Not set")
    }

    @Test func previewLoadsTheDownloadedFile() async {
        let doc = Document(title: "Bill", originalFilename: "bill.pdf")
        let model = DocumentDetailViewModel(
            document: doc, correspondent: nil, type: nil, service: MockFolioService()
        )
        await model.loadPreview()
        #expect(model.preview == .ready(URL(filePath: "/tmp/bill.pdf")))
    }

    @Test func previewFailureIsReportedInWords() async {
        let service = MockFolioService()
        await service.setFailure(.server(status: 404, message: "Document has no file"))
        let model = DocumentDetailViewModel(
            document: Document(title: "Bill"), correspondent: nil, type: nil, service: service
        )
        await model.loadPreview()
        #expect(model.preview == .failed("Document has no file"))
    }
}

@MainActor struct UploadViewModelTests {
    private func file(_ name: String) -> UploadFile {
        UploadFile(data: Data(name.utf8), filename: name, mimeType: "application/pdf")
    }

    @Test func uploadsEveryFileAndReportsCount() async {
        let service = MockFolioService()
        var notified = 0
        let model = UploadViewModel(service: service) { notified += 1 }
        await model.upload([file("a.pdf"), file("b.pdf")])
        #expect(model.state == .finished(count: 2))
        #expect(await service.uploads.count == 2)
        #expect(notified == 1)
    }

    @Test func stopsAtFirstFailureAndSaysHowFarItGot() async {
        let service = MockFolioService()
        await service.setFailUploadsAfter(1)
        let model = UploadViewModel(service: service) {}
        await model.upload([file("a.pdf"), file("b.pdf"), file("c.pdf")])
        #expect(model.state == .failed(message: "Couldn't reach the server.", uploaded: 1))
    }

    @Test func aSecondBatchWhileOneIsRunningIsIgnored() async {
        let service = MockFolioService()
        await service.setUploadDelay(.milliseconds(50))
        let model = UploadViewModel(service: service) {}
        let first = Task { await model.upload([file("a.pdf"), file("b.pdf")]) }
        while model.state == .idle { await Task.yield() }
        await model.upload([file("c.pdf")])
        await first.value
        #expect(await service.uploads.map(\.filename).contains("c.pdf") == false)
    }

    @Test func emptySelectionDoesNothing() async {
        let model = UploadViewModel(service: MockFolioService()) {}
        await model.upload([])
        #expect(model.state == .idle)
    }
}

@MainActor struct OnboardingViewModelTests {
    @Test(arguments: [
        ("folio.local:3000", "https://folio.local:3000"),
        ("http://192.168.1.5:3000/", "http://192.168.1.5:3000"),
        ("  https://docs.example.com/folio/ ", "https://docs.example.com/folio"),
    ])
    func normalizesAddresses(input: String, expected: String) throws {
        #expect(try ServerAddress.parse(input).absoluteString == expected)
    }

    @Test(arguments: ["", "   ", "ftp://example.com", "http://"])
    func rejectsUnusableAddresses(input: String) {
        #expect(throws: FolioError.invalidServerURL) { try ServerAddress.parse(input) }
    }

    @Test func connectChecksTheServerThenPersistsTheURL() async {
        let store = MemoryServerConfigStore()
        let service = MockFolioService()
        var connected: URL?
        let model = OnboardingViewModel(store: store, makeService: { _ in service }) {
            connected = $0
        }
        model.address = "folio.local:3000"
        await model.connect()
        #expect(await service.pings == 1)
        #expect(store.url?.absoluteString == "https://folio.local:3000")
        #expect(connected == store.url)
        #expect(model.state == .idle)
    }

    @Test func signsInBeforeCheckingTheServer() async {
        let service = MockFolioService()
        var events: [String] = []
        let model = OnboardingViewModel(
            store: MemoryServerConfigStore(), makeService: { _ in service },
            signIn: { events.append("sign-in \($0.absoluteString)") }
        ) { _ in }
        model.address = "folio.local"
        await model.connect()
        #expect(events == ["sign-in https://folio.local"])
        #expect(await service.pings == 1)
    }

    @Test func aFailedSignInIsNotPersistedAndSkipsThePing() async {
        let store = MemoryServerConfigStore()
        let service = MockFolioService()
        let model = OnboardingViewModel(
            store: store, makeService: { _ in service },
            signIn: { _ in throw FolioError.signInFailed("Sign-in was cancelled or denied.") }
        ) { _ in }
        model.address = "folio.local"
        await model.connect()
        #expect(store.url == nil)
        #expect(await service.pings == 0)
        #expect(model.state == .failed("Sign-in was cancelled or denied."))
    }

    @Test func unreachableServerIsNotPersisted() async {
        let store = MemoryServerConfigStore()
        let service = MockFolioService()
        await service.setFailure(.unreachable)
        let model = OnboardingViewModel(store: store, makeService: { _ in service }) { _ in }
        model.address = "folio.local"
        await model.connect()
        #expect(store.url == nil)
        #expect(model.state == .failed("Couldn't reach the server."))
    }

    @Test func invalidAddressFailsBeforeAnyRequest() async {
        let service = MockFolioService()
        let model = OnboardingViewModel(
            store: MemoryServerConfigStore(), makeService: { _ in service }
        ) { _ in }
        model.address = "ftp://nope"
        await model.connect()
        #expect(await service.pings == 0)
        #expect(model.state == .failed("That doesn't look like a server address."))
    }

    @Test func userDefaultsStoreRoundTrips() throws {
        let suite = "folio-test-\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = UserDefaultsServerConfigStore(defaults: defaults)
        #expect(store.load() == nil)
        try store.save(#require(URL(string: "https://folio.local")))
        #expect(store.load()?.absoluteString == "https://folio.local")
        store.save(nil)
        #expect(store.load() == nil)
    }
}
