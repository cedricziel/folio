import Foundation

public struct DocumentFileRevision: Identifiable, Equatable, Sendable {
    public enum Verification: String, Sendable {
        case pending, verified, unavailable
    }

    public let id: UUID
    public let number: Int64
    public let isCurrent: Bool
    public let createdAt: Date
    public let filename: String?
    public let mimeType: String
    public let verification: Verification
    public let byteSize: Int64?
}

public struct DocumentFileRevisionPage: Equatable, Sendable {
    public let revisions: [DocumentFileRevision]
    public let nextBefore: Int64?
}
