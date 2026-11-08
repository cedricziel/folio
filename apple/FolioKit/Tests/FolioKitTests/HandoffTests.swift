import Foundation
import Testing

@testable import FolioKit

private let serverA = URL(string: "https://a.folio.example")!
private let serverB = URL(string: "https://b.folio.example")!
private let vault = UUID(uuidString: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee")!
private let documentID = UUID(uuidString: "3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f")!

private func activity(from server: URL = serverA) -> [AnyHashable: Any] {
    DocumentActivityPayload(server: server, vaultID: vault, documentID: documentID).userInfo
}

@MainActor private func makeInbox() -> (DocumentHandoff, IntentRouter) {
    let router = IntentRouter()
    return (DocumentHandoff(router: router), router)
}

@MainActor struct DocumentHandoffTests {
    @Test func continuingOnTheSameServerOpensTheDocumentInItsVault() {
        let (handoff, router) = makeInbox()

        handoff.receive(activity(), session: .signedIn(serverA))

        #expect(router.request == .document(documentID, vaultID: vault))
        #expect(handoff.problem == nil)
        #expect(handoff.pending == nil)
    }

    @Test func serverAddressesMatchRegardlessOfTrailingSlashOrCase() {
        let (handoff, router) = makeInbox()

        handoff.receive(activity(from: URL(string: "https://A.folio.example/")!), session: .signedIn(serverA))

        #expect(router.request == .document(documentID, vaultID: vault))
    }

    @Test func activityFromAnotherServerIsRejected() {
        let (handoff, router) = makeInbox()

        handoff.receive(activity(from: serverB), session: .signedIn(serverA))

        #expect(router.request == nil)
        #expect(handoff.pending == nil)
        #expect(handoff.problem?.contains("b.folio.example") == true)
    }

    @Test func malformedOrNewerActivitiesAreRejected() {
        let (handoff, router) = makeInbox()
        var future = activity()
        future["version"] = DocumentActivityPayload.version + 1

        handoff.receive(future, session: .signedIn(serverA))
        #expect(router.request == nil)
        #expect(handoff.problem != nil)

        handoff.dismissProblem()
        handoff.receive(["document": documentID.uuidString], session: .signedIn(serverA))
        #expect(router.request == nil)
        #expect(handoff.problem != nil)
    }

    @Test func coldLaunchWaitsForTheSessionToRestore() {
        let (handoff, router) = makeInbox()

        handoff.receive(activity(), session: .restoring)
        #expect(router.request == nil)
        #expect(handoff.problem == nil)

        handoff.sessionChanged(to: .signedIn(serverA))
        #expect(router.request == .document(documentID, vaultID: vault))
        #expect(handoff.pending == nil)
    }

    @Test(arguments: [
        SessionRestoration.signedOut, .expired(serverA), .interrupted(serverA),
        .unavailable(server: serverA, message: "keychain"),
    ])
    func signedOutAsksToSignInAndOpensAfterwards(_ session: SessionRestoration) {
        let (handoff, router) = makeInbox()

        handoff.receive(activity(), session: session)
        #expect(router.request == nil)
        #expect(handoff.problem?.contains("a.folio.example") == true)

        handoff.sessionChanged(to: .signedIn(serverA))
        #expect(router.request == .document(documentID, vaultID: vault))
        #expect(handoff.problem == nil)
    }

    @Test func signingInToAnotherServerDropsThePendingDocument() {
        let (handoff, router) = makeInbox()
        handoff.receive(activity(), session: .signedOut)

        handoff.sessionChanged(to: .signedIn(serverB))

        #expect(router.request == nil)
        #expect(handoff.pending == nil)
        #expect(handoff.problem?.contains("a.folio.example") == true)
    }

    @Test func signOutBeforeTheWorkspaceOpensTheDocumentWithdrawsTheRequest() {
        let (handoff, router) = makeInbox()
        handoff.receive(activity(), session: .signedIn(serverA))
        #expect(router.request != nil)

        handoff.sessionChanged(to: .signedOut)
        handoff.sessionChanged(to: .signedIn(serverB))

        #expect(router.request == nil)
    }

    @Test func signOutLeavesOtherRequestsAlone() {
        let (handoff, router) = makeInbox()
        router.route(.scan)

        handoff.sessionChanged(to: .signedOut)

        #expect(router.request == .scan)
    }

    @Test func aNewerActivityReplacesAPendingOne() {
        let (handoff, router) = makeInbox()
        handoff.receive(activity(from: serverB), session: .restoring)
        handoff.receive(activity(), session: .restoring)

        handoff.sessionChanged(to: .signedIn(serverA))

        #expect(router.request == .document(documentID, vaultID: vault))
    }
}

@MainActor struct DocumentActivityHandoffTests {
    @Test func openDocumentIsOfferedForHandoffButNotForSearch() {
        let tracker = DocumentActivityTracker(locator: DocumentLocator())
        tracker.sessionChanged(to: .signedIn(serverA))
        tracker.show(DocumentSummary(id: documentID, title: "Lease", date: .now), vaultID: vault)
        let activity = NSUserActivity(activityType: DocumentActivityPayload.activityType)

        tracker.configure(activity)
        #expect(activity.isEligibleForHandoff)
        #expect(!activity.isEligibleForSearch)
        #expect(!activity.isEligibleForPublicIndexing)

        tracker.hide(documentID: documentID)
        tracker.configure(activity)
        #expect(!activity.isEligibleForHandoff)
    }
}
