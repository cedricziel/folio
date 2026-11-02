import Foundation
import UniformTypeIdentifiers

/// Reads provider-owned files before their temporary URLs expire and reports every failed selection.
public enum SharedAttachmentLoader {
    public struct Failure: Sendable, Equatable {
        public let name: String
        public let message: String
    }

    public struct Result: Sendable {
        public var files: [UploadFile] = []
        public var failures: [Failure] = []
        /// Upload is allowed only when the entire nonempty selection was read.
        public var canUpload: Bool { !files.isEmpty && failures.isEmpty }
    }

    /// Loads every provider, retaining successful files and named failures in selection order.
    @MainActor public static func load(_ providers: [NSItemProvider]) async -> Result {
        var result = Result()
        for (index, provider) in providers.enumerated() {
            let name = basename(provider.suggestedName) ?? "Attachment \(index + 1)"
            guard
                let type = [UTType.pdf, .image, .data].first(where: {
                    provider.hasItemConformingToTypeIdentifier($0.identifier)
                })
            else {
                result.failures.append(Failure(name: name, message: "This attachment type is not supported."))
                continue
            }
            do {
                let suggestion = provider.suggestedName
                let file = try await withCheckedThrowingContinuation {
                    (continuation: CheckedContinuation<UploadFile, any Error>) in
                    _ = provider.loadFileRepresentation(forTypeIdentifier: type.identifier) { url, error in
                        guard let url else {
                            continuation.resume(throwing: error ?? FolioError.unreadableResponse)
                            return
                        }
                        do {
                            // Provider URLs expire when this callback returns; retain the bytes, not the path.
                            var file = try UploadFile(contentsOf: url)
                            let original = basename(url.lastPathComponent) ?? "Attachment"
                            var filename = basename(suggestion) ?? original
                            let ext = (original as NSString).pathExtension
                            if !ext.isEmpty && !filename.lowercased().hasSuffix(".\(ext.lowercased())") {
                                filename += ".\(ext)"
                            }
                            file.filename = filename
                            continuation.resume(returning: file)
                        } catch {
                            continuation.resume(throwing: error)
                        }
                    }
                }
                result.files.append(file)
            } catch {
                result.failures.append(
                    Failure(name: name, message: "This attachment could not be read. Try sharing it again."))
            }
        }
        return result
    }

    private static func basename(_ value: String?) -> String? {
        guard let value else { return nil }
        let name =
            value.replacingOccurrences(of: "\\", with: "/")
            .components(separatedBy: "/").last?
            .components(separatedBy: .controlCharacters).joined()
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return name.isEmpty || name == "." || name == ".." ? nil : name
    }
}
