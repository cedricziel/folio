import Foundation
import Testing
import UniformTypeIdentifiers

@testable import FolioKit

@MainActor
struct SharedAttachmentLoaderTests {
    private func provider(_ url: URL, suggestion: String?) -> NSItemProvider {
        let provider = NSItemProvider()
        provider.suggestedName = suggestion
        provider.registerFileRepresentation(forTypeIdentifier: UTType.pdf.identifier, visibility: .all) { completion in
            completion(url, false, nil)
            return nil
        }
        return provider
    }

    private func failedProvider() -> NSItemProvider {
        let provider = NSItemProvider()
        provider.suggestedName = "Unreadable.pdf"
        provider.registerFileRepresentation(forTypeIdentifier: UTType.pdf.identifier, visibility: .all) { completion in
            completion(nil, false, CocoaError(.fileReadNoPermission))
            return nil
        }
        return provider
    }

    @Test(
        arguments: [
            (nil, "Invoice.pdf"), ("Invoice", "Invoice.pdf"), ("Invoice.pdf", "Invoice.pdf"),
            ("../Invoice.pdf", "Invoice.pdf"), ("folder\\Invoice.pdf", "Invoice.pdf"),
            ("Invoice.v2", "Invoice.v2.pdf"), ("Invoice.PDF", "Invoice.PDF"),
            ("..", "Invoice.pdf"), ("", "Invoice.pdf"), ("Invoice\n.pdf", "Invoice.pdf"),
        ] as [(String?, String)])
    func preservesSafeNamesAndMIME(suggestion: String?, expected: String) async throws {
        let directory = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appending(path: "Invoice.pdf")
        try Data("%PDF-fixture".utf8).write(to: url)

        let result = await SharedAttachmentLoader.load([provider(url, suggestion: suggestion)])

        #expect(result.canUpload)
        #expect(result.files.first?.filename == expected)
        #expect(result.files.first?.mimeType == "application/pdf")
        #expect(result.files.first?.data == Data("%PDF-fixture".utf8))
        #expect(result.failures.isEmpty)
        let model = ShareViewModel(
            files: result.files, service: MockFolioService(),
            background: ShareViewModelTests.RecordingUploader()
        )
        #expect(model.title == (expected as NSString).deletingPathExtension)
    }

    @Test func mixedSelectionRetainsSuccessAndNamedFailureButBlocksUpload() async throws {
        let directory = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appending(path: "Invoice.pdf")
        try Data("%PDF-fixture".utf8).write(to: url)

        let result = await SharedAttachmentLoader.load([provider(url, suggestion: nil), failedProvider()])

        #expect(result.files.count == 1)
        #expect(result.failures.map(\.name) == ["Unreadable.pdf"])
        #expect(!result.canUpload)
    }

    @Test func retryReadsFilesThatBecomeAvailable() async throws {
        let directory = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appending(path: "Invoice.pdf")
        let attachment = provider(url, suggestion: nil)
        let first = await SharedAttachmentLoader.load([attachment])
        #expect(!first.canUpload)

        try Data("%PDF-recovered".utf8).write(to: url)
        let retried = await SharedAttachmentLoader.load([attachment])
        #expect(retried.canUpload)
        #expect(retried.failures.isEmpty)
        #expect(retried.files.first?.data == Data("%PDF-recovered".utf8))
    }

    @Test func identicalNamesKeepDistinctContents() async throws {
        let directory = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let first = directory.appending(path: "first.pdf")
        let second = directory.appending(path: "second.pdf")
        try Data("first".utf8).write(to: first)
        try Data("second".utf8).write(to: second)

        let result = await SharedAttachmentLoader.load([
            provider(first, suggestion: "Invoice.pdf"), provider(second, suggestion: "Invoice.pdf"),
        ])

        #expect(result.canUpload)
        #expect(result.files.map(\.filename) == ["Invoice.pdf", "Invoice.pdf"])
        #expect(result.files.map(\.data) == [Data("first".utf8), Data("second".utf8)])
    }

    @Test func allFailuresRemainNamedAndBlockUpload() async {
        let result = await SharedAttachmentLoader.load([failedProvider()])
        #expect(result.files.isEmpty)
        #expect(result.failures.map(\.name) == ["Unreadable.pdf"])
        #expect(!result.canUpload)
    }

    @Test func unsupportedAttachmentIsReported() async {
        let provider = NSItemProvider()
        provider.suggestedName = "Unsupported"
        let result = await SharedAttachmentLoader.load([provider])
        #expect(!result.canUpload)
        #expect(result.failures.first?.name == "Unsupported")
        #expect(result.failures.first?.message == "This attachment type is not supported.")
    }

    @Test func emptySelectionCannotUpload() async {
        #expect(await !SharedAttachmentLoader.load([]).canUpload)
    }
}
