import Foundation
import Testing

@testable import FolioKit

struct DocumentLinkTests {
    private let vaultID = UUID(uuidString: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee")!
    private let documentID = UUID(uuidString: "3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f")!

    @Test func spotlightDonationCarriesTheDocumentVault() {
        let summary = DocumentSummary(id: documentID, title: "Household letter", date: Date(timeIntervalSince1970: 0))
        let item = SpotlightItem(summary, vaultID: vaultID, server: URL(string: "https://folio.example")!)
            .searchableItem

        #expect(
            item.uniqueIdentifier
                == "folio:document:aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee:3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f")
        #expect(
            DocumentLink(spotlightIdentifier: item.uniqueIdentifier)
                == DocumentLink(documentID: documentID, vaultID: vaultID))
        #expect(item.attributeSet.title == "Household letter")
    }

    @Test(arguments: [
        "3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f",
        "DocumentEntity/3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f",
        "unrelated:3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f",
    ])
    func unscopedIdentifiersAreRejected(identifier: String) {
        #expect(DocumentLink(spotlightIdentifier: identifier) == nil)
    }

    @Test(arguments: [
        "folio:document:invalid:3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f",
        "folio:document::3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f",
        "folio:other:aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee:3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f",
        "folio:document:aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee:3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f:extra",
    ])
    func malformedScopedLinkCannotBecomeADefaultVaultLink(identifier: String) {
        #expect(DocumentLink(spotlightIdentifier: identifier) == nil)
    }
}
