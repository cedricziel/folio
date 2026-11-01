import Foundation
import Testing

@testable import FolioKit

struct ShareUploadRequestTests {
    private func directory() throws -> URL {
        let url = FileManager.default.temporaryDirectory
            .appending(path: UUID().uuidString, directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    @Test func buildsAMultipartPostFromAFileOnDisk() throws {
        let file = UploadFile(
            data: Data("%PDF-1.4".utf8), filename: "scan.pdf", mimeType: "application/pdf", title: "Scan"
        )
        let built = try ShareUploadRequest.make(
            file: file, serverURL: #require(URL(string: "https://folio.example.com/base")), token: "secret",
            directory: directory()
        )

        #expect(built.request.httpMethod == "POST")
        #expect(built.request.url?.absoluteString == "https://folio.example.com/base/api/documents/upload")
        #expect(built.request.value(forHTTPHeaderField: "Authorization") == "Bearer secret")
        let contentType = try #require(built.request.value(forHTTPHeaderField: "Content-Type"))
        #expect(contentType.hasPrefix("multipart/form-data; boundary="))

        let body = try String(decoding: Data(contentsOf: built.bodyFile), as: UTF8.self)
        #expect(body.contains("name=\"file\"; filename=\"scan.pdf\""))
        #expect(body.contains("Content-Type: application/pdf"))
        #expect(body.contains("name=\"title\""))
        #expect(body.contains(contentType.replacingOccurrences(of: "multipart/form-data; boundary=", with: "")))
    }

    @Test func omitsAuthorizationWithoutAToken() throws {
        let file = UploadFile(data: Data(), filename: "a.png", mimeType: "image/png")
        let built = try ShareUploadRequest.make(
            file: file, serverURL: #require(URL(string: "http://localhost:3000")), token: nil, directory: directory()
        )
        #expect(built.request.value(forHTTPHeaderField: "Authorization") == nil)
    }
}

@MainActor
struct ShareViewModelTests {
    final class RecordingUploader: BackgroundUploading, @unchecked Sendable {
        var enqueued: [UploadFile] = []
        func enqueue(_ file: UploadFile) throws {
            enqueued.append(file)
        }
    }

    private func file(_ name: String = "a.pdf") -> UploadFile {
        UploadFile(data: Data(name.utf8), filename: name, mimeType: "application/pdf")
    }

    @Test func prefillsTheTitleFromASingleFile() {
        let model = ShareViewModel(
            files: [file("Bill.pdf")], service: MockFolioService(), background: RecordingUploader())
        #expect(model.title == "Bill")
        let many = ShareViewModel(
            files: [file("a.pdf"), file("b.pdf")], service: MockFolioService(), background: RecordingUploader()
        )
        #expect(many.title.isEmpty)
    }

    @Test func loadsTagsForThePicker() async {
        let service = MockFolioService()
        let tag = Tag(name: "Insurance")
        await service.setTags([tag])
        let model = ShareViewModel(files: [file()], service: service, background: RecordingUploader())
        await model.loadTags()
        #expect(model.tags == [tag])
    }

    @Test func uploadWithoutTagsHandsOffToTheBackgroundSession() async {
        let background = RecordingUploader()
        let service = MockFolioService()
        let model = ShareViewModel(files: [file()], service: service, background: background)
        await model.upload()
        #expect(model.state == .done)
        #expect(background.enqueued.count == 1)
        #expect(await service.uploads.isEmpty)
    }

    @Test func uploadWithTagsRunsInTheForegroundAndAttachesThem() async {
        let background = RecordingUploader()
        let service = MockFolioService()
        let tag = Tag(name: "Insurance")
        await service.setTags([tag])
        let model = ShareViewModel(files: [file()], service: service, background: background)
        await model.loadTags()
        model.selectedTagIDs = [tag.id]
        model.title = "Policy"
        await model.upload()

        #expect(model.state == .done)
        #expect(background.enqueued.isEmpty)
        let uploads = await service.uploads
        #expect(uploads.map(\.title) == ["Policy"])
        #expect(await service.tagged.map(\.tag) == [tag.id])
    }

    @Test func foregroundFailureIsReported() async {
        let service = MockFolioService()
        await service.setFailure(.unreachable)
        let model = ShareViewModel(files: [file()], service: service, background: RecordingUploader())
        model.selectedTagIDs = [UUID()]
        await model.upload()
        #expect(model.state == .failed("Couldn't reach the server."))
    }
}
