# Push notifications

Folio can notify the Apple app when a document someone uploaded needs review, has finished processing, or has failed. This page is for people running a Folio server.

## What it takes

Apple delivers every notification to an iPhone or Mac through the Apple Push Notification service (APNs). APNs accepts requests only when they are signed with a key belonging to the Apple Developer team that publishes the app. In practice:

- **The App Store build of Folio** can only receive notifications from a server that holds the Folio team's key. A self-hosted server can't create a key of its own for this app.
- **A Folio you build and sign yourself**, under your own bundle ID and team, can receive notifications from your server with your own key.

Without push configured, everything else keeps working. The app registers, learns that the server has no push (`push_available: false`) and says so in its settings.

## Turning it on

You need an APNs authentication key (`.p8`) from your Apple Developer account (Certificates, Identifiers & Profiles → Keys), its key ID, your team ID, and the bundle ID of your app. Then set:

```toml
[push]
transport = "apns"

[push.apns]
team_id = "ABCDE12345"
key_id = "KEY1234567"
topic = "com.example.folio"   # your app's bundle ID
```

Give the key itself as a file, `FOLIO_PUSH__APNS__PRIVATE_KEY_FILE=/run/secrets/apns.p8`, rather than pasting it into `config.toml`.

Admins can also set all of this on `/settings` under **Push notifications**, and a value saved there wins over the configuration. A key is stored there only when `auth.secret_key` comes from the configuration or a file. A key encrypted with a secret kept in the same database would be readable by anyone with a copy of it. The card has a **Send test notification** button, which sends a test to your own registered devices.

## What gets sent

- **Who.** By default, only the person who uploaded a document hears about it. The app lets people widen that to every document they may edit. Viewers are never notified, and losing access to a vault, signing out, or revoking the app on the Security page stops notifications at once.
- **When.** Only a document's first completed processing or its first terminal failure notifies. Reprocessing, re-embedding, repair, imports and retries that are still pending don't. Outcomes are gathered for `push.quiet_window_seconds` (default 30) and sent as one alert, such as "3 documents need review". A device gets at most `push.max_alerts_per_hour` (default 12) alerts an hour, and anything beyond that is counted into the next alert allowed.
- **What.** Alerts say only how many documents reached which outcome. They never contain titles, file names, tags or text, and they carry only document and vault IDs. The app can fetch and show the title on the device itself, if the person opts in.

## Troubleshooting

- **The card shows an APNs error.** APNs refused the key (for example `InvalidProviderToken`). Check the team ID, key ID, topic and key. Sending pauses for 15 minutes, or until you save the settings again.
- **Testing against a stub.** `FOLIO_PUSH__APNS__BASE_URL=http://127.0.0.1:9443` sends every push to that address over cleartext HTTP/2 instead of Apple. It is read from the configuration only, and the settings page warns while it is set.

## Enabling push in the Apple app build

The app ships with push turned off (`FOLIO_PUSH = NO` in `apple/Folio/project.yml`). With it off, the build carries no push entitlement, never asks for notification permission and never registers with APNs, so signing works with the existing profiles. To turn it on:

1. In the Apple Developer portal, enable **Push Notifications** on the App ID `com.cedricziel.folio`. `bundle exec fastlane bootstrap` does this through the App Store Connect API.
2. Regenerate the match profiles so they carry `aps-environment`: `bundle exec fastlane bootstrap force:true` (App Store and development, iOS and macOS).
3. Set `FOLIO_PUSH: 'YES'` in `apple/Folio/project.yml`. The target then signs with `apple/Folio/Push/FolioPush.entitlements`, which adds `aps-environment` (development for Debug, production for Release) to the usual entitlements.
4. Create an APNs key under Keys in the portal and configure the server as described above.
