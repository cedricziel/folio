import Foundation
import OpenTelemetryApi

/// Screens named in telemetry. The raw value is the exported `app.screen.name`.
public enum TelemetryScreen: String, CaseIterable, Sendable {
    case onboarding
    case appLock = "app_lock"
    case inbox
    case documents
    case savedView = "saved_view"
    case savedViewEditor = "saved_view.editor"
    case documentDetail = "document.detail"
    case documentPreview = "document.preview"
    case documentDiscussion = "document.discussion"
    case documentProcessing = "document.processing"
    case scanReview = "scan.review"
    case uploadReview = "upload.review"
    case taxonomy
    case sources
    case sourceOutcomes = "source.outcomes"
    case share
}

/// Deliberate user actions named in telemetry. The raw value is the exported `folio.action.name`.
public enum TelemetryAction: String, CaseIterable, Sendable {
    case documentOpen = "document.open"
    case documentUpload = "document.upload"
    case documentDelete = "document.delete"
    case documentRestore = "document.restore"
    case documentReview = "document.review"
    case documentDownload = "document.download"
    case documentShare = "document.share"
    case documentReprocess = "document.reprocess"
    case documentEdit = "document.edit"
    case tagAdd = "document.tag.add"
    case tagRemove = "document.tag.remove"
    case discussionPost = "discussion.post"
    case browseScopeChange = "browse.scope.change"
    case browseFilterChange = "browse.filter.change"
    case browseLoadMore = "browse.load_more"
    case browseRefresh = "browse.refresh"
    case searchSubmit = "search.submit"
    case savedViewOpen = "saved_view.open"
    case savedViewSave = "saved_view.save"
    case savedViewDelete = "saved_view.delete"
    case vaultSwitch = "vault.switch"
    case scanStart = "scan.start"
    case scanFinish = "scan.finish"
    case scanCancel = "scan.cancel"
    case shareSubmit = "share.submit"
    case shareCancel = "share.cancel"
    case signOut = "auth.sign_out"
    case appLockUnlock = "app_lock.unlock"
    case intentRun = "intent.run"
}

/// Where an upload came from, exported as `folio.upload.source`.
public enum UploadSource: String, CaseIterable, Sendable {
    case picker, scan, drop, paste, share, intent
}

/// App Intents named in telemetry, exported as `folio.intent.name` on `intent.run`.
public enum TelemetryIntent: String, CaseIterable, Sendable {
    case scanDocument = "scan_document"
    case uploadFile = "upload_file"
    case searchDocuments = "search_documents"
    case findDocuments = "find_documents"
    case getLatestDocument = "get_latest_document"
    case openDocument = "open_document"
    case addTag = "add_tag"
    case removeTag = "remove_tag"
    case getDocumentFile = "get_document_file"
}

/// Which list a browse request serves, exported as `folio.browse.scope`. Never sent to the browse API.
public enum BrowseScope: String, CaseIterable, Sendable {
    case inbox, all
    case savedView = "saved_view"
    case intent
}

extension DocumentBrowseQuery {
    /// What kinds of filter are set, never their values, plus scope, sort and page.
    func telemetryAttributes(cursor: String?) -> [String: AttributeValue] {
        var kinds: [String] = []
        if !text.isEmpty { kinds.append("text") }
        if !filter.tagIDs.isEmpty { kinds.append("tag") }
        if filter.correspondentID != nil { kinds.append("correspondent") }
        if filter.documentTypeID != nil { kinds.append("document_type") }
        if filter.documentDateFrom != nil || filter.documentDateTo != nil { kinds.append("document_date") }
        if filter.createdAfter != nil || filter.createdBefore != nil { kinds.append("created_date") }
        if filter.reviewed != nil { kinds.append("reviewed") }
        return [
            "folio.browse.scope": .string(scope.rawValue),
            "folio.browse.filters": .array(AttributeArray(values: kinds.sorted().map { .string($0) })),
            "folio.browse.sort": .string(sort.rawValue),
            "folio.browse.page": .string(cursor == nil ? "first" : "next"),
        ]
    }
}

/// Allows at most `limit` UI events in any rolling minute and counts the rest.
struct UIEventLimiter {
    let limit: Int
    private var times: [ContinuousClock.Instant] = []
    private var dropped = 0

    init(limit: Int = 60) {
        self.limit = limit
    }

    mutating func allow(at now: ContinuousClock.Instant) -> Bool {
        times.removeAll { now - $0 > .seconds(60) }
        guard times.count < limit else {
            dropped += 1
            return false
        }
        times.append(now)
        return true
    }

    mutating func takeDropped() -> Int {
        defer { dropped = 0 }
        return dropped
    }
}
