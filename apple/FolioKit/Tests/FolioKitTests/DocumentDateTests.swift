import Foundation
import Testing

@testable import FolioKit

struct DocumentDateTests {
    @Test func startsAtMidnightInTheGivenTimeZone() throws {
        let day = try #require(DocumentDate(rawValue: "2024-03-01"))
        #expect(day.start(in: TimeZone(identifier: "UTC")!) == Date(timeIntervalSince1970: 1_709_251_200))
        #expect(day.start(in: TimeZone(secondsFromGMT: 3600)!) == Date(timeIntervalSince1970: 1_709_247_600))
    }

    @Test(arguments: ["2024-02-29", "2000-02-29", "0001-01-01", "9999-12-31", "2026-11-02"])
    func calendarDatesRoundTrip(value: String) throws {
        let date = try #require(DocumentDate(rawValue: value))
        let data = try JSONEncoder().encode(date)

        #expect(try JSONDecoder().decode(DocumentDate.self, from: data).rawValue == value)
        #expect(String(decoding: data, as: UTF8.self) == "\"\(value)\"")
    }

    @Test(arguments: [
        "2025-02-29", "1900-02-29", "2026-04-31", "2026-00-01", "2026-13-01",
        "2026-01-00", "0000-01-01", "10000-01-01", "2026-1-02", "26-01-02",
        "2026-11-02T00:00:00Z", " 2026-11-02", "2026-11-02 ", "２０２６-１１-０２",
    ])
    func malformedOrImpossibleDatesAreRejected(value: String) throws {
        #expect(DocumentDate(rawValue: value) == nil)
        let data = try JSONEncoder().encode(value)
        #expect(throws: DecodingError.self) {
            try JSONDecoder().decode(DocumentDate.self, from: data)
        }
    }

    @Test(arguments: ["2024-03-10", "2024-11-03", "1500-03-01", "1582-10-10"])
    func historicalAndDaylightSavingDatesKeepTheirLiteralDay(value: String) throws {
        let data = try JSONEncoder().encode(value)
        let date = try JSONDecoder().decode(DocumentDate.self, from: data)

        #expect(date.rawValue == value)
        #expect(try JSONEncoder().encode(date) == data)
    }
}
