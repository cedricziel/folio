import Foundation

public enum ReadingPosition {
    /// The 1-based page to open: the requested page within 1...pageCount, or the first page when the
    /// page or the page count is unknown or not positive.
    public static func clamp(page: Int?, pageCount: Int?) -> Int {
        guard let page, page > 0, let pageCount, pageCount > 0 else { return 1 }
        return min(page, pageCount)
    }
}
