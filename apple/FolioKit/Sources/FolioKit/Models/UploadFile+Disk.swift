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

    static func mimeType(forFilename filename: String) -> String {
        let ext = (filename as NSString).pathExtension
        return UTType(filenameExtension: ext)?.preferredMIMEType ?? "application/octet-stream"
    }
}
