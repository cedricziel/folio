# Apple widgets and the Scan control

The Apple app ships an optional WidgetKit extension, `FolioWidgets` (`com.cedricziel.folio.widgets`), with:

- **Inbox** (small; Lock Screen circular and rectangular on iPhone): how many documents still need review. Tapping it opens the Inbox.
- **Latest document** (small, medium): the newest document's title and date. Tapping it opens the document. The medium size has a Scan button.
- **Scan to Folio** control (iOS 18, macOS 26): a Control Center, Lock Screen and Action button control that runs `ScanDocumentIntent`. It opens Folio, which shows the scanner after App lock is unlocked. On a Mac it opens the file picker, because Macs have no scanner. The review sheet still asks for the destination vault.

The extension is off by default. Embedding it needs its own App ID and App Store profiles, and release signing would fail without them.

## How the widgets get their data

Widgets never call the server. Each time the app becomes active or goes to the background, it reads up to 300 of the newest documents in the server's default vault. It writes a small snapshot to the `group.com.cedricziel.folio` App Group and reloads the widget timelines. The snapshot holds:

- the number of documents not yet filed among those 300. When more documents exist, the widget adds "+", for example "20+".
- the newest document's ID, vault, date and title,
- whether the user is signed in.

Privacy:

- While **App lock** is on, the snapshot holds no titles, and the widget shows "Locked document". Turning App lock on removes titles from the stored snapshot right away.
- Titles are marked `privacySensitive`, so the system redacts them on a locked device.
- Signing out, or a revoked sign-in, replaces the snapshot with a signed-out state that holds no documents.

A widget whose snapshot is more than six hours old shows when it was last updated. Push-driven reloads (`WidgetPushHandler`) can follow once push notifications (#359) are on.

## Turning it on

1. In the [developer portal](https://developer.apple.com/account/resources/identifiers/list), check that the App Group `group.com.cedricziel.folio` exists.
2. Register the App ID and create its profiles:

   ```sh
   cd apple
   bundle exec fastlane bootstrap widgets:true
   ```

   This registers `com.cedricziel.folio.widgets` with the App Groups capability. Fastlane cannot assign the group itself; it prints the step.
3. In the portal, open **Identifiers > com.cedricziel.folio.widgets > App Groups** and assign `group.com.cedricziel.folio`.
4. Run `bundle exec fastlane bootstrap widgets:true force:true`, so match regenerates the iOS and macOS App Store profiles with the group.
5. Set `FOLIO_WIDGETS: "YES"` in the `env` of the `beta` and `beta-mac` jobs in `.github/workflows/apple.yml`. Fastlane then generates the project with the extension and fetches its profiles.

For local builds, generate the project with the flag. A self-build (`Config/Local.xcconfig`) uses `$(FOLIO_BUNDLE_ID).widgets` and `$(FOLIO_APP_GROUP)`, and with Manual signing needs `FOLIO_PROFILE_WIDGETS_IOS` and `FOLIO_PROFILE_WIDGETS_MACOS`.

```sh
cd apple/Folio
FOLIO_WIDGETS=YES xcodegen generate
```

## How the flag works

`project.yml` includes `widgets.yml` only when `FOLIO_WIDGETS` is `YES` while XcodeGen runs. That file adds the extension target and embeds it in the app. It also sets `FolioWidgetsEnabled` in the app's Info.plist and registers the `$(FOLIO_BUNDLE_ID).widgets` URL scheme that widget taps use. Without the flag, the app writes no snapshot.

Pull request CI builds the app both ways. It checks that the default project leaves the extension out, then builds it with `FOLIO_WIDGETS=YES CODE_SIGNING_ALLOWED=NO` for the iOS Simulator and macOS.

## Not yet done

- Configuring the latest-document widget by vault, tag or saved view
- A Mark reviewed button
- Push reloads
- An Upload control on macOS
