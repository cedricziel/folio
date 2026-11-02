import Darwin
import Foundation

/// Serializes credential rotation across processes sharing the same lock file.
public struct OAuthRefreshLock: Sendable {
    private let url: URL?

    /// The parent directory must exist and be shared by every credential writer.
    public init(url: URL?) {
        self.url = url
    }

    /// Holds the lock across the operation, releasing it on success, failure, or cancellation.
    public func withLock<T: Sendable>(
        _ operation: @Sendable () async throws -> T
    ) async throws -> T {
        try Task.checkCancellation()
        guard let url else { throw CocoaError(.fileNoSuchFile) }
        let descriptor = open(url.path, O_CREAT | O_RDWR | O_CLOEXEC | O_NOFOLLOW, S_IRUSR | S_IWUSR)
        guard descriptor >= 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
        defer { close(descriptor) }
        while flock(descriptor, LOCK_EX | LOCK_NB) != 0 {
            let code = errno
            guard code == EWOULDBLOCK || code == EINTR else {
                throw POSIXError(POSIXErrorCode(rawValue: code) ?? .EIO)
            }
            try await Task.sleep(for: .milliseconds(25))
        }
        defer { flock(descriptor, LOCK_UN) }
        try Task.checkCancellation()
        return try await operation()
    }
}
