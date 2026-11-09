import Foundation
import Observation

/// The one continuation the app has not handled yet, held until it can be routed: while the app is
/// locked, while the session restores or reconnects, or until the window that received it is ready.
/// One per app; the last continuation wins, and each is handled exactly once.
@MainActor @Observable
public final class ContinuationInbox {
    public struct Item: Equatable, Identifiable, Sendable {
        public let id = UUID()
        public let incoming: HandoffIncoming
        /// The window that received it, which handles it while it stays open.
        public let scene: UUID
        public fileprivate(set) var sceneIsOpen = true
    }

    public private(set) var pending: Item?

    public init() {}

    public func receive(_ incoming: HandoffIncoming, scene: UUID) {
        pending = Item(incoming: incoming, scene: scene)
    }

    /// Lets any window take an item whose receiving window has closed.
    public func sceneClosed(_ scene: UUID) {
        if pending?.scene == scene { pending?.sceneIsOpen = false }
    }

    /// What the pending item would do now, without handing it out. Nil while locked or empty.
    public func decision(isLocked: Bool, session: SessionRestoration) -> HandoffDecision? {
        guard !isLocked, let pending else { return nil }
        return HandoffResolver.decide(incoming: pending.incoming, session: session)
    }

    /// Hands the pending item to `scene` and removes it, when it is that window's to handle and `accept`
    /// wants its decision. Held and sign-in decisions stay pending: they change with the session.
    public func take(
        scene: UUID, isLocked: Bool, session: SessionRestoration,
        where accept: (HandoffDecision) -> Bool = { _ in true }
    ) -> HandoffDecision? {
        guard let pending, pending.scene == scene || !pending.sceneIsOpen,
            let decision = decision(isLocked: isLocked, session: session)
        else { return nil }
        switch decision {
        case .hold, .signIn: return nil
        default: break
        }
        guard accept(decision) else { return nil }
        self.pending = nil
        return decision
    }

    /// After signing in, keeps only a continuation for that server: one for another server was waiting
    /// for a sign-in that did not happen.
    public func signedIn(to server: URL) {
        guard case let .payload(payload) = pending?.incoming else { return }
        if !ServerIdentity.same(payload.server, server) { pending = nil }
    }

    public func clear() {
        pending = nil
    }
}
