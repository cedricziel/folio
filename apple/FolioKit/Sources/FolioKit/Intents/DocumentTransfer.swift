import AppIntents
import CoreTransferable
import Foundation
import UniformTypeIdentifiers

/// How a document's original file is typed when it leaves Folio.
public enum DocumentTransfer {
    /// The file types offered by name, so receivers can pick a document by what it is. Anything else
    /// goes out as plain data.
    public static let fileTypes: [UTType] = [.pdf, .png, .jpeg, .heic, .tiff]

    public static func contentType(mimeType: String?, filename: String?) -> UTType {
        if let mimeType, let type = UTType(mimeType: mimeType.lowercased()), type != .data {
            return type
        }
        let ext = (filename.map { ($0 as NSString).pathExtension } ?? "").lowercased()
        if !ext.isEmpty, let type = UTType(filenameExtension: ext) {
            return type
        }
        return .data
    }

    /// The declared export type for a document whose content has `type`.
    public static func exportedFileType(for type: UTType) -> UTType {
        fileTypes.first { type.conforms(to: $0) } ?? .data
    }
}

extension DocumentEntity: Transferable {
    /// The original file first, then the text Folio read from it, then a link to it in the web app.
    /// Every download goes through the signed-in session and the vault the document was opened from.
    public static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(exportedContentType: .pdf, exporting: exportFile).exportingCondition { $0.exports(.pdf) }
        FileRepresentation(exportedContentType: .png, exporting: exportFile).exportingCondition { $0.exports(.png) }
        FileRepresentation(exportedContentType: .jpeg, exporting: exportFile).exportingCondition { $0.exports(.jpeg) }
        FileRepresentation(exportedContentType: .heic, exporting: exportFile).exportingCondition { $0.exports(.heic) }
        FileRepresentation(exportedContentType: .tiff, exporting: exportFile).exportingCondition { $0.exports(.tiff) }
        FileRepresentation(exportedContentType: .data, exporting: exportFile).exportingCondition { $0.exports(.data) }
        DataRepresentation(exportedContentType: .utf8PlainText, exporting: exportText)
            .exportingCondition { $0.hasRecognizedText }
        ProxyRepresentation(exporting: webLink)
    }

    func exports(_ type: UTType) -> Bool {
        DocumentTransfer.exportedFileType(for: contentType) == type
    }

    @Sendable private static func exportFile(_ entity: DocumentEntity) async throws -> SentTransferredFile {
        let file = try await IntentEnvironment.run { try await $0.transferFile(for: entity.id) }
        return SentTransferredFile(file.url)
    }

    @Sendable private static func exportText(_ entity: DocumentEntity) async throws -> Data {
        Data(try await IntentEnvironment.run { try await $0.recognizedText(for: entity.id) }.utf8)
    }

    @Sendable private static func webLink(_ entity: DocumentEntity) throws -> URL {
        guard let url = DocumentLocator.shared.webURL(for: entity.id) else { throw ShortcutError.notSignedIn }
        return url
    }
}
