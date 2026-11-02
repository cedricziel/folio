import Darwin
import Foundation
import Testing

@testable import FolioKit

struct OAuthRefreshLockTests {
    private func temporaryDirectory() throws -> URL {
        let directory = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return directory
    }

    @Test func lockExcludesOtherFileDescriptorsAndReleasesAfterSuccess() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appending(path: "refresh.lock")
        let lock = OAuthRefreshLock(url: url)

        let value = try await lock.withLock {
            let other = open(url.path, O_RDWR)
            defer { close(other) }
            #expect(other >= 0)
            #expect(flock(other, LOCK_EX | LOCK_NB) == -1)
            #expect(errno == EWOULDBLOCK)
            return 42
        }

        #expect(value == 42)
        let other = open(url.path, O_RDWR)
        defer { close(other) }
        #expect(flock(other, LOCK_EX | LOCK_NB) == 0)
    }

    @Test func throwingOperationReleasesLockForAnotherInstance() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appending(path: "refresh.lock")
        let first = OAuthRefreshLock(url: url)
        let second = OAuthRefreshLock(url: url)

        await #expect(throws: CancellationError.self) {
            try await first.withLock { throw CancellationError() }
        }
        let value = try await second.withLock { 7 }
        #expect(value == 7)
    }

    @Test func cancellationWhileWaitingDoesNotRunOperation() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let url = directory.appending(path: "refresh.lock")
        let holder = open(url.path, O_CREAT | O_RDWR, S_IRUSR | S_IWUSR)
        defer { close(holder) }
        #expect(flock(holder, LOCK_EX | LOCK_NB) == 0)
        let waiter = Task {
            try await OAuthRefreshLock(url: url).withLock {
                Issue.record("A cancelled waiter must not rotate credentials")
            }
        }
        waiter.cancel()
        await #expect(throws: CancellationError.self) { try await waiter.value }
    }

    @Test func inaccessibleLockFailsWithoutRunningOperation() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let lock = OAuthRefreshLock(url: directory.appending(path: "missing/refresh.lock"))

        await #expect(throws: POSIXError.self) {
            try await lock.withLock { Issue.record("Failed locking must prevent credential rotation") }
        }
    }
}
