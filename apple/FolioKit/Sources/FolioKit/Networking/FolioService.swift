import Foundation

/// Everything the app needs from a Folio server. View models depend on this, not on the generated client.
public protocol FolioService: Sendable {
    func ping() async throws
    func documents(matching query: String) async throws -> [Document]
    func documents(correspondentID: UUID?, documentTypeID: UUID?) async throws -> [Document]
    func deleteDocument(id: Document.ID) async throws
    /// Downloads the original file to a temporary location whose extension matches the file type.
    func downloadFile(for document: Document) async throws -> URL
    func upload(_ file: UploadFile) async throws -> Document
    func tags() async throws -> [Tag]
    func addTag(_ tagID: Tag.ID, to documentID: Document.ID) async throws
    func correspondents() async throws -> [Correspondent]
    func documentTypes() async throws -> [DocumentType]
}

public enum FolioError: Error, Equatable, LocalizedError {
    case invalidServerURL
    case server(status: Int, message: String?)
    case unreachable
    case unreadableResponse

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
        }
    }
}

/// Auth is not part of the API yet. Once it is, a Keychain-backed provider plugs in here.
public protocol BearerTokenProvider: Sendable {
    func token() async -> String?
}

public struct NoToken: BearerTokenProvider {
    public init() {}
    public func token() async -> String? {
        nil
    }
}
