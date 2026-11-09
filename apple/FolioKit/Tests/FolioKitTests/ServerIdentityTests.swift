import Foundation
import Testing

@testable import FolioKit

struct ServerIdentityTests {
    @Test(arguments: [
        ("https://Folio.Example.com:443/", "https://folio.example.com"),
        ("HTTPS://folio.example.com", "https://folio.example.com"),
        ("http://folio.lan:80", "http://folio.lan/"),
        ("https://folio.example.com/folio/", "https://folio.example.com/folio"),
        ("https://me:secret@folio.example.com/?q=1#top", "https://folio.example.com"),
        ("https://bücher.example", "https://xn--bcher-kva.example"),
        ("https://b%C3%BCcher.example", "https://xn--bcher-kva.example"),
        ("https://BÜCHER.example", "https://bücher.example"),
        ("http://[::1]:8080/", "http://[::1]:8080"),
    ])
    func sameServer(_ lhs: String, _ rhs: String) throws {
        let left = try #require(URL(string: lhs)), right = try #require(URL(string: rhs))
        #expect(ServerIdentity(left) != nil)
        #expect(ServerIdentity(left) == ServerIdentity(right))
        #expect(ServerIdentity.same(left, right))
    }

    @Test(arguments: [
        ("https://folio.example.com", "http://folio.example.com"),
        ("https://folio.example.com", "https://folio.example.com:8443"),
        ("http://folio.lan:3000", "http://folio.lan"),
        ("https://example.com/folio", "https://example.com"),
        ("https://example.com/Folio", "https://example.com/folio"),
        ("http://[::1]:8080", "http://[::1]"),
        ("https://folio.example.com", "https://folio.example.org"),
    ])
    func differentServer(_ lhs: String, _ rhs: String) throws {
        let left = try #require(URL(string: lhs)), right = try #require(URL(string: rhs))
        #expect(ServerIdentity(left) != ServerIdentity(right))
        #expect(!ServerIdentity.same(left, right))
    }

    @Test(arguments: ["ftp://folio.example.com", "file:///tmp/folio", "folio.example.com"])
    func onlyHTTPServersHaveAnIdentity(_ text: String) throws {
        let url = try #require(URL(string: text))
        #expect(ServerIdentity(url) == nil)
    }

    @Test func describesTheCanonicalForm() throws {
        let url = try #require(URL(string: "HTTP://[::1]:8080/folio/"))
        let identity = try #require(ServerIdentity(url))
        #expect(identity.description == "http://[::1]:8080/folio")
        #expect(identity.displayHost == "[::1]:8080")
    }

    @Test(arguments: [("bücher", "bcher-kva"), ("münchen", "mnchen-3ya"), ("例え", "r8jz45g"), ("ü", "tda")])
    func punycodeMatchesRFC3492(_ label: String, _ encoded: String) {
        #expect(Punycode.encode(label) == encoded)
    }
}
