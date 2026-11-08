import Foundation

/// A Gregorian calendar day encoded as YYYY-MM-DD, without a timezone.
public struct DocumentDate: RawRepresentable, Hashable, Sendable, Codable {
    public let rawValue: String

    public init?(rawValue: String) {
        let bytes = Array(rawValue.utf8)
        guard bytes.count == 10, bytes[4] == 45, bytes[7] == 45,
            bytes.enumerated().allSatisfy({ index, byte in
                index == 4 || index == 7 || (48...57).contains(byte)
            })
        else { return nil }
        let parts = rawValue.split(separator: "-")
        guard let year = Int(parts[0]), let month = Int(parts[1]), let day = Int(parts[2]),
            (1...9999).contains(year), (1...12).contains(month)
        else { return nil }
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
        let days = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        guard (1...days[month - 1]).contains(day) else { return nil }
        self.rawValue = rawValue
    }

    public init(from decoder: any Decoder) throws {
        let container = try decoder.singleValueContainer()
        let value = try container.decode(String.self)
        guard let date = Self(rawValue: value) else {
            throw DecodingError.dataCorruptedError(
                in: container, debugDescription: "Expected a valid YYYY-MM-DD calendar date")
        }
        self = date
    }

    /// Midnight at the start of this day in `timeZone`.
    public func start(in timeZone: TimeZone = .current) -> Date? {
        let parts = rawValue.split(separator: "-").compactMap { Int($0) }
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = timeZone
        return calendar.date(from: DateComponents(year: parts[0], month: parts[1], day: parts[2]))
    }

    public func encode(to encoder: any Encoder) throws {
        var container = encoder.singleValueContainer()
        try container.encode(rawValue)
    }
}
