import Foundation
import Testing

@testable import FolioKit

private let serverA = URL(string: "https://a.folio.example")!
private let serverASpelledOtherwise = URL(string: "https://A.folio.example:443/")!
private let serverB = URL(string: "https://b.folio.example")!
private let vault = UUID()
private let document = UUID()
private let payload = HandoffPayload(server: serverA, vaultID: vault, target: .document(document, pane: .discussion))
private let route = HandoffRoute(vaultID: vault, target: .document(document, pane: .discussion))
private let web = URL(
    string:
        "https://a.folio.example/vaults/\(vault.uuidString.lowercased())/documents/\(document.uuidString.lowercased())")!
private let elsewhere = HandoffDecision.otherServer(host: "a.folio.example", web: web)
private let spotlight = HandoffIncoming.spotlight(DocumentLink(documentID: document, vaultID: vault))
private let spotlightRoute = HandoffRoute(vaultID: vault, target: .document(document))

private func decide(_ incoming: HandoffIncoming, _ session: SessionRestoration) -> HandoffDecision {
    HandoffResolver.decide(incoming: incoming, session: session)
}

private func decide(_ session: SessionRestoration) -> HandoffDecision {
    decide(.payload(payload), session)
}

struct HandoffResolverTests {
    @Test func restoringHoldsEveryContinuation() {
        #expect(decide(.restoring) == .hold)
        #expect(decide(spotlight, .restoring) == .hold)
        #expect(decide(.universalLink(web), .restoring) == .hold)
    }

    @Test func signedOutOffersSignInButIgnoresSpotlight() {
        #expect(decide(.signedOut) == .signIn(prefill: serverA, then: route))
        #expect(decide(spotlight, .signedOut) == .ignore)
    }

    @Test func signedInToTheSameServerOpensTheTarget() {
        #expect(decide(.signedIn(serverA)) == .open(route))
        #expect(decide(.signedIn(serverASpelledOtherwise)) == .open(route))
        #expect(decide(spotlight, .signedIn(serverB)) == .open(spotlightRoute))
    }

    @Test func signedInToAnotherServerOffersTheBrowser() {
        #expect(decide(.signedIn(serverB)) == elsewhere)
    }

    @Test(arguments: [SessionRestoration.expired(serverA), .interrupted(serverA)])
    func reconnectingToTheSameServerHolds(_ session: SessionRestoration) {
        #expect(decide(session) == .hold)
        #expect(decide(spotlight, session) == .hold)
    }

    @Test(arguments: [SessionRestoration.expired(serverB), .interrupted(serverB)])
    func reconnectingToAnotherServerOffersTheBrowser(_ session: SessionRestoration) {
        #expect(decide(session) == elsewhere)
    }

    @Test func unavailableHoldsUnlessItNamesAnotherServer() {
        #expect(decide(.unavailable(server: serverA, message: "keychain")) == .hold)
        #expect(decide(.unavailable(server: nil, message: "keychain")) == .hold)
        #expect(decide(spotlight, .unavailable(server: nil, message: "keychain")) == .hold)
        #expect(decide(.unavailable(server: serverB, message: "keychain")) == elsewhere)
    }

    @Test(arguments: [
        SessionRestoration.restoring, .signedOut, .signedIn(serverA), .expired(serverB),
        .unavailable(server: nil, message: "x"),
    ])
    func unreadableContinuationsOpenTheirWebPage(_ session: SessionRestoration) {
        let malformed = HandoffIncoming.activity(
            type: HandoffActivityType.document, userInfo: ["version": 2], webpageURL: web)
        #expect(malformed == .unreadable(webpageURL: web))
        #expect(decide(malformed, session) == .openInBrowser(web))

        let login = URL(string: "https://a.folio.example/login")!
        #expect(decide(.universalLink(login), session) == .openInBrowser(login))

        #expect(decide(.unreadable(webpageURL: nil), session) == .ignore)
    }

    @Test func universalLinksRouteLikeHandoff() {
        #expect(
            decide(.universalLink(web), .signedIn(serverA))
                == .open(HandoffRoute(vaultID: vault, target: .document(document))))
        #expect(decide(.universalLink(web), .signedIn(serverB)) == elsewhere)
    }
}
