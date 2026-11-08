import Foundation
import Testing

@testable import FolioKit

struct FileLeaseTests {
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "folio-lease-\(UUID().uuidString)", directoryHint: .isDirectory)

    private func lease() -> FileLease {
        FileLease(file: directory.appending(path: "batch.lock"), timeout: 60)
    }

    /// Makes `generation` look as if its holder stopped renewing it two minutes ago.
    private func abandon(_ generation: Int) throws {
        try FileManager.default.setAttributes(
            [.modificationDate: Date.now.addingTimeInterval(-120)],
            ofItemAtPath: directory.appending(path: "batch.lock.\(generation)").path)
    }

    private func files() throws -> [String] {
        try FileManager.default.contentsOfDirectory(atPath: directory.path).sorted()
    }

    @Test func aHeldLeaseIsRefusedUntilReleased() throws {
        defer { try? FileManager.default.removeItem(at: directory) }
        let holder = try #require(lease().acquire())

        #expect(lease().acquire() == nil)
        lease().release(holder)
        let next = try #require(lease().acquire())
        #expect(next > holder)
        #expect(try files() == ["batch.lock.\(next)"], "superseded generations are cleaned up")
    }

    @Test func anAbandonedLeaseIsTakenOverAndItsHolderCannotRenewOrReleaseIt() throws {
        defer { try? FileManager.default.removeItem(at: directory) }
        let abandoned = try #require(lease().acquire())
        try abandon(abandoned)

        let taker = try #require(lease().acquire())

        #expect(!lease().renew(abandoned))
        lease().release(abandoned)
        #expect(lease().acquire() == nil, "the late release left the new holder's lease in place")
        #expect(lease().renew(taker))
    }

    @Test func aRenewedLeaseIsNotTakenOver() throws {
        defer { try? FileManager.default.removeItem(at: directory) }
        let holder = try #require(lease().acquire())
        try abandon(holder)

        #expect(lease().renew(holder))

        #expect(lease().acquire() == nil)
    }

    @Test func aRecreatedOlderGenerationDoesNotHoldTheLease() throws {
        defer { try? FileManager.default.removeItem(at: directory) }
        let first = try #require(lease().acquire())
        try abandon(first)
        let current = try #require(lease().acquire())
        // A process that listed before the takeover recreates the generation it meant to take.
        try Data().write(to: directory.appending(path: "batch.lock.\(first)"))

        #expect(!lease().renew(first))
        #expect(lease().renew(current))
        #expect(lease().acquire() == nil)
    }

    @Test func onlyOneOfManyContendersTakesOverAnAbandonedLease() throws {
        defer { try? FileManager.default.removeItem(at: directory) }
        for _ in 0..<50 {
            let abandoned = try #require(lease().acquire())
            try abandon(abandoned)
            let winners = Winners()

            DispatchQueue.concurrentPerform(iterations: 8) { _ in
                if let generation = lease().acquire() {
                    winners.add(generation)
                }
            }

            #expect(winners.count == 1)
            winners.releaseAll(lease())
        }
    }

    @Test func removingForgetsEveryGeneration() throws {
        defer { try? FileManager.default.removeItem(at: directory) }
        let holder = try #require(lease().acquire())
        try Data().write(to: directory.appending(path: "batch.lock"))

        lease().removeAll()

        #expect(try files().isEmpty)
        #expect(lease().acquire() != nil)
        #expect(!lease().renew(holder + 1_000))
    }

    private final class Winners: @unchecked Sendable {
        private let lock = NSLock()
        private var generations: [Int] = []

        var count: Int { lock.withLock { generations.count } }

        func add(_ generation: Int) {
            lock.withLock { generations.append(generation) }
        }

        func releaseAll(_ lease: FileLease) {
            for generation in lock.withLock({ generations }) {
                lease.release(generation)
            }
        }
    }
}
