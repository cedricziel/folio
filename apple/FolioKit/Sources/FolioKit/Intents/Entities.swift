import AppIntents
import CoreSpotlight
import Foundation

public struct DocumentEntity: AppEntity, IndexedEntity {
    public static let typeDisplayRepresentation = TypeDisplayRepresentation(
        name: "Document", numericFormat: "\(placeholder: .int) documents"
    )
    public static let defaultQuery = DocumentEntityQuery()

    public let id: UUID

    @Property(title: "Title")
    public var title: String

    @Property(title: "Correspondent")
    public var correspondent: String?

    @Property(title: "Type")
    public var documentType: String?

    @Property(title: "Date")
    public var date: Date

    @Property(title: "Tags")
    public var tags: [String]

    public init(_ summary: DocumentSummary) {
        id = summary.id
        title = summary.title
        correspondent = summary.correspondent
        documentType = summary.documentType
        date = summary.date
        tags = summary.tags
    }

    public var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(
            title: "\(title)",
            subtitle: subtitle.map { "\($0)" },
            image: .init(systemName: "doc.text")
        )
    }

    private var subtitle: String? {
        let parts = [correspondent, documentType, FolioFormat.isoDay(date)].compactMap { $0 }
        return parts.joined(separator: " · ")
    }

    public var attributeSet: CSSearchableItemAttributeSet {
        let attributes = CSSearchableItemAttributeSet(contentType: .content)
        attributes.title = title
        attributes.displayName = title
        attributes.contentDescription = subtitle
        attributes.keywords = tags + [correspondent, documentType].compactMap { $0 }
        attributes.contentCreationDate = date
        return attributes
    }
}

public struct DocumentEntityQuery: EntityStringQuery {
    public init() {}

    public func entities(for identifiers: [UUID]) async throws -> [DocumentEntity] {
        try await IntentEnvironment.run { try await $0.documents(ids: identifiers) }.map(DocumentEntity.init)
    }

    public func entities(matching string: String) async throws -> [DocumentEntity] {
        try await IntentEnvironment.run { try await $0.search(string) }.map(DocumentEntity.init)
    }

    public func suggestedEntities() async throws -> [DocumentEntity] {
        try await IntentEnvironment.run { try await $0.recent() }.map(DocumentEntity.init)
    }
}

public struct TagEntity: AppEntity {
    public static let typeDisplayRepresentation = TypeDisplayRepresentation(
        name: "Tag", numericFormat: "\(placeholder: .int) tags"
    )
    public static let defaultQuery = TagEntityQuery()

    public let id: UUID

    @Property(title: "Name")
    public var name: String

    public init(_ tag: Tag) {
        id = tag.id
        name = tag.name
    }

    public var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(name)", image: .init(systemName: "tag"))
    }
}

public struct TagEntityQuery: EntityStringQuery {
    public init() {}

    public func entities(for identifiers: [UUID]) async throws -> [TagEntity] {
        try await IntentEnvironment.run { try await $0.tags(ids: identifiers) }.map(TagEntity.init)
    }

    public func entities(matching string: String) async throws -> [TagEntity] {
        try await IntentEnvironment.run { try await $0.tags(matching: string) }.map(TagEntity.init)
    }

    public func suggestedEntities() async throws -> [TagEntity] {
        try await IntentEnvironment.run { try await $0.tags() }.map(TagEntity.init)
    }
}

public struct CorrespondentEntity: AppEntity {
    public static let typeDisplayRepresentation = TypeDisplayRepresentation(
        name: "Correspondent", numericFormat: "\(placeholder: .int) correspondents"
    )
    public static let defaultQuery = CorrespondentEntityQuery()

    public let id: UUID

    @Property(title: "Name")
    public var name: String

    public init(_ correspondent: Correspondent) {
        id = correspondent.id
        name = correspondent.name
    }

    public var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(name)", image: .init(systemName: "person.crop.square"))
    }
}

public struct CorrespondentEntityQuery: EntityStringQuery {
    public init() {}

    public func entities(for identifiers: [UUID]) async throws -> [CorrespondentEntity] {
        try await IntentEnvironment.run { try await $0.correspondents(ids: identifiers) }
            .map(CorrespondentEntity.init)
    }

    public func entities(matching string: String) async throws -> [CorrespondentEntity] {
        try await IntentEnvironment.run { try await $0.correspondents(matching: string) }
            .map(CorrespondentEntity.init)
    }

    public func suggestedEntities() async throws -> [CorrespondentEntity] {
        try await IntentEnvironment.run { try await $0.correspondents() }.map(CorrespondentEntity.init)
    }
}

public struct DocumentTypeEntity: AppEntity {
    public static let typeDisplayRepresentation = TypeDisplayRepresentation(
        name: "Document type", numericFormat: "\(placeholder: .int) document types"
    )
    public static let defaultQuery = DocumentTypeEntityQuery()

    public let id: UUID

    @Property(title: "Name")
    public var name: String

    public init(_ type: DocumentType) {
        id = type.id
        name = type.name
    }

    public var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(name)", image: .init(systemName: "doc.plaintext"))
    }
}

public struct DocumentTypeEntityQuery: EntityStringQuery {
    public init() {}

    public func entities(for identifiers: [UUID]) async throws -> [DocumentTypeEntity] {
        try await IntentEnvironment.run { try await $0.documentTypes(ids: identifiers) }
            .map(DocumentTypeEntity.init)
    }

    public func entities(matching string: String) async throws -> [DocumentTypeEntity] {
        try await IntentEnvironment.run { try await $0.documentTypes(matching: string) }
            .map(DocumentTypeEntity.init)
    }

    public func suggestedEntities() async throws -> [DocumentTypeEntity] {
        try await IntentEnvironment.run { try await $0.documentTypes() }.map(DocumentTypeEntity.init)
    }
}

/// Vault entities stay inactive until intents can scope every operation to the selected vault.
public enum FolioFeatures {
    public static let vaults = false
}

public struct VaultEntity: AppEntity {
    public static let typeDisplayRepresentation = TypeDisplayRepresentation(
        name: "Vault", numericFormat: "\(placeholder: .int) vaults"
    )
    public static let defaultQuery = VaultEntityQuery()

    public let id: UUID

    @Property(title: "Name")
    public var name: String

    public init(id: UUID, name: String) {
        self.id = id
        self.name = name
    }

    public var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(name)", image: .init(systemName: "lock.rectangle.stack"))
    }
}

public struct VaultEntityQuery: EntityStringQuery {
    public init() {}

    public func entities(for _: [UUID]) async throws -> [VaultEntity] {
        try vaults()
    }

    public func entities(matching _: String) async throws -> [VaultEntity] {
        try vaults()
    }

    public func suggestedEntities() async throws -> [VaultEntity] {
        try vaults()
    }

    private func vaults() throws -> [VaultEntity] {
        guard FolioFeatures.vaults else { return [] }
        return []
    }
}
