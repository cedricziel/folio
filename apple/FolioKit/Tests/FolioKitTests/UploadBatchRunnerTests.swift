import Foundation
import Testing

@testable import FolioKit

private func file(_ name: String) -> UploadFile {
    UploadFile(data: Data(name.utf8), filename: name, mimeType: "application/pdf")
}

private let first = Tag(name: "Insurance")
private let second = Tag(name: "Taxes")

struct UploadBatchRunnerTests {
    @Test func uploadSucceedsTaggingFailsThenRetryAttachesOnlyTheRemainingTag() async throws {
        let service = MockFolioService()
        let store = InMemoryUploadProgressStore()
        let runner = UploadBatchRunner(service: service, store: store)
        let files = [file("a.pdf")]
        await service.setFailingTags([second.id])

        await #expect(throws: FolioError.server(status: 500, message: nil)) {
            _ = try await runner.run(files, tagIDs: [first.id, second.id], scope: "s")
        }
        let stopped = try #require(runner.progress(files: files, tagIDs: [first.id, second.id], scope: "s"))
        #expect(stopped.uploaded == 1)
        #expect(stopped.remainingTags == 1)

        await service.setFailingTags([])
        let documents = try await runner.run(files, tagIDs: [first.id, second.id], scope: "s")

        #expect(await service.uploads.map(\.filename) == ["a.pdf"])
        let tagged = await service.tagged
        #expect(tagged.map(\.tag) == [first.id, second.id])
        #expect(Set(tagged.map(\.document)) == [try #require(documents.first?.id)])
        #expect(runner.progress(files: files, tagIDs: [first.id, second.id], scope: "s") == nil)
    }

    @Test func secondFileFailsThenRetryUploadsOnlyThatFile() async throws {
        let service = MockFolioService()
        let runner = UploadBatchRunner(service: service, store: InMemoryUploadProgressStore())
        let files = [file("a.pdf"), file("b.pdf")]
        await service.setFailUploadsAfter(1)

        await #expect(throws: FolioError.unreachable) {
            _ = try await runner.run(files, tagIDs: [first.id], scope: "s")
        }
        #expect(runner.progress(files: files, tagIDs: [first.id], scope: "s")?.uploaded == 1)

        await service.setFailUploadsAfter(nil)
        let documents = try await runner.run(files, tagIDs: [first.id], scope: "s")

        #expect(await service.uploads.map(\.filename) == ["a.pdf", "b.pdf"])
        #expect(documents.map(\.originalFilename) == ["a.pdf", "b.pdf"])
        #expect(await service.tagged.map(\.document) == documents.map(\.id))
    }

    @Test func lostUploadAnswerIsFoundOnTheServerInsteadOfUploadedAgain() async throws {
        let service = MockFolioService()
        let runner = UploadBatchRunner(service: service, store: InMemoryUploadProgressStore())
        let files = [file("a.pdf")]
        await service.setLoseUploadResponses(true)

        await #expect(throws: FolioError.unreachable) {
            _ = try await runner.run(files, tagIDs: [first.id], scope: "s")
        }
        let stored = try #require(await service.documents(matching: "").first)

        await service.setLoseUploadResponses(false)
        let documents = try await runner.run(files, tagIDs: [first.id], scope: "s")

        #expect(await service.uploads.count == 1)
        #expect(documents.map(\.id) == [stored.id])
        #expect(await service.tagged.map(\.document) == [stored.id])
    }

    @Test func lostUploadThatNeverReachedTheServerIsSentAgain() async throws {
        let service = MockFolioService()
        let runner = UploadBatchRunner(service: service, store: InMemoryUploadProgressStore())
        await service.setFailure(.unreachable)
        await #expect(throws: FolioError.unreachable) {
            _ = try await runner.run([file("a.pdf")], tagIDs: [first.id], scope: "s")
        }

        await service.setFailure(nil)
        _ = try await runner.run([file("a.pdf")], tagIDs: [first.id], scope: "s")

        #expect(await service.filters.count == 1)
        #expect(await service.uploads.map(\.filename) == ["a.pdf"])
    }

    @Test func serverRefusalIsNotLookedUpBeforeUploadingAgain() async throws {
        let service = MockFolioService()
        let runner = UploadBatchRunner(service: service, store: InMemoryUploadProgressStore())
        await service.setFailure(.server(status: 503, message: nil))
        _ = try? await runner.run([file("a.pdf")], tagIDs: [first.id], scope: "s")

        await service.setFailure(nil)
        _ = try await runner.run([file("a.pdf")], tagIDs: [first.id], scope: "s")

        #expect(await service.filters.isEmpty)
        #expect(await service.uploads.count == 1)
    }

    @Test func aDifferentBatchDoesNotReuseProgress() async throws {
        let service = MockFolioService()
        let runner = UploadBatchRunner(service: service, store: InMemoryUploadProgressStore())
        await service.setFailUploadsAfter(1)
        _ = try? await runner.run([file("a.pdf"), file("b.pdf")], tagIDs: [first.id], scope: "s")
        await service.setFailUploadsAfter(nil)

        _ = try await runner.run([file("a.pdf"), file("b.pdf")], tagIDs: [second.id], scope: "s")

        #expect(await service.uploads.map(\.filename) == ["a.pdf", "a.pdf", "b.pdf"])
    }

    @Test func keysSeparateServersTagsAndContents() {
        let base = UploadBatchRunner.key(files: [file("a.pdf")], tagIDs: [first.id], scope: "s")
        #expect(base == UploadBatchRunner.key(files: [file("a.pdf")], tagIDs: [first.id], scope: "s"))
        #expect(base != UploadBatchRunner.key(files: [file("a.pdf")], tagIDs: [first.id], scope: "other"))
        #expect(base != UploadBatchRunner.key(files: [file("a.pdf")], tagIDs: [second.id], scope: "s"))
        var changed = file("a.pdf")
        changed.data = Data("different".utf8)
        #expect(base != UploadBatchRunner.key(files: [changed], tagIDs: [first.id], scope: "s"))
    }

    @Test func fileStoreForgetsStaleProgress() throws {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "folio-progress-\(UUID().uuidString)", directoryHint: .isDirectory)
        defer { try? FileManager.default.removeItem(at: directory) }
        let progress = UploadBatchProgress(
            steps: [UploadStep(documentID: UUID(), remainingTags: [first.id])], updatedAt: .now)
        FileUploadProgressStore(directory: directory).save(progress, for: "batch")

        #expect(FileUploadProgressStore(directory: directory).load("batch") == progress)
        #expect(FileUploadProgressStore(directory: directory, maxAge: -1).load("batch") == nil)
        #expect(FileUploadProgressStore(directory: directory).load("batch") == nil)
    }
}

struct ShortcutUploadRetryTests {
    private func catalog(_ service: MockFolioService, store: any UploadProgressStoring) -> DocumentCatalog {
        DocumentCatalog(service: service, progress: store, uploadScope: "https://folio.example.com")
    }

    /// Each Shortcut run builds a new catalog; the progress file carries over between them.
    @Test func rerunningAShortcutFinishesTaggingWithoutUploadingAgain() async throws {
        let directory = FileManager.default.temporaryDirectory
            .appending(path: "folio-progress-\(UUID().uuidString)", directoryHint: .isDirectory)
        defer { try? FileManager.default.removeItem(at: directory) }
        let service = MockFolioService()
        await service.setFailingTags([second.id])

        await #expect(throws: FolioError.self) {
            _ = try await catalog(service, store: FileUploadProgressStore(directory: directory))
                .upload([file("a.pdf")], title: "Policy", tagIDs: [first.id, second.id])
        }
        await service.setFailingTags([])
        let summaries = try await catalog(service, store: FileUploadProgressStore(directory: directory))
            .upload([file("a.pdf")], title: "Policy", tagIDs: [first.id, second.id])

        #expect(await service.uploads.map(\.title) == ["Policy"])
        #expect(await service.tagged.map(\.tag) == [first.id, second.id])
        #expect(summaries.count == 1)
    }

    @Test func rerunningAShortcutUploadsOnlyTheFilesThatFailed() async throws {
        let service = MockFolioService()
        let store = InMemoryUploadProgressStore()
        await service.setFailUploadsAfter(1)
        await #expect(throws: FolioError.unreachable) {
            _ = try await catalog(service, store: store).upload([file("a.pdf"), file("b.pdf")], tagIDs: [first.id])
        }

        await service.setFailUploadsAfter(nil)
        let summaries = try await catalog(service, store: store)
            .upload([file("a.pdf"), file("b.pdf")], tagIDs: [first.id])

        #expect(await service.uploads.map(\.filename) == ["a.pdf", "b.pdf"])
        #expect(await service.tagged.count == 2)
        #expect(summaries.count == 2)
    }
}

/// Through the real HTTP client: the upload is accepted, the tag request fails, and the retry only tags.
@Suite(.timeLimit(.minutes(1))) struct UploadBatchRunnerHTTPTests {
    private let documentID = "3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f"

    private let documentJSON = """
        {"id": "3f2c1b2e-5b0a-4c53-9d6e-0a1b2c3d4e5f", "title": "Jahresabrechnung 2025",
         "vault_id": "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee", "uploaded_by": "fixture-user",
         "created_by": "archive-creator", "updated_by": "archive-editor",
         "content": "Rechnungsnummer 2025-00318", "original_filename": "bill.pdf",
         "file_path": "a/bill.pdf", "mime_type": "application/pdf", "correspondent_id": null,
         "document_type_id": "11111111-2222-4333-8444-555555555555", "page_count": 3, "file_size": 48213,
         "ocr_status": "done", "embed_status": "skipped",
         "correspondent": null, "document_type": "Invoice",
         "tags": [], "created_at": "2025-03-14T10:15:30.123456789Z", "updated_at": "2025-03-14T10:15:30Z"}
        """

    @Test func tagFailureRetrySendsOnlyTheTagRequest() async throws {
        let host = "batch-\(UUID().uuidString.lowercased()).example.com"
        ShareUploadProtocol.serve(host, [.json(201, documentJSON), .status(500), .status(204)])
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ShareUploadProtocol.self]
        let service = LiveFolioService(
            baseURL: URL(string: "https://\(host)")!, tokenProvider: RotatingTokens(),
            session: URLSession(configuration: configuration)
        )
        let runner = UploadBatchRunner(service: service, store: InMemoryUploadProgressStore())
        let files = [file("a.pdf")]

        await #expect(throws: FolioError.self) { _ = try await runner.run(files, tagIDs: [first.id], scope: host) }
        let documents = try await runner.run(files, tagIDs: [first.id], scope: host)

        let tags = "/api/documents/\(documentID)/tags"
        let paths = ShareUploadProtocol.requests(to: host).map { $0.url?.path() ?? "" }
        #expect(Array(paths.prefix(3)) == ["/api/documents/upload", tags, tags])
        #expect(!paths.dropFirst(3).contains("/api/documents/upload"))
        #expect(documents.map(\.id.uuidString).map { $0.lowercased() } == [documentID])
    }
}
