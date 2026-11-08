import Foundation
import Testing

@testable import FolioKit

private let serverA = URL(string: "https://a.folio.example")!
private let serverB = URL(string: "https://b.folio.example:8443/folio")!
private let vault = UUID()
private let route = HandoffRoute(vaultID: vault, target: .document(UUID(), pane: .discussion))

private func web(_ target: HandoffTarget) -> URL {
    HandoffLink.webURL(for: HandoffPayload(server: serverA, vaultID: vault, target: target))!
}

struct HandoffPresentationTests {
    @Test func holdAndIgnoreShowNothing() {
        #expect(HandoffPresentation.from(.hold, currentServer: serverB) == .nothing)
        #expect(HandoffPresentation.from(.ignore, currentServer: nil) == .nothing)
    }

    @Test func openNavigates() {
        #expect(HandoffPresentation.from(.open(route), currentServer: serverA) == .navigate(route))
    }

    @Test func signInPrefillsTheSendersServer() {
        #expect(
            HandoffPresentation.from(.signIn(prefill: serverA, then: route), currentServer: nil)
                == .prefillSignIn(serverA))
    }

    @Test func unreadableContinuationsOpenTheBrowser() {
        let page = URL(string: "https://a.folio.example/login")!
        #expect(HandoffPresentation.from(.openInBrowser(page), currentServer: serverA) == .browser(page))
    }

    @Test func anotherServerExplainsBothHosts() {
        let page = web(.document(UUID()))
        let presentation = HandoffPresentation.from(
            .otherServer(host: "a.folio.example", web: page), currentServer: serverB)
        #expect(
            presentation
                == .alert(
                    title: "On another server",
                    message: "This document is on a.folio.example. You're signed in to b.folio.example:8443.",
                    browserURL: page))
    }

    @Test(arguments: [
        (HandoffTarget.library(.all), "This vault is on a.folio.example."),
        (.savedView(UUID()), "This saved view is on a.folio.example."),
    ])
    func anotherServerNamesWhatWasHandedOver(_ target: HandoffTarget, _ message: String) {
        let presentation = HandoffPresentation.from(
            .otherServer(host: "a.folio.example", web: web(target)), currentServer: nil)
        guard case let .alert(_, text, _) = presentation else {
            Issue.record("expected an alert, got \(presentation)")
            return
        }
        #expect(text == message)
    }

    @Test func onlyTheOtherServerAlertOffersButtons() {
        let page = web(.document(UUID()))
        let alert = HandoffPresentation.from(.otherServer(host: "a.folio.example", web: page), currentServer: serverB)
        #expect(alert.actions == [.openInBrowser(page), .cancel])
        #expect(alert.actions.map(\.title) == ["Open in Browser", "Cancel"])

        let others: [HandoffPresentation] = [
            .navigate(route), .browser(page), .prefillSignIn(serverA), .nothing,
        ]
        for presentation in others {
            #expect(presentation.actions.isEmpty, "\(presentation)")
        }
    }
}
