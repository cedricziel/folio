import Foundation

public struct DocumentDatePatchPayload: Codable, Hashable, Sendable {
    public var documentDate: String??

    public init(documentDate: String?? = nil) {
        self.documentDate = documentDate
    }

    private enum CodingKeys: String, CodingKey {
        case documentDate = "document_date"
    }

    public init(from decoder: any Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        documentDate =
            values.contains(.documentDate)
            ? .some(try values.decodeIfPresent(String.self, forKey: .documentDate)) : nil
    }

    public func encode(to encoder: any Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        if let documentDate {
            try values.encode(documentDate, forKey: .documentDate)
        }
    }
}
