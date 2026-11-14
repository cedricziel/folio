import Foundation
import Testing

@testable import FolioKit

@Suite("Vault role decoding")
struct VaultRoleTests {
    private let known = "11111111-1111-1111-1111-111111111111"
    private let novel = "22222222-2222-2222-2222-222222222222"

    private var payload: Data {
        Data(
            """
            {"vaults":[
              {"id":"\(known)","name":"Home","role":"owner"},
              {"id":"\(novel)","name":"Archive","role":"something_new"}
            ],"default_vault_id":"\(known)"}
            """.utf8)
    }

    @Test func unknownRoleDoesNotFailTheList() throws {
        let catalog = try JSONDecoder().decode(VaultCatalog.self, from: payload)
        #expect(catalog.vaults.count == 2)
        #expect(catalog.vaults[0].role == .owner)
        #expect(catalog.vaults[1].role == .unknown("something_new"))
    }

    @Test func unknownRoleIsReadOnly() throws {
        let role = try JSONDecoder().decode(VaultCatalog.self, from: payload).vaults[1].role
        #expect(!role.canUpload)
        #expect(!role.canEdit)
    }

    @Test func unknownRoleNeverShowsTheServerString() {
        #expect(!Vault.Role.unknown("something_new").title.contains("something"))
    }

    @Test func unknownVaultIsNotTreatedAsOwned() throws {
        let vault = try JSONDecoder().decode(VaultCatalog.self, from: payload).vaults[1]
        #expect(vault.sharing == .shared)
    }

    @Test func knownRolesKeepTheirPrivileges() throws {
        for (raw, upload, edit) in [
            ("viewer", false, false), ("contributor", true, false), ("editor", true, true), ("owner", true, true),
        ] {
            let role = try JSONDecoder().decode(Vault.Role.self, from: Data("\"\(raw)\"".utf8))
            #expect(role.canUpload == upload && role.canEdit == edit)
        }
    }

    @Test func roundTripKeepsUnknownRole() throws {
        let catalog = try JSONDecoder().decode(VaultCatalog.self, from: payload)
        let again = try JSONDecoder().decode(VaultCatalog.self, from: JSONEncoder().encode(catalog))
        #expect(again == catalog)
        let json = try #require(String(data: JSONEncoder().encode(catalog.vaults[1].role), encoding: .utf8))
        #expect(json == "\"something_new\"")
    }
}
