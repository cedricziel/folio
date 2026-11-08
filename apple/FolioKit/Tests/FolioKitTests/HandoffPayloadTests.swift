import Foundation
import Testing

@testable import FolioKit

private let server = URL(string: "https://folio.example.com/folio")!
private let vault = UUID(uuidString: "AAAAAAAA-BBBB-4CCC-8DDD-EEEEEEEEEEEE")!
private let document = UUID(uuidString: "11111111-2222-4333-8444-555555555555")!
private let savedView = UUID(uuidString: "66666666-7777-4888-8999-AAAAAAAAAAAA")!

private let payloads: [HandoffPayload] = [
    HandoffPayload(server: server, vaultID: vault, target: .document(document)),
    HandoffPayload(server: server, vaultID: vault, target: .document(document, pane: .discussion)),
    HandoffPayload(server: server, vaultID: vault, target: .document(document, pane: .discussion, page: 7)),
    HandoffPayload(server: server, vaultID: vault, target: .library(.inbox)),
    HandoffPayload(server: server, vaultID: vault, target: .library(.all)),
    HandoffPayload(server: server, vaultID: vault, target: .savedView(savedView)),
]

private func keys(_ info: [AnyHashable: Any]) -> Set<String> {
    Set(info.keys.compactMap { $0 as? String })
}

struct HandoffPayloadTests {
    @Test(arguments: payloads)
    func roundTripsThroughUserInfo(_ payload: HandoffPayload) {
        #expect(HandoffPayload(activityType: payload.activityType, userInfo: payload.userInfo) == payload)
    }

    @Test func activityTypesAreFixed() {
        #expect(payloads.map(\.activityType) == [
            "com.cedricziel.folio.document", "com.cedricziel.folio.document", "com.cedricziel.folio.document",
            "com.cedricziel.folio.library",
            "com.cedricziel.folio.library", "com.cedricziel.folio.saved-view",
        ])
        #expect(HandoffActivityType.all.count == 3)
    }

    @Test func readsAndWritesTheOnScreenDocumentActivity() {
        let onScreen = DocumentActivityPayload(server: server, vaultID: vault, documentID: document)

        let handoff = HandoffPayload(activityType: DocumentActivityPayload.activityType, userInfo: onScreen.userInfo)
        #expect(handoff == HandoffPayload(onScreen))
        #expect(handoff?.target == .document(document))

        let discussion = HandoffPayload(server: server, vaultID: vault, target: .document(document, pane: .discussion))
        #expect(DocumentActivityPayload(userInfo: discussion.userInfo) == onScreen)
        #expect(discussion.documentActivity == onScreen)
    }

    @Test func requiredKeysNameTheTarget() {
        #expect(payloads[0].requiredUserInfoKeys == DocumentActivityPayload.requiredKeys)
        #expect(payloads[2].requiredUserInfoKeys == DocumentActivityPayload.requiredKeys)
        #expect(payloads[3].requiredUserInfoKeys == ["version", "server", "vault", "list"])
        #expect(payloads[5].requiredUserInfoKeys == ["version", "server", "vault", "savedView"])
        for payload in payloads {
            #expect(payload.requiredUserInfoKeys.isSubset(of: keys(payload.userInfo)))
        }
    }

    @Test func userInfoHoldsOnlyIdentifiers() {
        #expect(keys(payloads[0].userInfo) == ["version", "server", "vault", "document"])
        #expect(keys(payloads[1].userInfo) == ["version", "server", "vault", "document", "pane"])
        #expect(keys(payloads[2].userInfo) == ["version", "server", "vault", "document", "pane", "page"])
        #expect(keys(payloads[3].userInfo) == ["version", "server", "vault", "list"])
        #expect(keys(payloads[5].userInfo) == ["version", "server", "vault", "savedView"])
        #expect(payloads[0].userInfo["vault"] as? String == vault.uuidString.lowercased())
        #expect(payloads[0].userInfo["document"] as? String == document.uuidString.lowercased())
        #expect(payloads[0].userInfo["version"] as? Int == 1)
    }

    @Test(arguments: payloads)
    func malformedUserInfoIsRejected(_ payload: HandoffPayload) {
        let type = payload.activityType
        #expect(HandoffPayload(activityType: type, userInfo: nil) == nil)
        for key in payload.requiredUserInfoKeys {
            var missing = payload.userInfo
            missing.removeValue(forKey: key)
            #expect(HandoffPayload(activityType: type, userInfo: missing) == nil, "missing \(key)")
            var corrupt = payload.userInfo
            corrupt[key] = "not-a-uuid"
            #expect(HandoffPayload(activityType: type, userInfo: corrupt) == nil, "corrupt \(key)")
        }
        for address in ["ftp://folio.example.com", "folio.example.com", "https://"] {
            var info = payload.userInfo
            info["server"] = address
            #expect(HandoffPayload(activityType: type, userInfo: info) == nil, "server \(address)")
        }
        for version in [0, 2] as [Any] + ["1"] {
            var info = payload.userInfo
            info["version"] = version
            #expect(HandoffPayload(activityType: type, userInfo: info) == nil, "version \(version)")
        }
        #expect(HandoffPayload(activityType: "com.cedricziel.folio.search", userInfo: payload.userInfo) == nil)
        #expect(HandoffPayload(activityType: "com.example.document", userInfo: payload.userInfo) == nil)
    }

    @Test func thePageRoundTrips() {
        let payload = payloads[2]
        #expect(payload.userInfo["page"] as? Int == 7)
        #expect(HandoffPayload(activityType: payload.activityType, userInfo: payload.userInfo)?.target
            == .document(document, pane: .discussion, page: 7))
    }

    @Test func pagesThatAreNotIntegersAreDropped() {
        let base = payloads[0].userInfo
        let values: [Any?] = [nil, "7", 2.5]
        for value in values {
            var info = base
            info["page"] = value
            let decoded = HandoffPayload(activityType: HandoffActivityType.document, userInfo: info)
            #expect(decoded?.target == .document(document), "page \(String(describing: value))")
        }
        let onScreen = DocumentActivityPayload(server: server, vaultID: vault, documentID: document)
        #expect(
            HandoffPayload(activityType: HandoffActivityType.document, userInfo: onScreen.userInfo)?.target
                == .document(document, pane: nil, page: nil))
    }

    @Test func unknownExtrasAreIgnored() {
        var info = payloads[0].userInfo
        info["pane"] = "history"
        info["future"] = true
        #expect(HandoffPayload(activityType: HandoffActivityType.document, userInfo: info) == payloads[0])
    }
}
