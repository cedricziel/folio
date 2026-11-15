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
    /// IDs of the documents `documents(filter:)` would return, without their content.
    func documentIDs(filter: DocumentFilter) async throws -> [Document.ID]
    func deleteDocument(id: Document.ID) async throws
    /// Restores a trashed document in this service's vault and returns its current metadata.
    func restoreDocument(id: Document.ID) async throws -> Document
    /// Downloads the original file to a temporary location whose extension matches the file type.
    func downloadFile(for document: Document) async throws -> URL
    /// Returns a bounded JPEG preview, or nil when the document or thumbnail is unavailable.
    func thumbnail(id: Document.ID) async throws -> Data?
    func upload(_ file: UploadFile) async throws -> Document
    /// Uploads with an `Idempotency-Key`: sending the same key again returns the document the first request
    /// created instead of a second one.
    func upload(_ file: UploadFile, idempotencyKey: UUID) async throws -> Document
    func taxonomy(_ kind: TaxonomyKind, deleted: Bool) async throws -> [TaxonomyEntry]
    func saveTaxonomy(_ kind: TaxonomyKind, id: UUID?, name: String) async throws
    func deleteTaxonomy(_ kind: TaxonomyKind, id: UUID) async throws
    func restoreTaxonomy(_ kind: TaxonomyKind, id: UUID) async throws
    func tags() async throws -> [Tag]
    func addTag(_ tagID: Tag.ID, to documentID: Document.ID) async throws
    func removeTag(_ tagID: Tag.ID, from documentID: Document.ID) async throws
    /// Nil clears the correspondent.
    func setCorrespondent(_ correspondentID: Correspondent.ID?, for documentID: Document.ID) async throws
    /// Nil clears the document type.
    func setDocumentType(_ documentTypeID: DocumentType.ID?, for documentID: Document.ID) async throws
    func correspondents() async throws -> [Correspondent]
    func documentTypes() async throws -> [DocumentType]
}

public enum FolioError: Error, Equatable, LocalizedError {
    case invalidServerURL
    case server(status: Int, message: String?)
    case unreachable
    case offline
    case timedOut
    case hostUnreachable(host: String?)
    case untrustedCertificate(host: String?)
    case insecureAddress
    case unreadableResponse
    case signInRequired
    case signInFailed(String)

    /// Names the cause of a failed request. Only the host of the failing URL is kept, never its path or query.
    public init(transport error: URLError) {
        let host = error.failingURL?.host()
        switch error.code {
        case .notConnectedToInternet, .networkConnectionLost, .dataNotAllowed:
            self = .offline
        case .cannotFindHost, .dnsLookupFailed, .cannotConnectToHost:
            self = .hostUnreachable(host: host)
        case .timedOut:
            self = .timedOut
        case .serverCertificateUntrusted, .serverCertificateHasBadDate, .serverCertificateHasUnknownRoot,
            .serverCertificateNotYetValid, .secureConnectionFailed, .clientCertificateRejected:
            self = .untrustedCertificate(host: host)
        case .appTransportSecurityRequiresSecureConnection:
            self = .insecureAddress
        default:
            self = .unreachable
        }
    }

    /// Whether trying again can succeed without the user changing anything. A certificate problem or a blocked
    /// `http://` address stays the same on every attempt.
    public var isRetryable: Bool {
        switch self {
        case .unreachable, .offline, .timedOut, .hostUnreachable: true
        case let .server(status, _): status >= 500 || status == 408 || status == 429
        case .untrustedCertificate, .insecureAddress, .invalidServerURL, .unreadableResponse, .signInRequired,
            .signInFailed:
            false
        }
    }

    public var errorDescription: String? {
        switch self {
        case .offline:
            "You're offline. Try again when you're connected."
        case .timedOut:
            "The server took too long to answer. Try again."
        case let .hostUnreachable(host?):
            "Couldn't connect to \(host). Check the address, or the server may be down."
        case .hostUnreachable(nil):
            "Couldn't connect to the server. Check the address, or the server may be down."
        case let .untrustedCertificate(host?):
            "\(host) doesn't have a certificate this device trusts, so Folio won't connect."
        case .untrustedCertificate(nil):
            "The server doesn't have a certificate this device trusts, so Folio won't connect."
        case .insecureAddress:
            "Folio can't use http:// addresses. Use an https:// address instead."
        case .invalidServerURL:
            "That doesn't look like a server address."
        case let .server(_, message?):
            message
        case .server(let status, nil) where status >= 500:
            "The server had a problem. Try again later."
        case .server(let status, nil):
            "The server answered with status \(status)."
        case .unreachable:
            "Couldn't reach the server."
        case .unreadableResponse:
            "Couldn't read the server's answer."
        case .signInRequired:
            "Your session has expired. Sign in again."
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
    func documentIDs(filter: DocumentFilter) async throws -> [Document.ID] {
        try await documents(filter: filter).map(\.id)
    }

    func upload(_ file: UploadFile, idempotencyKey _: UUID) async throws -> Document {
        try await upload(file)
    }

    func taxonomy(_ kind: TaxonomyKind, deleted: Bool) async throws -> [TaxonomyEntry] {
        throw FolioError.server(status: 501, message: "Classification management is unavailable.")
    }
    func saveTaxonomy(_ kind: TaxonomyKind, id: UUID?, name: String) async throws {
        throw FolioError.server(status: 501, message: "Classification management is unavailable.")
    }
    func deleteTaxonomy(_ kind: TaxonomyKind, id: UUID) async throws {
        throw FolioError.server(status: 501, message: "Classification management is unavailable.")
    }
    func restoreTaxonomy(_ kind: TaxonomyKind, id: UUID) async throws {
        throw FolioError.server(status: 501, message: "Classification management is unavailable.")
    }
    func setDocumentReview(_ reviewed: Bool, for id: Document.ID) async throws -> Document {
        throw FolioError.server(status: 501, message: "This server does not support document review.")
    }

    func setDocumentDate(_ date: DocumentDate?, for id: Document.ID) async throws -> Document {
        throw FolioError.server(status: 501, message: "This server does not support document date editing.")
    }

    func setCorrespondent(_ correspondentID: Correspondent.ID?, for documentID: Document.ID) async throws {
        throw FolioError.server(status: 501, message: "Classification is unavailable.")
    }

    func setDocumentType(_ documentTypeID: DocumentType.ID?, for documentID: Document.ID) async throws {
        throw FolioError.server(status: 501, message: "Classification is unavailable.")
    }
}
