import AppIntents
import CoreSpotlight
import Foundation
import UniformTypeIdentifiers

public struct DocumentEntity: AppEntity, IndexedEntity {
    public static let typeDisplayRepresentation = TypeDisplayRepresentation(
        name: "Document", numericFormat: "\(placeholder: .int) documents"
    )
    public static let defaultQuery = DocumentEntityQuery()

    public let id: DocumentEntityID

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

    /// What the original file is, so a transfer offers it under its own type.
    public let contentType: UTType
    public let hasRecognizedText: Bool
    /// Whether others can reach the document's vault, so the system confirms changes to it.
    public let sharing: VaultSharing

    public var documentID: Document.ID {
        id.documentID
    }

    public init(_ summary: DocumentSummary) {
        id = DocumentEntityID(documentID: summary.id, vaultID: summary.vaultID)
        contentType = DocumentTransfer.contentType(mimeType: summary.mimeType, filename: summary.originalFilename)
        hasRecognizedText = summary.hasRecognizedText
        sharing = summary.vaultSharing
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

    /// Documents gone from the vault their id names leave Spotlight. An id without a vault is looked up in the
    /// default vault, where other vaults' documents miss, and one the server couldn't answer for keeps its entry.
    public func entities(for identifiers: [DocumentEntityID]) async throws -> [DocumentEntity] {
        let resolution = try await IntentEnvironment.run { try await $0.resolve(ids: identifiers) }
        let gone = resolution.missing.filter { $0.vaultID != nil }.map(\.documentID)
        if !gone.isEmpty {
            IntentEnvironment.spotlight.remove(documentIDs: gone, fallbackServer: IntentEnvironment.serverURL())
        }
        return resolution.found.map(DocumentEntity.init)
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

    public let id: VaultScopedID

    @Property(title: "Name")
    public var name: String

    /// Shown under the name when the account has more than one vault.
    public let vaultName: String?

    public var tagID: Tag.ID {
        id.id
    }

    public init(_ tag: Tag, vaultID: UUID? = nil, vaultName: String? = nil) {
        id = VaultScopedID(tag.id, vaultID: vaultID)
        self.vaultName = vaultName
        name = tag.name
    }

    init(_ scoped: VaultScoped<Tag>) {
        self.init(scoped.value, vaultID: scoped.vaultID, vaultName: scoped.vaultName)
    }

    public var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(
            title: "\(name)", subtitle: vaultName.map { "\($0)" }, image: .init(systemName: "tag"))
    }
}

public struct TagEntityQuery: EntityStringQuery {
    public init() {}

    public func entities(for identifiers: [VaultScopedID]) async throws -> [TagEntity] {
        try await IntentEnvironment.run { try await $0.lookup(identifiers) { try await $0.tags(ids: $1) } }
            .map(TagEntity.init)
    }

    public func entities(matching string: String) async throws -> [TagEntity] {
        try await IntentEnvironment.run { try await $0.acrossVaults { try await $0.tags(matching: string) } }
            .map(TagEntity.init)
    }

    public func suggestedEntities() async throws -> [TagEntity] {
        try await IntentEnvironment.run { try await $0.acrossVaults { try await $0.tags() } }.map(TagEntity.init)
    }
}

public struct CorrespondentEntity: AppEntity {
    public static let typeDisplayRepresentation = TypeDisplayRepresentation(
        name: "Correspondent", numericFormat: "\(placeholder: .int) correspondents"
    )
    public static let defaultQuery = CorrespondentEntityQuery()

    public let id: VaultScopedID

    @Property(title: "Name")
    public var name: String

    /// Shown under the name when the account has more than one vault.
    public let vaultName: String?

    public var correspondentID: Correspondent.ID {
        id.id
    }

    public init(_ correspondent: Correspondent, vaultID: UUID? = nil, vaultName: String? = nil) {
        id = VaultScopedID(correspondent.id, vaultID: vaultID)
        self.vaultName = vaultName
        name = correspondent.name
    }

    init(_ scoped: VaultScoped<Correspondent>) {
        self.init(scoped.value, vaultID: scoped.vaultID, vaultName: scoped.vaultName)
    }

    public var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(
            title: "\(name)", subtitle: vaultName.map { "\($0)" }, image: .init(systemName: "person.crop.square"))
    }
}

public struct CorrespondentEntityQuery: EntityStringQuery {
    public init() {}

    public func entities(for identifiers: [VaultScopedID]) async throws -> [CorrespondentEntity] {
        try await IntentEnvironment.run { try await $0.lookup(identifiers) { try await $0.correspondents(ids: $1) } }
            .map(CorrespondentEntity.init)
    }

    public func entities(matching string: String) async throws -> [CorrespondentEntity] {
        try await IntentEnvironment.run {
            try await $0.acrossVaults { try await $0.correspondents(matching: string) }
        }
        .map(CorrespondentEntity.init)
    }

    public func suggestedEntities() async throws -> [CorrespondentEntity] {
        try await IntentEnvironment.run { try await $0.acrossVaults { try await $0.correspondents() } }
            .map(CorrespondentEntity.init)
    }
}

public struct DocumentTypeEntity: AppEntity {
    public static let typeDisplayRepresentation = TypeDisplayRepresentation(
        name: "Document type", numericFormat: "\(placeholder: .int) document types"
    )
    public static let defaultQuery = DocumentTypeEntityQuery()

    public let id: VaultScopedID

    @Property(title: "Name")
    public var name: String

    /// Shown under the name when the account has more than one vault.
    public let vaultName: String?

    public var documentTypeID: DocumentType.ID {
        id.id
    }

    public init(_ type: DocumentType, vaultID: UUID? = nil, vaultName: String? = nil) {
        id = VaultScopedID(type.id, vaultID: vaultID)
        self.vaultName = vaultName
        name = type.name
    }

    init(_ scoped: VaultScoped<DocumentType>) {
        self.init(scoped.value, vaultID: scoped.vaultID, vaultName: scoped.vaultName)
    }

    public var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(
            title: "\(name)", subtitle: vaultName.map { "\($0)" }, image: .init(systemName: "doc.plaintext"))
    }
}

public struct DocumentTypeEntityQuery: EntityStringQuery {
    public init() {}

    public func entities(for identifiers: [VaultScopedID]) async throws -> [DocumentTypeEntity] {
        try await IntentEnvironment.run { try await $0.lookup(identifiers) { try await $0.documentTypes(ids: $1) } }
            .map(DocumentTypeEntity.init)
    }

    public func entities(matching string: String) async throws -> [DocumentTypeEntity] {
        try await IntentEnvironment.run {
            try await $0.acrossVaults { try await $0.documentTypes(matching: string) }
        }
        .map(DocumentTypeEntity.init)
    }

    public func suggestedEntities() async throws -> [DocumentTypeEntity] {
        try await IntentEnvironment.run { try await $0.acrossVaults { try await $0.documentTypes() } }
            .map(DocumentTypeEntity.init)
    }
}

/// Switches vault entities and the vault parameter of intents off without a new build of the intents.
public enum FolioFeatures {
    public static let vaults = true
}

public struct VaultEntity: AppEntity {
    public static let typeDisplayRepresentation = TypeDisplayRepresentation(
        name: "Vault", numericFormat: "\(placeholder: .int) vaults"
    )
    public static let defaultQuery = VaultEntityQuery()

    public let id: UUID

    @Property(title: "Name")
    public var name: String

    public let role: Vault.Role
    /// Whether others can reach the vault, so the system confirms changes to it.
    public let sharing: VaultSharing

    public init(_ vault: Vault) {
        id = vault.id
        role = vault.role
        sharing = vault.sharing
        name = vault.name
    }

    public var displayRepresentation: DisplayRepresentation {
        let subtitle = sharing == .shared ? "\(role.title) · Shared" : role.title
        return DisplayRepresentation(
            title: "\(name)", subtitle: "\(subtitle)", image: .init(systemName: "lock.rectangle.stack"))
    }
}

public struct VaultEntityQuery: EntityStringQuery {
    public init() {}

    public func entities(for identifiers: [UUID]) async throws -> [VaultEntity] {
        try await vaults().filter { identifiers.contains($0.id) }
    }

    public func entities(matching string: String) async throws -> [VaultEntity] {
        let needle = string.trimmingCharacters(in: .whitespacesAndNewlines)
        return try await vaults().filter { needle.isEmpty || $0.name.localizedCaseInsensitiveContains(needle) }
    }

    public func suggestedEntities() async throws -> [VaultEntity] {
        try await vaults()
    }

    private func vaults() async throws -> [VaultEntity] {
        guard FolioFeatures.vaults else { return [] }
        return try await IntentEnvironment.run { try await $0.vaults() }.vaults.map(VaultEntity.init)
    }
}

#if compiler(>=6.4)
    @available(iOS 27, macOS 27, *)
    extension EntityOwnership {
        init(_ sharing: VaultSharing) {
            switch sharing {
            case .personal: self = []
            case .shared: self = .shared
            case .unknown: self = .unknown
            }
        }
    }

    @available(iOS 27, macOS 27, *)
    extension DocumentEntity: OwnershipProvidingEntity {
        public var ownership: EntityOwnership {
            EntityOwnership(sharing)
        }
    }

    @available(iOS 27, macOS 27, *)
    extension VaultEntity: OwnershipProvidingEntity {
        public var ownership: EntityOwnership {
            EntityOwnership(sharing)
        }
    }
#endif
