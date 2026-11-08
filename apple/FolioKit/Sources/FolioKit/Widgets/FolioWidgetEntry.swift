import Foundation

/// One moment of a widget's timeline, derived from the snapshot the app last wrote.
public struct FolioWidgetEntry: Equatable, Sendable {
    public enum Content: Equatable, Sendable {
        /// The app has not written a snapshot yet.
        case notSetUp
        case signedOut
        case documents(WidgetSnapshot, isStale: Bool)
    }

    /// Widgets cannot refresh themselves; after this long they say the numbers may be out of date.
    public static let staleAfter: TimeInterval = 6 * 60 * 60

    public var date: Date
    public var content: Content

    public init(date: Date, content: Content) {
        self.date = date
        self.content = content
    }

    public static func timeline(for snapshot: WidgetSnapshot?, now: Date) -> [Self] {
        guard let snapshot else { return [Self(date: now, content: .notSetUp)] }
        guard snapshot.isSignedIn else { return [Self(date: now, content: .signedOut)] }
        let staleDate = snapshot.updatedAt.addingTimeInterval(staleAfter)
        guard now < staleDate else { return [Self(date: now, content: .documents(snapshot, isStale: true))] }
        return [
            Self(date: now, content: .documents(snapshot, isStale: false)),
            Self(date: staleDate, content: .documents(snapshot, isStale: true)),
        ]
    }
}
