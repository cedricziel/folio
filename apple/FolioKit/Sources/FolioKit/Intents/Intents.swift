import AppIntents
import Foundation
import UniformTypeIdentifiers

private func countText(_ count: Int, _ noun: String) -> String {
    "\(count) \(noun)\(count == 1 ? "" : "s")"
}

public struct ScanDocumentIntent: AppIntent {
    public static let title: LocalizedStringResource = "Scan a document"
    public static let description = IntentDescription(
        "Opens Folio's scanner so you can photograph a paper document and file it."
    )
    public static let openAppWhenRun = true

    public init() {}

    @MainActor
    public func perform() async throws -> some IntentResult {
        IntentEnvironment.ran(.scanDocument)
        IntentRouter.shared.route(.scan)
        return .result()
    }
}

/// Reports progress to the system, and from iOS and macOS 27 keeps running in the background for as long as it
/// keeps reporting. Cancelling stops before the next file; running it again resumes instead of uploading twice.
public struct UploadFileIntent: AppIntent, ProgressReportingIntent {
    public static let title: LocalizedStringResource = "Upload to Folio"
    public static let description = IntentDescription(
        "Files PDFs and images in Folio. Folio reads the text for you.",
        categoryName: "Documents", resultValueName: "Uploaded documents"
    )

    @Parameter(
        title: "Files", description: "PDFs or images to file",
        supportedContentTypes: [.pdf, .image]
    )
    public var files: [IntentFile]

    @Parameter(title: "Title", description: "Used when you upload a single file")
    public var title: String?

    @Parameter(title: "Tags")
    public var tags: [TagEntity]

    @Parameter(title: "Vault", description: "Where to file them; your default vault when empty")
    public var vault: VaultEntity?

    public static var parameterSummary: some ParameterSummary {
        Summary("Upload \(\.$files) to Folio") {
            \.$title
            \.$tags
            \.$vault
        }
    }

    public init() {}

    public init(files: [IntentFile], title: String? = nil, tags: [TagEntity] = [], vault: VaultEntity? = nil) {
        self.files = files
        self.title = title
        self.tags = tags
        self.vault = vault
    }

    public func perform() async throws -> some IntentResult & ReturnsValue<[DocumentEntity]> & ProvidesDialog {
        IntentEnvironment.ran(.uploadFile)
        guard !files.isEmpty else { throw ShortcutError("Choose at least one file to upload.") }
        #if compiler(>=6.4)
            let summaries: [DocumentSummary]
            if #available(iOS 27, macOS 27, *) {
                summaries = try await performBackgroundTask { try await upload() }
            } else {
                summaries = try await upload()
            }
        #else
            let summaries = try await upload()
        #endif
        return .result(
            value: summaries.map(DocumentEntity.init),
            dialog: "Filed \(countText(summaries.count, "document")) in Folio."
        )
    }

    private func upload() async throws -> [DocumentSummary] {
        let reporter = UploadProgressReporter(progress, total: files.count)
        return try await IntentEnvironment.run {
            let scope = try DocumentCatalog.vault(asked: vault?.id, implied: tags.map(\.id.vaultID))
            return try await $0.scoped(to: scope).upload(
                files.map { try UploadFile(intentFile: $0) }, title: title, tagIDs: tags.map(\.tagID),
                finished: { reporter.finished($0) })
        }
    }
}

#if compiler(>=6.4)
    @available(iOS 27, macOS 27, *)
    extension UploadFileIntent: LongRunningIntent {}
#endif

/// Shows how many of an upload's files are done, the way the in-app upload reports it.
struct UploadProgressReporter: Sendable {
    private let progress: Progress
    private let total: Int

    init(_ progress: Progress, total: Int) {
        self.progress = progress
        self.total = total
        progress.totalUnitCount = Int64(total)
        progress.completedUnitCount = 0
        progress.localizedDescription = "Uploading to Folio"
        progress.localizedAdditionalDescription = Self.detail(0, of: total)
    }

    func finished(_ count: Int) {
        progress.completedUnitCount = Int64(count)
        progress.localizedAdditionalDescription = Self.detail(count, of: total)
    }

    private static func detail(_ count: Int, of total: Int) -> String {
        "\(count) of \(total) documents"
    }
}

public struct SearchDocumentsIntent: AppIntent {
    public static let title: LocalizedStringResource = "Search documents"
    public static let description = IntentDescription(
        "Finds documents in Folio by what they say.", categoryName: "Documents",
        resultValueName: "Documents"
    )

    @Parameter(title: "Search for", requestValueDialog: "What should I search for?")
    public var query: String

    @Parameter(title: "Vault", description: "Where to search; your default vault when empty")
    public var vault: VaultEntity?

    public static var parameterSummary: some ParameterSummary {
        Summary("Search Folio for \(\.$query)") {
            \.$vault
        }
    }

    public init() {}

    public init(query: String, vault: VaultEntity? = nil) {
        self.query = query
        self.vault = vault
    }

    public func perform() async throws -> some IntentResult & ReturnsValue<[DocumentEntity]> & ProvidesDialog {
        IntentEnvironment.ran(.searchDocuments)
        let summaries = try await IntentEnvironment.run { try await $0.scoped(to: vault?.id).search(query) }
        let dialog: LocalizedStringResource =
            summaries.isEmpty
            ? "Nothing in Folio matches \(query)."
            : "Found \(countText(summaries.count, "document")) for \(query)."
        return .result(value: summaries.map(DocumentEntity.init), dialog: IntentDialog(dialog))
    }
}

public struct FindDocumentsIntent: AppIntent {
    public static let title: LocalizedStringResource = "Find documents"
    public static let description = IntentDescription(
        "Lists documents with a given tag, correspondent or type, or from a range of dates.",
        categoryName: "Documents", resultValueName: "Documents"
    )

    @Parameter(title: "Tag")
    public var tag: TagEntity?

    @Parameter(title: "Correspondent")
    public var correspondent: CorrespondentEntity?

    @Parameter(title: "Type")
    public var documentType: DocumentTypeEntity?

    @Parameter(title: "From", description: "First day to include")
    public var from: Date?

    @Parameter(title: "Through", description: "Last day to include")
    public var through: Date?

    @Parameter(title: "Vault", description: "Where to look; the vault of the tag, correspondent or type when empty")
    public var vault: VaultEntity?

    public static var parameterSummary: some ParameterSummary {
        Summary("Find documents in Folio") {
            \.$tag
            \.$correspondent
            \.$documentType
            \.$from
            \.$through
            \.$vault
        }
    }

    public init() {}

    public func perform() async throws -> some IntentResult & ReturnsValue<[DocumentEntity]> & ProvidesDialog {
        IntentEnvironment.ran(.findDocuments)
        let criteria = DocumentCriteria(
            tagID: tag?.tagID, correspondentID: correspondent?.correspondentID,
            documentTypeID: documentType?.documentTypeID, from: from, through: through
        )
        let implied = [tag?.id.vaultID, correspondent?.id.vaultID, documentType?.id.vaultID]
        let summaries = try await IntentEnvironment.run {
            try await $0.scoped(to: DocumentCatalog.vault(asked: vault?.id, implied: implied)).find(criteria)
        }
        return .result(
            value: summaries.map(DocumentEntity.init),
            dialog: summaries.isEmpty
                ? "No documents in Folio match."
                : "Found \(countText(summaries.count, "document"))."
        )
    }
}

public struct GetLatestDocumentIntent: AppIntent {
    public static let title: LocalizedStringResource = "Get the latest document"
    public static let description = IntentDescription(
        "Returns the document most recently added to Folio.", categoryName: "Documents",
        resultValueName: "Document"
    )

    @Parameter(title: "Vault", description: "Where to look; your default vault when empty")
    public var vault: VaultEntity?

    public static var parameterSummary: some ParameterSummary {
        Summary("Get the latest document") {
            \.$vault
        }
    }

    public init() {}

    public init(vault: VaultEntity?) {
        self.vault = vault
    }

    public func perform() async throws -> some IntentResult & ReturnsValue<DocumentEntity> & ProvidesDialog {
        IntentEnvironment.ran(.getLatestDocument)
        let latest = try await IntentEnvironment.run { try await $0.scoped(to: vault?.id).latest() }
        return .result(value: DocumentEntity(latest), dialog: "Your latest document is \(latest.title).")
    }
}

public struct OpenDocumentIntent: OpenIntent, PredictableIntent {
    public static let title: LocalizedStringResource = "Open a document"
    public static let description = IntentDescription(
        "Opens a document in Folio.", categoryName: "Documents"
    )
    public static let openAppWhenRun = true

    @Parameter(title: "Document", requestValueDialog: "Which document?")
    public var target: DocumentEntity

    // Spotlight offers an intent only when its summary shows every required parameter.
    public static var parameterSummary: some ParameterSummary {
        Summary("Open \(\.$target)")
    }

    public static var predictionConfiguration: some IntentPredictionConfiguration {
        IntentPrediction(parameters: (\Self.$target)) { target in
            DisplayRepresentation(title: "Open \(target.title)")
        }
    }

    public init() {}

    public init(target: DocumentEntity) {
        self.target = target
    }

    @MainActor
    public func perform() async throws -> some IntentResult {
        IntentEnvironment.ran(.openDocument)
        IntentRouter.shared.route(.document(target.documentID, vaultID: target.id.vaultID))
        return .result()
    }
}

public struct AddTagIntent: AppIntent {
    public static let title: LocalizedStringResource = "Add a tag to a document"
    public static let description = IntentDescription(
        "Puts a tag on a document in Folio.", categoryName: "Documents"
    )

    @Parameter(title: "Document")
    public var document: DocumentEntity

    @Parameter(title: "Tag")
    public var tag: TagEntity

    public static var parameterSummary: some ParameterSummary {
        Summary("Add \(\.$tag) to \(\.$document)")
    }

    public init() {}

    public init(document: DocumentEntity, tag: TagEntity) {
        self.document = document
        self.tag = tag
    }

    public func perform() async throws -> some IntentResult & ProvidesDialog {
        IntentEnvironment.ran(.addTag)
        let outcome = try await IntentEnvironment.run {
            try await $0.apply(TagChange(.add, tag: tag, document: document))
        }
        await registerUndo(of: outcome.applied)
        guard outcome.applied.documents.isEmpty else {
            return .result(dialog: "Tagged \(document.title) with \(tag.name).")
        }
        return .result(dialog: "\(document.title) already has \(tag.name).")
    }
}

public struct RemoveTagIntent: AppIntent {
    public static let title: LocalizedStringResource = "Remove a tag from a document"
    public static let description = IntentDescription(
        "Takes a tag off a document in Folio.", categoryName: "Documents"
    )

    @Parameter(title: "Document")
    public var document: DocumentEntity

    @Parameter(title: "Tag")
    public var tag: TagEntity

    public static var parameterSummary: some ParameterSummary {
        Summary("Remove \(\.$tag) from \(\.$document)")
    }

    public init() {}

    public init(document: DocumentEntity, tag: TagEntity) {
        self.document = document
        self.tag = tag
    }

    public func perform() async throws -> some IntentResult & ProvidesDialog {
        IntentEnvironment.ran(.removeTag)
        let outcome = try await IntentEnvironment.run {
            try await $0.apply(TagChange(.remove, tag: tag, document: document))
        }
        await registerUndo(of: outcome.applied)
        guard outcome.applied.documents.isEmpty else {
            return .result(dialog: "Removed \(tag.name) from \(document.title).")
        }
        return .result(dialog: "\(document.title) doesn't have \(tag.name).")
    }
}

@available(iOS 26, macOS 26, *)
extension AddTagIntent: UndoableIntent {}

extension AddTagIntent {
    @MainActor private func registerUndo(of change: TagChange) {
        if #available(iOS 26, macOS 26, *) {
            TagUndo.register(change, with: undoManager, revert: TagUndo.revert)
        }
    }
}

@available(iOS 26, macOS 26, *)
extension RemoveTagIntent: UndoableIntent {}

extension RemoveTagIntent {
    @MainActor private func registerUndo(of change: TagChange) {
        if #available(iOS 26, macOS 26, *) {
            TagUndo.register(change, with: undoManager, revert: TagUndo.revert)
        }
    }
}

extension TagChange {
    /// One document and a tag that must come from the same vault.
    init(_ kind: Kind, tag: TagEntity, document: DocumentEntity) throws {
        _ = try DocumentCatalog.vault(asked: document.id.vaultID, implied: [tag.id.vaultID])
        self.init(kind, tagID: tag.tagID, tagVaultID: tag.id.vaultID, documents: [document.id])
    }
}

public struct GetDocumentFileIntent: AppIntent {
    public static let title: LocalizedStringResource = "Get a document's file"
    public static let description = IntentDescription(
        "Returns the original PDF or image, so Shortcuts can send it to other apps.",
        categoryName: "Documents", resultValueName: "File"
    )

    @Parameter(title: "Document")
    public var document: DocumentEntity

    public static var parameterSummary: some ParameterSummary {
        Summary("Get the file of \(\.$document)")
    }

    public init() {}

    public init(document: DocumentEntity) {
        self.document = document
    }

    public func perform() async throws -> some IntentResult & ReturnsValue<IntentFile> {
        IntentEnvironment.ran(.getDocumentFile)
        let (url, original) = try await IntentEnvironment.run { try await $0.file(for: document.id) }
        let type = original.mimeType.flatMap { UTType(mimeType: $0) }
        return .result(value: IntentFile(fileURL: url, filename: url.lastPathComponent, type: type))
    }
}

extension UploadFile {
    init(intentFile file: IntentFile) throws {
        let name = file.filename
        guard !file.data.isEmpty else { throw ShortcutFailure.unreadableFile(name) }
        self.init(
            data: file.data, filename: name,
            mimeType: file.type?.preferredMIMEType ?? UploadFile.mimeType(forFilename: name)
        )
    }
}

public enum FolioIntentsPackage: AppIntentsPackage {}
