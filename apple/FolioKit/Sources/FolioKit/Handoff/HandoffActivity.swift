import Foundation

@MainActor
public enum HandoffActivity {
    /// Writes `payload` into a window's activity for Handoff, with the web page as the fallback for
    /// devices without Folio. A document keeps the title and identity the on-screen activity gives it.
    /// The activity never enters search, public indexing or prediction; without a payload it hands
    /// nothing over.
    public static func configure(
        _ activity: NSUserActivity, payload: HandoffPayload?, tracker: DocumentActivityTracker = .shared
    ) {
        if case .document = payload?.target {
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
