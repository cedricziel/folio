import Foundation

public struct SourceOutcome: Identifiable, Sendable, Equatable {
    public enum Result: String, Sendable, CaseIterable {
        case imported, skipped, duplicate, failed

        public var title: String {
            switch self {
            case .imported: "Imported"
            case .skipped: "Skipped by a rule"
            case .duplicate: "Duplicate"
            case .failed: "Import failed"
            }
        }

        public var explanation: String {
            switch self {
            case .imported: "The document was saved."
            case .skipped: "A rule stopped this item before import."
            case .duplicate: "This content is already in the vault."
            case .failed: "The import did not finish. Check the source before trying again."
            }
        }
    }

    public let id: Int64
    public let itemID: UUID
    public let attemptID: UUID
    public let occurredAt: Date
    public let outcome: Result
    public let documentID: UUID?

    public init(id: Int64, itemID: UUID, attemptID: UUID, occurredAt: Date, outcome: Result, documentID: UUID? = nil) {
        self.id = id
        self.itemID = itemID
        self.attemptID = attemptID
        self.occurredAt = occurredAt
        self.outcome = outcome
        self.documentID = outcome == .imported ? documentID : nil
    }
}

public struct SourceOutcomePage: Sendable {
    public let vaultID: UUID
    public let sourceID: UUID
    public let outcomes: [SourceOutcome]
    public let nextBefore: Int64?

    public init(vaultID: UUID, sourceID: UUID, outcomes: [SourceOutcome], nextBefore: Int64? = nil) {
        self.vaultID = vaultID
        self.sourceID = sourceID
        self.outcomes = outcomes
        self.nextBefore = nextBefore
    }
}

public protocol SourceOutcomeService: Sendable {
    func sourceOutcomes(vaultID: UUID, sourceID: UUID, before: Int64?) async throws -> SourceOutcomePage
}
