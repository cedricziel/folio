import Foundation

@MainActor
public enum HandoffActivity {
    /// Writes `payload` into a window's activity for Handoff, with the web page as the fallback for
    /// devices without Folio. A document builds on the window's on-screen activity from `tracker`, keeping
    /// its identity. The activity never enters search, public indexing or prediction; without a payload it
    /// hands nothing over.
    public static func configure(
        _ activity: NSUserActivity, payload: HandoffPayload?, tracker: DocumentActivityTracker?
    ) {
        if case .document = payload?.target, let tracker {
            tracker.configure(activity)
        } else {
            activity.title = nil
            activity.targetContentIdentifier = nil
        }
        activity.keywords = []
        activity.isEligibleForSearch = false
        activity.isEligibleForPublicIndexing = false
        #if os(iOS)
            activity.isEligibleForPrediction = false
        #endif
        activity.userInfo = payload?.userInfo ?? [:]
        activity.requiredUserInfoKeys = payload?.requiredUserInfoKeys ?? []
        activity.webpageURL = payload.flatMap(HandoffLink.webURL(for:))
        activity.isEligibleForHandoff = payload != nil
        activity.needsSave = true
    }
}
