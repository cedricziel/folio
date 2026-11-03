import Foundation

public extension DocumentFilter {
    func applyingReview(_ reviewed: Bool?, from: String, to: String) throws -> Self {
        let start = from.isEmpty ? nil : DocumentDate(rawValue: from)
        let end = to.isEmpty ? nil : DocumentDate(rawValue: to)
        guard (from.isEmpty || start != nil), (to.isEmpty || end != nil) else {
            throw FolioError.server(status: 422, message: "Enter dates as YYYY-MM-DD.")
        }
        if let start, let end, start.rawValue > end.rawValue {
            throw FolioError.server(status: 422, message: "The end date must not precede the start date.")
        }
        var filter = self
        filter.reviewed = reviewed
        filter.documentDateFrom = start
        filter.documentDateTo = end
        return filter
    }
}
