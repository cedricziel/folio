import Foundation
import Testing

@testable import FolioKit

struct AppConfigurationTests {
    @Test func readsTheAppGroupAndCallbackSchemeFromInfoPlist() {
        let info: [String: Any] = [
            "FolioAppGroup": "group.com.example.folio", "FolioCallbackScheme": "com.example.folio",
        ]

        #expect(AppConfiguration.appGroup(in: info) == "group.com.example.folio")
        #expect(AppConfiguration.callbackScheme(in: info) == "com.example.folio")
        #expect(AppConfiguration.redirectURI(scheme: "com.example.folio") == "com.example.folio:/oauth/callback")
    }

    @Test func fallsBackToTheOfficialIdentifiers() {
        let missing: [[String: Any]?] = [nil, [:], ["FolioAppGroup": "", "FolioCallbackScheme": "  "]]
        for info in missing {
            #expect(AppConfiguration.appGroup(in: info) == "group.com.cedricziel.folio")
            #expect(AppConfiguration.callbackScheme(in: info) == "com.cedricziel.folio")
        }
    }

    @Test func unitTestsRunWithTheOfficialIdentifiers() {
        #expect(UserDefaultsServerConfigStore.appGroup == "group.com.cedricziel.folio")
        #expect(OAuthConfiguration.callbackScheme == "com.cedricziel.folio")
        #expect(OAuthConfiguration.redirectURI == "com.cedricziel.folio:/oauth/callback")
    }
}
