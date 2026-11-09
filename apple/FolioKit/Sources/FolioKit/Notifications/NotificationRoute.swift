import Foundation

/// Where a delivered notification leads, read from its `folio` payload.
///
/// The payload only ever carries IDs and counts; titles are fetched by the device when the person opted in.
public struct FolioNotification: Equatable, Sendable {
    public static let documentCategory = "FOLIO_DOCUMENT"
    public static let inboxCategory = "FOLIO_INBOX"
    public static let testCategory = "FOLIO_TEST"
    public static let openAction = "FOLIO_OPEN"
    public static let markReviewedAction = "FOLIO_MARK_REVIEWED"

    /// Set when the notification covers exactly one document
    public var documentID: UUID?
    /// Set when every document is in one vault
    public var vaultID: UUID?
    public var needsReview: Int
    public var failed: Int
    public var processed: Int
    public var isTest: Bool

    /// Nil for notifications that are not Folio's or that a newer payload version wrote.
    public init?(userInfo: [AnyHashable: Any]) {
        guard let folio = userInfo["folio"] as? [String: Any], (folio["v"] as? Int) == 1 else { return nil }
        documentID = (folio["document"] as? String).flatMap(UUID.init(uuidString:))
        vaultID = (folio["vault"] as? String).flatMap(UUID.init(uuidString:))
        let counts = folio["counts"] as? [String: Any] ?? [:]
        needsReview = counts["needs_review"] as? Int ?? 0
        failed = counts["failed"] as? Int ?? 0
        processed = counts["processed"] as? Int ?? 0
        isTest = folio["test"] as? Bool ?? false
    }

    /// Opens the single document, otherwise the Inbox.
    public var route: IntentRouter.Request {
        if let documentID {
            return .document(documentID, vaultID: vaultID)
        }
        return .inbox
    }

    /// Mark reviewed applies to one document that needs review.
    public var canMarkReviewed: Bool {
        documentID != nil && needsReview > 0
    }
}
