import Foundation
import Testing

@testable import FolioKit

struct DocumentImportTests {
    @Test func importsSupportedFilesAndReportsRejectedFiles() async throws {
        let directory = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let pdf = directory.appending(path: "letter.pdf")
        let image = directory.appending(path: "photo.png")
        let text = directory.appending(path: "notes.txt")
        let folder = directory.appending(path: "folder.pdf")
        try Data("%PDF".utf8).write(to: pdf)
        try Data([137, 80, 78, 71]).write(to: image)
        try Data("notes".utf8).write(to: text)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)

        let result = await UploadFile.readDocuments([
            pdf, text, image, folder, directory.appending(path: "missing.pdf"),
        ])

        #expect(result.files.map(\.filename) == ["letter.pdf", "photo.png"])
        #expect(result.files.first?.data == Data("%PDF".utf8))
        #expect(result.failed == ["notes.txt", "folder.pdf", "missing.pdf"])
    }

    @Test func rejectsRemoteURLs() async throws {
        let url = try #require(URL(string: "https://example.com/letter.pdf"))
        let result = await UploadFile.readDocuments([url])
        #expect(result.files.isEmpty)
        #expect(result.failed == ["letter.pdf"])
    }
}
