import Foundation

/// Everything the app needs from a Folio server. View models depend on this, not on the generated client.
public protocol FolioService: DocumentEntryService {
    func vaults() async throws -> VaultCatalog
    func savedViews(after: UUID?) async throws -> [SavedView]
    func savedView(id: UUID) async throws -> SavedView
    func saveView(_ draft: SavedViewDraft) async throws -> SavedView
    func updateSavedView(id: UUID, draft: SavedViewDraft) async throws -> SavedView
    func deleteSavedView(id: UUID) async throws
    /// Returns a service whose requests retain this vault even if the visible library changes.
    func scoped(to vaultID: UUID) -> any FolioService
    func documentPage(query: DocumentBrowseQuery, cursor: String?) async throws -> DocumentPage
    func document(id: Document.ID) async throws -> Document
    func setDocumentReview(_ reviewed: Bool, for id: Document.ID) async throws -> Document
    func setDocumentDate(_ date: DocumentDate?, for id: Document.ID) async throws -> Document
    func ping() async throws
    func documents(matching query: String) async throws -> [Document]
    /// Server-side filtering; the filters combine with AND and a document must carry every tag listed.
    func documents(filter: DocumentFilter) async throws -> [Document]
    func deleteDocument(id: Document.ID) async throws
    /// Restores a trashed document in this service's vault and returns its current metadata.
    func restoreDocument(id: Document.ID) async throws -> Document
    /// Downloads the original file to a temporary location whose extension matches the file type.
    func downloadFile(for document: Document) async throws -> URL
    /// Returns a bounded JPEG preview, or nil when the document or thumbnail is unavailable.
    func thumbnail(id: Document.ID) async throws -> Data?
    func upload(_ file: UploadFile) async throws -> Document
    func tags() async throws -> [Tag]
    func addTag(_ tagID: Tag.ID, to documentID: Document.ID) async throws
    func removeTag(_ tagID: Tag.ID, from documentID: Document.ID) async throws
    func correspondents() async throws -> [Correspondent]
    func documentTypes() async throws -> [DocumentType]
}

public enum FolioError: Error, Equatable, LocalizedError {
    case invalidServerURL
    case server(status: Int, message: String?)
    case unreachable
    case unreadableResponse
    case signInRequired
    case signInFailed(String)

    public var errorDescription: String? {
        switch self {
        case .invalidServerURL:
            "That doesn't look like a server address."
        case let .server(_, message?):
            message
        case .server(let status, nil):
            "The server answered with status \(status)."
        case .unreachable:
            "Couldn't reach the server."
        case .unreadableResponse:
            "Couldn't read the server's answer."
        case .signInRequired:
            "Sign in again to continue."
        case let .signInFailed(message):
            message
        }
    }
}

/// Supplies the bearer token for API requests.
public protocol BearerTokenProvider: Sendable {
    func token() async throws -> String?
    /// Called after the server rejected `token`. Returns a fresh token to retry with, or nil to give up.
    func refresh(rejecting token: String?) async throws -> String?
}

public extension BearerTokenProvider {
    func refresh(rejecting _: String?) async -> String? {
        nil
    }
}

public struct NoToken: BearerTokenProvider {
    public init() {}
    public func token() async throws -> String? {
        nil
    }
}

public extension FolioService {
    func setDocumentReview(_ reviewed: Bool, for id: Document.ID) async throws -> Document {
        throw FolioError.server(status: 501, message: "This server does not support document review.")
    }

    func setDocumentDate(_ date: DocumentDate?, for id: Document.ID) async throws -> Document {
        throw FolioError.server(status: 501, message: "This server does not support document date editing.")
    }
}
