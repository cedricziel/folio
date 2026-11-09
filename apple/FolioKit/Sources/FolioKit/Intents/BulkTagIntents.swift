import AppIntents
import Foundation

#if compiler(>=6.4)
    /// Tags many documents at once. The documents arrive as identifiers only, so none is fetched to run it.
    @available(iOS 27, macOS 27, *)
    public struct TagDocumentsIntent: AppIntent, UndoableIntent {
        public static let title: LocalizedStringResource = "Add a tag to documents"
        public static let description = IntentDescription(
            "Puts a tag on many documents in Folio at once.", categoryName: "Documents"
        )

        @Parameter(title: "Documents")
        public var documents: EntityCollection<DocumentEntity>

        @Parameter(title: "Tag")
        public var tag: TagEntity

        public static var parameterSummary: some ParameterSummary {
            Summary("Add \(\.$tag) to \(\.$documents)")
        }

        public init() {}

        public init(documents: EntityCollection<DocumentEntity>, tag: TagEntity) {
            self.documents = documents
            self.tag = tag
        }

        public func perform() async throws -> some IntentResult & ProvidesDialog {
            IntentEnvironment.ran(.tagDocuments)
            let message = try await applyTag(.add, tag: tag, documents: documents.identifiers, intent: self)
            return .result(dialog: "\(message)")
        }
    }

    /// Takes a tag off many documents at once. The documents arrive as identifiers only.
    @available(iOS 27, macOS 27, *)
    public struct UntagDocumentsIntent: AppIntent, UndoableIntent {
        public static let title: LocalizedStringResource = "Remove a tag from documents"
        public static let description = IntentDescription(
            "Takes a tag off many documents in Folio at once.", categoryName: "Documents"
        )

        @Parameter(title: "Documents")
        public var documents: EntityCollection<DocumentEntity>

        @Parameter(title: "Tag")
        public var tag: TagEntity

        public static var parameterSummary: some ParameterSummary {
            Summary("Remove \(\.$tag) from \(\.$documents)")
        }

        public init() {}

        public init(documents: EntityCollection<DocumentEntity>, tag: TagEntity) {
            self.documents = documents
            self.tag = tag
        }

        public func perform() async throws -> some IntentResult & ProvidesDialog {
            IntentEnvironment.ran(.untagDocuments)
            let message = try await applyTag(.remove, tag: tag, documents: documents.identifiers, intent: self)
            return .result(dialog: "\(message)")
        }
    }

    @available(iOS 27, macOS 27, *)
    private func applyTag(
        _ kind: TagChange.Kind, tag: TagEntity, documents: [DocumentEntityID], intent: some UndoableIntent
    ) async throws -> String {
        let change = TagChange(kind, tagID: tag.tagID, tagVaultID: tag.id.vaultID, documents: documents)
        let outcome = try await IntentEnvironment.run { try await $0.apply(change) }
        await registerUndo(of: outcome.applied, for: intent)
        return outcome.summary(tagName: tag.name)
    }

    @available(iOS 27, macOS 27, *)
    @MainActor private func registerUndo(of change: TagChange, for intent: some UndoableIntent) {
        TagUndo.register(change, with: intent.undoManager, revert: TagUndo.revert)
    }
#endif
