import Foundation
import Testing

@testable import FolioKit

private let vault = UUID(uuidString: "AAAAAAAA-BBBB-4CCC-8DDD-EEEEEEEEEEEE")!
private let document = UUID(uuidString: "11111111-2222-4333-8444-555555555555")!
private let savedView = UUID(uuidString: "66666666-7777-4888-8999-AAAAAAAAAAAA")!
private let v = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"
private let d = "11111111-2222-4333-8444-555555555555"
private let s = "66666666-7777-4888-8999-aaaaaaaaaaaa"

struct HandoffLinkTests {
    @Test(arguments: [
        ("https://folio.example.com", HandoffTarget.document(document), "https://folio.example.com/vaults/\(v)/documents/\(d)"),
        ("https://folio.example.com/", .document(document, pane: .discussion), "https://folio.example.com/vaults/\(v)/documents/\(d)"),
        ("https://folio.example.com", .library(.inbox), "https://folio.example.com/vaults/\(v)/"),
        ("https://folio.example.com", .library(.all), "https://folio.example.com/vaults/\(v)/?view=all"),
        ("https://folio.example.com", .savedView(savedView), "https://folio.example.com/vaults/\(v)/saved-views/\(s)"),
        ("https://example.com/folio", .document(document), "https://example.com/folio/vaults/\(v)/documents/\(d)"),
        ("https://example.com/folio/", .library(.inbox), "https://example.com/folio/vaults/\(v)/"),
        ("https://example.com/folio", .library(.all), "https://example.com/folio/vaults/\(v)/?view=all"),
        ("http://folio.lan:3000/folio", .savedView(savedView), "http://folio.lan:3000/folio/vaults/\(v)/saved-views/\(s)"),
    ])
    func webURLsPointAtTheSameTarget(_ server: String, _ target: HandoffTarget, _ expected: String) throws {
        let payload = HandoffPayload(server: try #require(URL(string: server)), vaultID: vault, target: target)
        #expect(HandoffLink.webURL(for: payload)?.absoluteString == expected)
    }

    @Test(arguments: [
        ("https://folio.example.com", HandoffTarget.document(document)),
        ("https://folio.example.com", .library(.inbox)),
        ("https://folio.example.com", .library(.all)),
        ("https://folio.example.com", .savedView(savedView)),
        ("https://example.com/folio", .document(document)),
        ("https://example.com/a/b", .library(.all)),
        ("http://folio.lan:3000/folio", .savedView(savedView)),
    ])
    func webURLsParseBack(_ server: String, _ target: HandoffTarget) throws {
        let payload = HandoffPayload(server: try #require(URL(string: server)), vaultID: vault, target: target)
        let url = try #require(HandoffLink.webURL(for: payload))
        #expect(HandoffLink.parse(url) == payload)
    }

    @Test func linkUnderAPathPrefixNamesThePrefixedServer() throws {
        let link = try #require(URL(string: "https://example.com/folio/vaults/\(v)/documents/\(d)"))
        let payload = try #require(HandoffLink.parse(link))
        #expect(payload.server.absoluteString == "https://example.com/folio")
        #expect(payload.target == .document(document))
        #expect(payload.vaultID == vault)
    }

    @Test func vaultRootWithoutTrailingSlashIsTheInbox() throws {
        let link = try #require(URL(string: "https://folio.example.com/vaults/\(v)"))
        #expect(HandoffLink.parse(link)?.target == .library(.inbox))
    }

    @Test(arguments: [
        "https://folio.example.com/vaults/\(v)/documents/\(d)/download",
        "https://folio.example.com/vaults/\(v)/documents/\(d)/",
        "https://folio.example.com/vaults/\(v)/entries",
        "https://folio.example.com/vaults/\(v)/documents/search",
        "https://folio.example.com/vaults/\(v)/saved-views/new",
        "https://folio.example.com/vaults/not-a-uuid/documents/\(d)",
        "https://folio.example.com/vaults/\(v)/documents/not-a-uuid",
        "https://folio.example.com/documents/\(d)",
        "https://folio.example.com/login",
        "https://folio.example.com/oauth/authorize",
        "https://folio.example.com/",
        "ftp://folio.example.com/vaults/\(v)/",
        "https://folio.example.com//vaults/\(v)/",
    ])
    func otherPagesAreNotTargets(_ text: String) throws {
        #expect(HandoffLink.parse(try #require(URL(string: text))) == nil)
    }
}
