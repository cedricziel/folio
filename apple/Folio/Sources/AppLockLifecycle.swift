import FolioKit
import SwiftUI

#if os(iOS)
    import UIKit
#else
    import AppKit
#endif

@MainActor
final class AppLockLifecycle: NSObject {
    let lock: AppLock
    private let authenticator: DeviceAuthenticator
    #if os(iOS)
        private var covers: [UIWindowScene: UIWindow] = [:]
        private var inactiveScenes: Set<UIScene> = []
    #else
        private var covers: [NSWindow: NSWindow] = [:]
    #endif

    override init() {
        let authenticator = DeviceAuthenticator()
        self.authenticator = authenticator
        lock = AppLock(defaults: .standard, authenticate: { await authenticator.authenticate(reason: $0) })
        super.init()
        let center = NotificationCenter.default
        #if os(iOS)
            center.addObserver(
                self, selector: #selector(resignActive), name: UIApplication.willResignActiveNotification, object: nil)
            center.addObserver(
                self, selector: #selector(becomeActive), name: UIApplication.didBecomeActiveNotification, object: nil)
            center.addObserver(
                self, selector: #selector(enterBackground), name: UIApplication.didEnterBackgroundNotification,
                object: nil)
            center.addObserver(
                self, selector: #selector(deviceLocked),
                name: UIApplication.protectedDataWillBecomeUnavailableNotification, object: nil)
            center.addObserver(
                self, selector: #selector(sceneInactive), name: UIScene.willDeactivateNotification, object: nil)
            center.addObserver(
                self, selector: #selector(sceneActive), name: UIScene.didActivateNotification, object: nil)
            center.addObserver(
                self, selector: #selector(sceneDisconnected), name: UIScene.didDisconnectNotification, object: nil)
            center.addObserver(
                self, selector: #selector(refreshCovers), name: UIWindow.didBecomeVisibleNotification, object: nil)
        #else
            center.addObserver(
                self, selector: #selector(resignActive), name: NSApplication.willResignActiveNotification, object: nil)
            center.addObserver(
                self, selector: #selector(becomeActive), name: NSApplication.didBecomeActiveNotification, object: nil)
            let workspace = NSWorkspace.shared.notificationCenter
            workspace.addObserver(
                self, selector: #selector(deviceLocked), name: NSWorkspace.screensDidSleepNotification, object: nil)
            workspace.addObserver(
                self, selector: #selector(deviceLocked), name: NSWorkspace.willSleepNotification, object: nil)
            workspace.addObserver(
                self, selector: #selector(deviceLocked), name: NSWorkspace.sessionDidResignActiveNotification,
                object: nil)
            // Workspace session notifications cover user switching, not every screen-lock path.
            DistributedNotificationCenter.default().addObserver(
                self, selector: #selector(deviceLocked), name: Notification.Name("com.apple.screenIsLocked"),
                object: nil
            )
        #endif
    }

    @objc private func resignActive() {
        lock.resignActive()
        refreshCovers()
    }

    @objc private func becomeActive() {
        lock.becomeActive()
        refreshCovers()
    }

    @objc private func enterBackground() {
        lock.enterBackground()
        authenticator.cancel()
        refreshCovers()
    }

    @objc private func deviceLocked() {
        lock.deviceLocked()
        authenticator.cancel()
        refreshCovers()
    }

    #if os(iOS)
        @objc private func sceneInactive(_ notification: Notification) {
            guard let scene = notification.object as? UIScene else { return }
            inactiveScenes.insert(scene)
            refreshCovers()
        }

        @objc private func sceneActive(_ notification: Notification) {
            guard let scene = notification.object as? UIScene else { return }
            inactiveScenes.remove(scene)
            refreshCovers()
        }

        @objc private func sceneDisconnected(_ notification: Notification) {
            guard let scene = notification.object as? UIWindowScene else { return }
            inactiveScenes.remove(scene)
            covers.removeValue(forKey: scene)?.isHidden = true
        }
    #endif

    @objc func refreshCovers() {
        #if os(iOS)
            for scene in UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }) {
                let covered =
                    lock.isEnabled
                    && (!lock.isActive || inactiveScenes.contains(scene) || scene.activationState != .foregroundActive)
                if covered && covers[scene] == nil {
                    let window = UIWindow(windowScene: scene)
                    window.isOpaque = true
                    window.backgroundColor = UIColor(Color.paper)
                    window.windowLevel = .alert + 1
                    window.rootViewController = UIHostingController(rootView: AppPrivacyCover())
                    covers[scene] = window
                    window.isHidden = false
                } else if !covered {
                    covers.removeValue(forKey: scene)?.isHidden = true
                }
            }
        #else
            if lock.needsPrivacyCover {
                let existingCovers = Set(covers.values)
                for window in NSApplication.shared.windows
                where window.isVisible && !existingCovers.contains(window) && covers[window] == nil {
                    let cover = NSWindow(
                        contentRect: window.frame, styleMask: .borderless, backing: .buffered, defer: false)
                    cover.isReleasedWhenClosed = false
                    cover.isOpaque = true
                    cover.backgroundColor = NSColor(Color.paper)
                    cover.contentView = NSHostingView(rootView: AppPrivacyCover())
                    covers[window] = cover
                    window.addChildWindow(cover, ordered: .above)
                }
            } else {
                for (window, cover) in covers {
                    window.removeChildWindow(cover)
                    cover.orderOut(nil)
                }
                covers.removeAll()
            }
        #endif
    }
}

private struct AppPrivacyCover: View {
    var body: some View {
        ZStack {
            Color.paper.ignoresSafeArea()
            Label("Folio", systemImage: "lock.fill")
                .font(FolioTheme.Typography.title)
                .foregroundStyle(Color.inkMuted)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Folio privacy cover")
    }
}
