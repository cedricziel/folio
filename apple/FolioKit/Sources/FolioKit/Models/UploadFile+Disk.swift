import Foundation
import UniformTypeIdentifiers

public extension UploadFile {
    /// Reads a file picked from Files, Photos or the share sheet.
    init(contentsOf url: URL) throws {
        let scoped = url.startAccessingSecurityScopedResource()
        defer {
            if scoped {
                url.stopAccessingSecurityScopedResource()
            }
        }
        try self.init(
            data: Data(contentsOf: url), filename: url.lastPathComponent,
            mimeType: Self.mimeType(forFilename: url.lastPathComponent)
        )
    }

    /// Reads local PDFs and images off the main actor, retaining readable files when others fail.
    static func readDocuments(_ urls: [URL]) async -> (files: [UploadFile], failed: [String]) {
        await Task.detached {
            var files: [UploadFile] = []
            var failed: [String] = []
            for url in urls {
                let scoped = url.startAccessingSecurityScopedResource()
                defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                guard url.isFileURL,
                    let type = UTType(filenameExtension: url.pathExtension),
                    type.conforms(to: .pdf) || type.conforms(to: .image),
                    let values = try? url.resourceValues(forKeys: [.isRegularFileKey]),
                    values.isRegularFile == true,
                    let file = try? UploadFile(contentsOf: url)
                else {
                    failed.append(url.lastPathComponent)
                    continue
                }
                files.append(file)
            }
            return (files, failed)
        }.value
    }

    static func mimeType(forFilename filename: String) -> String {
        let ext = (filename as NSString).pathExtension
        return UTType(filenameExtension: ext)?.preferredMIMEType ?? "application/octet-stream"
    }
}
