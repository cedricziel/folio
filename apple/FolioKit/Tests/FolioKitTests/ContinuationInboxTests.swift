import Foundation
import Testing

@testable import FolioKit

private let serverA = URL(string: "https://a.folio.example")!
private let serverB = URL(string: "https://b.folio.example")!
private let vault = UUID()
private let firstWindow = UUID()
private let secondWindow = UUID()

private func handoff(_ document: UUID = UUID(), server: URL = serverA) -> HandoffIncoming {
    .payload(HandoffPayload(server: server, vaultID: vault, target: .document(document)))
}

private func opening(_ document: UUID) -> HandoffDecision {
    .open(HandoffRoute(vaultID: vault, target: .document(document)))
}

@MainActor struct ContinuationInboxTests {
    @Test func lockedOrHeldItemsStayPendingUntilTheSessionAllows() {
        let inbox = ContinuationInbox()
        let document = UUID()
        inbox.receive(handoff(document), scene: firstWindow)

        #expect(inbox.take(scene: firstWindow, isLocked: true, session: .signedIn(serverA)) == nil)
        #expect(inbox.take(scene: firstWindow, isLocked: false, session: .restoring) == nil)
        #expect(inbox.take(scene: firstWindow, isLocked: false, session: .expired(serverA)) == nil)
        #expect(inbox.pending != nil)

        #expect(inbox.take(scene: firstWindow, isLocked: false, session: .signedIn(serverA)) == opening(document))
        #expect(inbox.pending == nil)
    }

    @Test func theLastContinuationWins() {
        let inbox = ContinuationInbox()
        let later = UUID()
        inbox.receive(handoff(), scene: firstWindow)
        inbox.receive(handoff(later), scene: firstWindow)

        #expect(inbox.take(scene: firstWindow, isLocked: false, session: .signedIn(serverA)) == opening(later))
        #expect(inbox.take(scene: firstWindow, isLocked: false, session: .signedIn(serverA)) == nil)
    }

    @Test func onlyTheReceivingWindowTakesItWhileItIsOpen() {
        let inbox = ContinuationInbox()
        let document = UUID()
        inbox.receive(handoff(document), scene: firstWindow)

        #expect(inbox.take(scene: secondWindow, isLocked: false, session: .signedIn(serverA)) == nil)
        inbox.sceneClosed(secondWindow)
        #expect(inbox.take(scene: secondWindow, isLocked: false, session: .signedIn(serverA)) == nil)

        inbox.sceneClosed(firstWindow)
        #expect(inbox.take(scene: secondWindow, isLocked: false, session: .signedIn(serverA)) == opening(document))
        #expect(inbox.take(scene: firstWindow, isLocked: false, session: .signedIn(serverA)) == nil)
    }

    @Test func aWindowTakesOnlyTheDecisionsItHandles() {
        let inbox = ContinuationInbox()
        inbox.receive(handoff(server: serverB), scene: firstWindow)

        #expect(inbox.take(scene: firstWindow, isLocked: false, session: .signedIn(serverA)) { $0.route != nil } == nil)
        #expect(inbox.pending != nil)
        guard case .otherServer = inbox.take(scene: firstWindow, isLocked: false, session: .signedIn(serverA)) else {
            Issue.record("expected the other-server decision")
            return
        }
    }

    @Test func clearEmptiesTheInbox() {
        let inbox = ContinuationInbox()
        inbox.receive(handoff(), scene: firstWindow)
        inbox.clear()
        #expect(inbox.pending == nil)
        #expect(inbox.decision(isLocked: false, session: .signedIn(serverA)) == nil)
    }

    @Test func aHeldItemAsksForSignInWhenRestorationFindsNoCredentials() {
        let inbox = ContinuationInbox()
        let document = UUID()
        inbox.receive(handoff(document), scene: firstWindow)
        #expect(inbox.take(scene: firstWindow, isLocked: false, session: .restoring) == nil)

        #expect(inbox.take(scene: firstWindow, isLocked: false, session: .signedOut) == nil)
        #expect(
            inbox.decision(isLocked: false, session: .signedOut)
                == .signIn(prefill: serverA, then: HandoffRoute(vaultID: vault, target: .document(document))))
        #expect(inbox.decision(isLocked: true, session: .signedOut) == nil)
        #expect(inbox.pending != nil)
    }

    @Test func signingInToAnotherServerDropsTheContinuation() {
        let inbox = ContinuationInbox()
        inbox.receive(handoff(), scene: firstWindow)
        inbox.signedIn(to: serverB)
        #expect(inbox.pending == nil)

        let document = UUID()
        inbox.receive(handoff(document), scene: firstWindow)
        inbox.signedIn(to: URL(string: "https://A.folio.example/")!)
        #expect(inbox.take(scene: firstWindow, isLocked: false, session: .signedIn(serverA)) == opening(document))
    }
}
