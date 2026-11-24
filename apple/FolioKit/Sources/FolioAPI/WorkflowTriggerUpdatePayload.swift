import Foundation
import OpenAPIRuntime

public struct WorkflowTriggerUpdatePayload: Codable, Hashable, Sendable {
    public let expectedRevision: Int64
    public let trigger: Components.Schemas.WorkflowTrigger
    public let documentChange: OpenAPIObjectContainer??

    public init(
        expectedRevision: Int64, trigger: Components.Schemas.WorkflowTrigger,
        documentChange: OpenAPIObjectContainer?? = nil
    ) {
        self.expectedRevision = expectedRevision
        self.trigger = trigger
        self.documentChange = documentChange
    }

    private enum CodingKeys: String, CodingKey {
        case expectedRevision = "expected_revision"
        case trigger
        case documentChange = "document_change"
    }

    public init(from decoder: any Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        expectedRevision = try values.decode(Int64.self, forKey: .expectedRevision)
        trigger = try values.decode(Components.Schemas.WorkflowTrigger.self, forKey: .trigger)
        documentChange =
            values.contains(.documentChange)
            ? .some(try values.decodeIfPresent(OpenAPIObjectContainer.self, forKey: .documentChange)) : nil
    }

    public func encode(to encoder: any Encoder) throws {
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(expectedRevision, forKey: .expectedRevision)
        try values.encode(trigger, forKey: .trigger)
        if let documentChange {
            try values.encode(documentChange, forKey: .documentChange)
        }
    }
}
