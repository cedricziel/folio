import FileProvider
import FolioKit
import OSLog

extension Error {
    var providerError: any Error {
        let error = self as NSError
        if error.domain == NSCocoaErrorDomain || error.domain == NSFileProviderErrorDomain { return self }
        if self is CancellationError || (self as? URLError)?.code == .cancelled {
            return NSError(domain: NSCocoaErrorDomain, code: NSUserCancelledError)
        }
        if self is URLError { return NSFileProviderError(.serverUnreachable) }
        switch self as? FolioError {
        case .signInRequired, .server(status: 401, message: _):
            return NSFileProviderError(.notAuthenticated)
        case .unreachable:
            return NSFileProviderError(.serverUnreachable)
        case .server(status: 404, message: _):
            return NSFileProviderError(.noSuchItem)
        case .server(status: 403, message: _):
            return NSError(domain: NSCocoaErrorDomain, code: NSFileReadNoPermissionError)
        case .server(let status, _) where status >= 500 || status == 429:
            return NSFileProviderError(.serverUnreachable)
        default:
            return NSError(
                domain: NSCocoaErrorDomain, code: NSXPCConnectionReplyInvalid,
                userInfo: [NSUnderlyingErrorKey: self])
        }
    }
}

enum ProviderDiagnostics {
    private static let logger = Logger(subsystem: "com.cedricziel.folio.fileprovider", category: "transfer")

    static func failure(operation: String, phase: String, error: Error) {
        let raw = error as NSError
        let mapped = error.providerError as NSError
        let status: Int
        if case let .server(code, _) = error as? FolioError { status = code } else { status = 0 }
        logger.error(
            "\(operation, privacy: .public) phase=\(phase, privacy: .public) domain=\(raw.domain, privacy: .public) code=\(raw.code) http=\(status) mappedDomain=\(mapped.domain, privacy: .public) mappedCode=\(mapped.code)"
        )
    }
}
