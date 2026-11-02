import Foundation

/// Copy and number formats from design/README.md: ISO dates in lists, calm words for states.
public enum FolioFormat {
    /// `2025-03-14`, the form lists use so dates compare across rows.
    public static func isoDay(_ date: Date, timeZone: TimeZone = .current) -> String {
        date.formatted(
            Date.ISO8601FormatStyle(timeZone: timeZone).year().month().day()
        )
    }

    public static func statusText(_ status: Document.Status) -> String? {
        switch status {
        case let .reading(pages?):
            "Reading \(pages) \(pages == 1 ? "page" : "pages")…"
        case .reading(nil):
            "Reading…"
        case .indexing:
            "Indexing…"
        case .failed:
            "Couldn't process"
        case .needsReview:
            "Needs a look"
        case .filed:
            nil
        }
    }
}
