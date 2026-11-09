import Foundation
import Testing

@testable import FolioKit

/// User-started upload batches hand their progress to the system so they can finish in the background (#364).
@MainActor struct ContinuedUploadTests {
    @Test func aBatchReportsSystemProgressUntilItFinishes() async {
        let scheduler = RecordingScheduler()
        let model = UploadViewModel(service: MockFolioService(), continuedWork: scheduler) {}

        await model.upload([file("a.pdf"), file("b.pdf")])

        #expect(scheduler.begun.map(\.total) == [2])
        #expect(scheduler.work?.reported == [0, 1, 2])
        #expect(scheduler.work?.result == true)
        #expect(model.state == .finished(count: 2))
    }

    @Test func aFailedBatchEndsItsSystemWorkUnsuccessfully() async {
        let service = MockFolioService()
        await service.setFailUploadsAfter(1)
        let scheduler = RecordingScheduler()
        let model = UploadViewModel(service: service, continuedWork: scheduler) {}

        await model.upload([file("a.pdf"), file("b.pdf")])

        #expect(scheduler.work?.reported == [0, 1])
        #expect(scheduler.work?.result == false)
    }

    @Test func withoutSystemSupportTheBatchStillRuns() async {
        let scheduler = RecordingScheduler(grants: false)
        let model = UploadViewModel(service: MockFolioService(), continuedWork: scheduler) {}

        await model.upload([file("a.pdf")])

        #expect(scheduler.begun.count == 1)
        #expect(model.state == .finished(count: 1))
    }

    @Test func cancellingFromTheSystemStopsTheBatchAndRetryResumesWithTheSameKeys() async {
        let service = MockFolioService()
        let scheduler = RecordingScheduler()
        let model = UploadViewModel(service: service, continuedWork: scheduler) {}
        let files = [file("a.pdf"), file("b.pdf")]
        await service.suspendUploads()
        let batch = Task { await model.upload(files) }
        await service.waitForUploadRequest()
        await service.setUploadDelay(.seconds(30))

        scheduler.cancel?()
        await service.releaseUploads()
        await batch.value

        #expect(model.state == .failed(message: "Upload stopped.", uploaded: 0))
        #expect(scheduler.work?.result == false)
        #expect(await service.uploads.isEmpty)

        await service.setUploadDelay(nil)
        await model.upload(files)

        let keys = await service.uploadKeys
        #expect(keys.count == 3)
        #expect(keys[0] == keys[1], "The interrupted file is retried under its first key.")
        #expect(model.state == .finished(count: 2))
    }

    @Test func aRetryAfterALostResponseDoesNotUploadTwice() async {
        let service = MockFolioService()
        await service.setLoseUploadResponses(true)
        let model = UploadViewModel(service: service) {}
        let files = [file("a.pdf"), file("b.pdf")]
        await model.upload(files)
        await service.setLoseUploadResponses(false)

        await model.upload(files)

        #expect(await service.uploads.map(\.filename) == ["a.pdf", "b.pdf"])
        #expect(model.state == .finished(count: 2))
    }

    private func file(_ name: String) -> UploadFile {
        UploadFile(data: Data(name.utf8), filename: name, mimeType: "application/pdf")
    }
}

@MainActor private final class RecordingScheduler: ContinuedWorkScheduling {
    private let grants: Bool
    private(set) var begun: [(title: String, total: Int)] = []
    private(set) var work: RecordingWork?
    private(set) var cancel: (@MainActor @Sendable () -> Void)?

    init(grants: Bool = true) {
        self.grants = grants
    }

    func begin(title: String, total: Int, onCancel: @escaping @MainActor @Sendable () -> Void) -> (any ContinuedWork)? {
        begun.append((title, total))
        cancel = onCancel
        guard grants else { return nil }
        let work = RecordingWork()
        self.work = work
        return work
    }
}

@MainActor private final class RecordingWork: ContinuedWork {
    private(set) var reported: [Int] = []
    private(set) var result: Bool?

    func update(completed: Int) {
        reported.append(completed)
    }

    func finish(success: Bool) {
        result = success
    }
}
