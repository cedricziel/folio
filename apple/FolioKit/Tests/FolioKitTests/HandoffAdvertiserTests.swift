import Foundation
import Testing

@testable import FolioKit

private let server = URL(string: "https://folio.example.com")!
private let vault = UUID()
private let document = UUID()
private let savedView = UUID()

private func state(
    _ session: SessionRestoration = .signedIn(server), isLocked: Bool = false, list: AdvertisedList = .library(.inbox),
    document: UUID? = nil, discussion: Bool = false
) -> AdvertisingState {
    AdvertisingState(
        session: session, isLocked: isLocked, vaultID: vault, list: list, documentID: document,
        showingDiscussion: discussion)
}

struct HandoffAdvertiserTests {
    @Test func aSelectedDocumentWinsOverTheList() {
        let payload = HandoffAdvertiser.payload(for: state(list: .savedView(savedView), document: document))
        #expect(payload == HandoffPayload(server: server, vaultID: vault, target: .document(document)))
    }

    @Test func thePaneFollowsTheDiscussion() {
        #expect(
            HandoffAdvertiser.payload(for: state(document: document, discussion: true))?.target
                == .document(document, pane: .discussion))
        #expect(
            HandoffAdvertiser.payload(for: state(document: document, discussion: false))?.target == .document(document))
    }

    @Test(arguments: [
        AdvertisedList.library(.inbox), .library(.all), .savedView(savedView),
    ])
    func withoutASelectionTheListIsAdvertised(_ list: AdvertisedList) {
        let target = HandoffAdvertiser.payload(for: state(list: list))?.target
        switch list {
        case let .library(kind): #expect(target == .library(kind))
        case let .savedView(id): #expect(target == .savedView(id))
        }
    }

    @Test(arguments: [
        SessionRestoration.restoring, .signedOut, .expired(server), .interrupted(server),
        .unavailable(server: server, message: "x"),
    ])
    func nothingIsAdvertisedWithoutASignedInSession(_ session: SessionRestoration) {
        #expect(HandoffAdvertiser.payload(for: state(session, document: document)) == nil)
    }

    @Test func nothingIsAdvertisedWhileLocked() {
        #expect(HandoffAdvertiser.payload(for: state(isLocked: true, document: document)) == nil)
        #expect(HandoffAdvertiser.payload(for: state(isLocked: true)) == nil)
    }

    @Test func nothingIsAdvertisedWithoutAVault() {
        var noVault = state()
        noVault.vaultID = nil
        #expect(HandoffAdvertiser.payload(for: noVault) == nil)
    }

    @Test func theReportedPageTravelsWithTheDocument() {
        var paged = state(document: document)
        paged.page = 4
        #expect(HandoffAdvertiser.payload(for: paged)?.target == .document(document, page: 4))
        #expect(HandoffAdvertiser.payload(for: state(document: document))?.target == .document(document, page: nil))
        #expect(HandoffAdvertiser.payload(for: state(document: document))?.userInfo["page"] == nil)
    }

    @Test func aSplitViewAdvertisesOnlyTheSelectedDocument() {
        let selected = state(list: .library(.all), document: document)
        #expect(HandoffAdvertiser.listPayload(for: selected) == nil)
        #expect(HandoffAdvertiser.payload(for: selected)?.target == .document(document))

        let cleared = state(list: .library(.all))
        #expect(HandoffAdvertiser.listPayload(for: cleared)?.target == .library(.all))
        #expect(HandoffAdvertiser.listPayload(for: state(isLocked: true)) == nil)
    }

    @Test func searchTextIsNotAnInput() {
        let labels = Mirror(reflecting: state()).children.compactMap(\.label)
        #expect(labels == ["session", "isLocked", "vaultID", "list", "documentID", "showingDiscussion", "page"])
    }
}
