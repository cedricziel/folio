import FolioAPI
import Foundation
import Testing

struct DocumentDatePatchTests {
    @Test func omittedDateDoesNotSendAField() throws {
        let data = try JSONEncoder().encode(DocumentDatePatchPayload())
        #expect(String(decoding: data, as: UTF8.self) == "{}")
    }

    @Test func clearingDateSendsNull() throws {
        let data = try JSONEncoder().encode(DocumentDatePatchPayload(documentDate: .some(nil)))
        #expect(String(decoding: data, as: UTF8.self) == "{\"document_date\":null}")
        let decoded = try JSONDecoder().decode(DocumentDatePatchPayload.self, from: data)
        #expect(decoded.documentDate != nil)
        #expect(decoded.documentDate! == nil)
    }

    @Test func historicalDayRoundTripsWithoutCalendarConversion() throws {
        let patch = DocumentDatePatchPayload(documentDate: .some("1582-10-10"))
        let data = try JSONEncoder().encode(patch)
        #expect(try JSONDecoder().decode(DocumentDatePatchPayload.self, from: data) == patch)
    }
}
