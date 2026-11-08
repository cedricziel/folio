import Foundation
import Testing

@testable import FolioKit

struct FileLeaseTests {
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "folio-lease-\(UUID().uuidString)", directoryHint: .isDirectory)

    private func lease() -> FileLease {
        FileLease(file: directory.appending(path: "batch.lock"), timeout: 60)
    }

    private func age(_ lease: FileLease, seconds: TimeInterval) throws {
        try FileManager.default.setAttributes(
            [.modificationDate: Date.now.addingTimeInterval(-seconds)], ofItemAtPath: lease.file.path)
    }

    @Test func aHeldLeaseIsRefusedUntilReleased() throws {
        defer { try? FileManager.default.removeItem(at: directory) }
        let holder = try #require(lease().acquire())

        #expect(lease().acquire() == nil)
        lease().release(holder)
        #expect(lease().acquire() != nil)
    }

    @Test func anAbandonedLeaseIsTakenOverAndItsHolderCannotReleaseIt() throws {
        defer { try? FileManager.default.removeItem(at: directory) }
        let abandoned = try #require(lease().acquire())
        try age(lease(), seconds: 120)

        let taker = try #require(lease().acquire())

        #expect(!lease().renew(abandoned))
        lease().release(abandoned)
        #expect(lease().acquire() == nil, "the late release left the new holder's lease in place")
        #expect(lease().renew(taker))
    }

    @Test func aRenewedLeaseIsNotTakenOver() throws {
        defer { try? FileManager.default.removeItem(at: directory) }
        let holder = try #require(lease().acquire())
        try age(lease(), seconds: 120)

        #expect(lease().renew(holder))

        #expect(lease().acquire() == nil)
    }

    @Test func onlyOneOfTwoContendersTakesOverAnAbandonedLease() throws {
        defer { try? FileManager.default.removeItem(at: directory) }
        for _ in 0..<200 {
            let abandoned = try #require(lease().acquire())
            try age(lease(), seconds: 120)
            let winners = Winners()

            DispatchQueue.concurrentPerform(iterations: 2) { _ in
                if let token = lease().acquire() {
                    winners.add(token)
                }
            }

            #expect(winners.count == 1)
            lease().release(abandoned)
            winners.releaseAll(lease())
            #expect(!FileManager.default.fileExists(atPath: lease().file.path))
        }
        let leftovers = try FileManager.default.contentsOfDirectory(atPath: directory.path)
        #expect(leftovers.isEmpty, "stale files moved aside are deleted")
    }

    private final class Winners: @unchecked Sendable {
        private let lock = NSLock()
        private var tokens: [UUID] = []

        var count: Int { lock.withLock { tokens.count } }

        func add(_ token: UUID) {
            lock.withLock { tokens.append(token) }
        }

        func releaseAll(_ lease: FileLease) {
            for token in lock.withLock({ tokens }) {
                lease.release(token)
            }
        }
    }
}
