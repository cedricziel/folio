@testable import FolioKit
import Foundation

actor MockFolioService: FolioService {
    var documentList: [Document] = []
    var correspondentList: [Correspondent] = []
    var typeList: [DocumentType] = []
    var failure: FolioError?
    var failUploadsAfter: Int?
    var failLookups = false
    private(set) var queries: [String] = []
    private(set) var uploads: [UploadFile] = []
    private(set) var deleted: [Document.ID] = []
    private(set) var pings = 0

    init(
        documents: [Document] = [], correspondents: [Correspondent] = [], types: [DocumentType] = []
    ) {
        documentList = documents
        correspondentList = correspondents
        typeList = types
    }

    func setFailure(_ error: FolioError?) {
        failure = error
    }

    func setFailUploadsAfter(_ count: Int?) {
        failUploadsAfter = count
    }

    func setFailLookups(_ value: Bool) {
        failLookups = value
    }

    func ping() async throws {
        pings += 1
        if let failure {
            throw failure
        }
    }

    func documents(matching query: String) async throws -> [Document] {
        queries.append(query)
        if let failure {
            throw failure
        }
        return documentList
    }

    func deleteDocument(id: Document.ID) async throws {
        if let failure {
            throw failure
        }
        deleted.append(id)
        documentList.removeAll { $0.id == id }
    }

    func downloadFile(for document: Document) async throws -> URL {
        if let failure {
            throw failure
        }
        return URL(filePath: "/tmp/\(document.originalFilename ?? "file")")
    }

    func upload(_ file: UploadFile) async throws -> Document {
        if let failure {
            throw failure
        }
        if let limit = failUploadsAfter, uploads.count >= limit {
            throw FolioError.unreachable
        }
        uploads.append(file)
        return Document(title: file.title ?? file.filename, originalFilename: file.filename)
    }

    func tags() async throws -> [Tag] {
        []
    }

    func correspondents() async throws -> [Correspondent] {
        if failLookups {
            throw FolioError.unreachable
        }
        return correspondentList
    }

    func documentTypes() async throws -> [DocumentType] {
        if failLookups {
            throw FolioError.unreachable
        }
        return typeList
    }
}

final class MemoryServerConfigStore: ServerConfigStore, @unchecked Sendable {
    var url: URL?
    init(url: URL? = nil) {
        self.url = url
    }

    func load() -> URL? {
        url
    }

    func save(_ url: URL?) {
        self.url = url
    }
}
