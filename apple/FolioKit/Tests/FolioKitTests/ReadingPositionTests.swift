import Testing

@testable import FolioKit

struct ReadingPositionTests {
    @Test(arguments: [
        (7 as Int?, 12 as Int?, 7), (12, 12, 12), (40, 12, 12), (1, 1, 1),
        (nil, 12, 1), (0, 12, 1), (-3, 12, 1), (5, nil, 1), (nil, nil, 1), (5, 0, 1),
    ])
    func clampsToTheLoadedDocument(_ page: Int?, _ pageCount: Int?, _ expected: Int) {
        #expect(ReadingPosition.clamp(page: page, pageCount: pageCount) == expected)
    }
}
