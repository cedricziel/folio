import AppIntents
import CoreSpotlight
import Foundation
import UniformTypeIdentifiers

private func countText(_ count: Int, _ noun: String) -> String {
    "\(count) \(noun)\(count == 1 ? "" : "s")"
}

public struct ScanDocumentIntent: AppIntent {
    public static let title: LocalizedStringResource = "Scan a document"
    public static let description = IntentDescription(
        "Opens Folio's scanner so you can photograph a paper document and file it.")
    public static let openAppWhenRun = true

    public init() {}

    @MainActor
    public func perform() async throws -> some IntentResult {
        IntentRouter.shared.route(.scan)
        return .result()
    }
}

public struct UploadFileIntent: AppIntent {
    public static let title: LocalizedStringResource = "Upload to Folio"
    public static let description = IntentDescription(
        "Files PDFs and images in Folio. Folio reads the text for you.",
        categoryName: "Documents", resultValueName: "Uploaded documents")

    @Parameter(
        title: "Files", description: "PDFs or images to file",
        supportedContentTypes: [.pdf, .image])
    public var files: [IntentFile]

    @Parameter(title: "Title", description: "Used when you upload a single file")
    public var title: String?

    @Parameter(title: "Tags")
    public var tags: [TagEntity]

    public static var parameterSummary: some ParameterSummary {
        Summary("Upload \(\.$files) to Folio") {
            \.$title
            \.$tags
        }
    }

    public init() {}

    public init(files: [IntentFile], title: String? = nil, tags: [TagEntity] = []) {
        self.files = files
        self.title = title
        self.tags = tags
    }

    public func perform() async throws -> some IntentResult & ReturnsValue<[DocumentEntity]> & ProvidesDialog {
        let summaries = try await IntentEnvironment.run {
            try await $0.upload(files.map { try UploadFile(intentFile: $0) }, title: title, tagIDs: tags.map(\.id))
        }
        return .result(
            value: summaries.map(DocumentEntity.init),
            dialog: "Filed \(countText(summaries.count, "document")) in Folio.")
    }
}

public struct SearchDocumentsIntent: AppIntent {
    public static let title: LocalizedStringResource = "Search documents"
    public static let description = IntentDescription(
        "Finds documents in Folio by what they say.", categoryName: "Documents",
        resultValueName: "Documents")

    @Parameter(title: "Search for", requestValueDialog: "What should I search for?")
    public var query: String

    public static var parameterSummary: some ParameterSummary {
        Summary("Search Folio for \(\.$query)")
    }

    public init() {}

    public init(query: String) {
        self.query = query
    }

    public func perform() async throws -> some IntentResult & ReturnsValue<[DocumentEntity]> & ProvidesDialog {
        let summaries = try await IntentEnvironment.run { try await $0.search(query) }
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
        categoryName: "Documents", resultValueName: "Documents")

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

    public static var parameterSummary: some ParameterSummary {
        Summary("Find documents in Folio") {
            \.$tag
            \.$correspondent
            \.$documentType
            \.$from
            \.$through
        }
    }

    public init() {}

    public func perform() async throws -> some IntentResult & ReturnsValue<[DocumentEntity]> & ProvidesDialog {
        let criteria = DocumentCriteria(
            tagID: tag?.id, correspondentID: correspondent?.id, documentTypeID: documentType?.id,
            from: from, through: through)
        let summaries = try await IntentEnvironment.run { try await $0.find(criteria) }
        return .result(
            value: summaries.map(DocumentEntity.init),
            dialog: summaries.isEmpty
                ? "No documents in Folio match."
                : "Found \(countText(summaries.count, "document")).")
    }
}

public struct GetLatestDocumentIntent: AppIntent {
    public static let title: LocalizedStringResource = "Get the latest document"
    public static let description = IntentDescription(
        "Returns the document most recently added to Folio.", categoryName: "Documents",
        resultValueName: "Document")

    public init() {}

    public func perform() async throws -> some IntentResult & ReturnsValue<DocumentEntity> & ProvidesDialog {
        let latest = try await IntentEnvironment.run { try await $0.latest() }
        return .result(value: DocumentEntity(latest), dialog: "Your latest document is \(latest.title).")
    }
}

public struct OpenDocumentIntent: OpenIntent {
    public static let title: LocalizedStringResource = "Open a document"
    public static let description = IntentDescription(
        "Opens a document in Folio.", categoryName: "Documents")
    public static let openAppWhenRun = true

    @Parameter(title: "Document")
    public var target: DocumentEntity

    public init() {}

    public init(target: DocumentEntity) {
        self.target = target
    }

    @MainActor
    public func perform() async throws -> some IntentResult {
        IntentRouter.shared.route(.document(target.id))
        return .result()
    }
}

public struct AddTagIntent: AppIntent {
    public static let title: LocalizedStringResource = "Add a tag to a document"
    public static let description = IntentDescription(
        "Puts a tag on a document in Folio.", categoryName: "Documents")

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
        try await IntentEnvironment.run { try await $0.addTag(tag.id, to: document.id) }
        return .result(dialog: "Tagged \(document.title) with \(tag.name).")
    }
}

public struct RemoveTagIntent: AppIntent {
    public static let title: LocalizedStringResource = "Remove a tag from a document"
    public static let description = IntentDescription(
        "Takes a tag off a document in Folio.", categoryName: "Documents")

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
        try await IntentEnvironment.run { try await $0.removeTag(tag.id, from: document.id) }
        return .result(dialog: "Removed \(tag.name) from \(document.title).")
    }
}

public struct GetDocumentFileIntent: AppIntent {
    public static let title: LocalizedStringResource = "Get a document's file"
    public static let description = IntentDescription(
        "Returns the original PDF or image, so Shortcuts can send it to other apps.",
        categoryName: "Documents", resultValueName: "File")

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
            mimeType: file.type?.preferredMIMEType ?? UploadFile.mimeType(forFilename: name))
    }
}

public enum FolioIntentsPackage: AppIntentsPackage {}

/// Re-indexes a document in Spotlight when someone opens it.
public enum SpotlightDonor {
    public static func donate(_ summary: DocumentSummary) async {
        try? await CSSearchableIndex.default().indexAppEntities([DocumentEntity(summary)])
    }
}
