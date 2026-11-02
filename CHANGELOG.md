# Changelog

## [0.6.0](https://github.com/cedricziel/folio/compare/v0.5.0...v0.6.0) (2026-11-02)


### Features

* **apple:** add library workspace navigation ([2b12762](https://github.com/cedricziel/folio/commit/2b12762505281d5b0c9f4ca516a1ec724bbbd0b0))
* **apple:** add library workspace navigation ([#195](https://github.com/cedricziel/folio/issues/195)) ([cfdd0be](https://github.com/cedricziel/folio/commit/cfdd0bea147e688d0994312f278988a938985151))
* **infra:** add bounded document browse queries ([55cb15f](https://github.com/cedricziel/folio/commit/55cb15feef720abe5e19129c326f5e89b3f7434b))
* **infra:** add bounded document browse queries ([#193](https://github.com/cedricziel/folio/issues/193)) ([642aa16](https://github.com/cedricziel/folio/commit/642aa166f246536d2da0d4a104bf2562d0c4f86d))


### Bug Fixes

* **apple:** keep previews alive across navigation cancellation ([ff5b496](https://github.com/cedricziel/folio/commit/ff5b4969c48d6fbd85609af1a1def6829c5b226e))
* **apple:** keep previews alive across navigation cancellation ([#196](https://github.com/cedricziel/folio/issues/196)) ([9303974](https://github.com/cedricziel/folio/commit/9303974044da0bcd2145eec8903747e3119956a7))


### Performance Improvements

* **api:** batch document tag enrichment ([2126fc8](https://github.com/cedricziel/folio/commit/2126fc8d8b315eb088a5171a8abeb1b8b2ca1c0b))
* **api:** batch document tag enrichment ([#204](https://github.com/cedricziel/folio/issues/204)) ([64297c1](https://github.com/cedricziel/folio/commit/64297c13e206f9304c7ab8bf71e79af35490f389))

## [0.5.0](https://github.com/cedricziel/folio/releases/tag/v0.5.0) (2026-11-02)

This release expands local document processing and gives webhook actions distinct delivery identities.

### Before you upgrade

> [!WARNING]
> **Webhook delivery IDs now identify individual actions.** Use `X-Folio-Run` for run lookups and `X-Folio-Delivery` for deduplication. Finish pending runs before upgrading when deduplication must span the transition. ([#190](https://github.com/cedricziel/folio/pull/190))

### Added

* Configure OCR, embeddings, and processing through config files or environment variables. Fresh installations honor these defaults; saved settings take precedence. ([#182](https://github.com/cedricziel/folio/pull/182))
* Choose OCRmyPDF for local text recognition from scanned PDFs and images. The container includes its CLI. ([#185](https://github.com/cedricziel/folio/pull/185))
* Export opt-in browser and Apple telemetry through Folio, keeping export credentials on the server. ([#157](https://github.com/cedricziel/folio/pull/157))

### Fixed

* Stop retaining recovery codes and authenticator setup secrets in Turbo's page cache after navigation. ([#187](https://github.com/cedricziel/folio/pull/187))
* Reject malformed or omitted classification IDs without clearing existing metadata. Explicit empty values still clear the selected field. ([#189](https://github.com/cedricziel/folio/pull/189))

[Full changelog](https://github.com/cedricziel/folio/compare/v0.4.0...v0.5.0)

## [0.4.0](https://github.com/cedricziel/folio/compare/v0.3.1...v0.4.0) (2026-11-02)


### Features

* **mcp:** serve archive workflow skills ([5213781](https://github.com/cedricziel/folio/commit/5213781ff8f89aa6a03cd9bf03f75e94939a37c8))
* **mcp:** serve archive workflow skills ([#183](https://github.com/cedricziel/folio/issues/183)) ([7b0caba](https://github.com/cedricziel/folio/commit/7b0cabafd024e19086791edaf7271e9a1ff7ccea))


### Bug Fixes

* **apple:** bound File Provider staging filenames ([24cb80d](https://github.com/cedricziel/folio/commit/24cb80de6a5368be4543a03c329bb47ab340efdf))
* **apple:** derive document status from pipeline stages ([c3119ac](https://github.com/cedricziel/folio/commit/c3119ac0519aa9008ceafbb21ecec6f6b861edeb))
* **apple:** derive document status from pipeline stages ([#158](https://github.com/cedricziel/folio/issues/158)) ([599d81e](https://github.com/cedricziel/folio/commit/599d81e986a94ab18240580a5d13fdce46994e50))
* **apple:** isolate File Provider download handoffs ([5bbcc04](https://github.com/cedricziel/folio/commit/5bbcc04a02ab0d27594420482f76ca50ce4b8eae))
* **apple:** isolate File Provider download handoffs ([#179](https://github.com/cedricziel/folio/issues/179)) ([c809d9f](https://github.com/cedricziel/folio/commit/c809d9f402f89685e8a919c8a0fecd8ae8081b8f))
* **apple:** persist File Provider enumeration snapshots ([dc28896](https://github.com/cedricziel/folio/commit/dc288961135fb67ad241a0f7be3312d37d7ec64a))
* **apple:** persist File Provider enumeration snapshots ([#181](https://github.com/cedricziel/folio/issues/181)) ([1a5dc83](https://github.com/cedricziel/folio/commit/1a5dc83e0ea45cf338b5ff549587f6a31274431f))
* **apple:** report actionable File Provider download errors ([1a46a2c](https://github.com/cedricziel/folio/commit/1a46a2cae092657fc2099b116d9ae0965c43ce5d))
* **apple:** report actionable File Provider download errors ([#184](https://github.com/cedricziel/folio/issues/184)) ([d1cab5a](https://github.com/cedricziel/folio/commit/d1cab5a386c5018aaa7541cfb33bd3cc614416b5))
* **apple:** return uploaded File Provider items before processing ([8f7fde8](https://github.com/cedricziel/folio/commit/8f7fde81c21e571ce14a9873bf0b380f02e0ca97))
* **apple:** return uploaded File Provider items before processing ([#180](https://github.com/cedricziel/folio/issues/180)) ([4abb936](https://github.com/cedricziel/folio/commit/4abb936a7af4f3a6151d0aff98e1327e0c8e3615))

## [0.3.1](https://github.com/cedricziel/folio/compare/v0.3.0...v0.3.1) (2026-11-02)

### Fixed

- Preserve document loading across navigation in the Apple app. ([#112](https://github.com/cedricziel/folio/pull/112))
- Prevent the macOS app from crashing when login completes. ([#114](https://github.com/cedricziel/folio/pull/114))

## [0.3.0](https://github.com/cedricziel/folio/compare/v0.2.0...v0.3.0) (2026-11-02)


### Features

* **auth:** add TOTP two-factor and passkeys ([#80](https://github.com/cedricziel/folio/issues/80)) ([5f2a102](https://github.com/cedricziel/folio/commit/5f2a102d22cc06043f5a74b6c6dd479fc10abbfe))
* **events:** persist events in a transactional outbox ([#86](https://github.com/cedricziel/folio/issues/86)) ([98c455b](https://github.com/cedricziel/folio/commit/98c455b88ea94bc6ab4bd29a0a94efbe92987307))
* **settings:** add an mcp.enabled toggle to /settings ([44c015f](https://github.com/cedricziel/folio/commit/44c015f72b816e8f64cc36a44cd5d98ee978f321))
* **vaults:** add vault isolation and access-policy foundations ([#78](https://github.com/cedricziel/folio/issues/78)) ([9e3d017](https://github.com/cedricziel/folio/commit/9e3d017e03727942a372f6127bca61c83c4855af))
* **web:** show the signed-in user in the sidebar ([#99](https://github.com/cedricziel/folio/issues/99)) ([09462d7](https://github.com/cedricziel/folio/commit/09462d701bcd5a26f36fecdee99f9c7cba241a81))


### Bug Fixes

* **auth:** integrate current crypto and MCP changes ([107bdb8](https://github.com/cedricziel/folio/commit/107bdb8010be44746a3359cb01b8b799fa957ec3))
* **auth:** integrate MFA with current main ([bf204fe](https://github.com/cedricziel/folio/commit/bf204fef451915bf93e225b24d37e939a7031d89))
* **auth:** keep optional passkey autofill quiet ([92b321e](https://github.com/cedricziel/folio/commit/92b321ebad9d7c02dd1066dc004a157c3940b0ae))
* **auth:** make TOTP enrollment updates conditional ([19448d4](https://github.com/cedricziel/folio/commit/19448d4d4b1feadf379ddf7f0422920f3472730b))
* **auth:** report the actual MFA retry delay ([435b4e8](https://github.com/cedricziel/folio/commit/435b4e81f9f1c720ca14618d7d43340a0f1ff1c2))
* **auth:** require fresh proof before passkey enrollment ([5d719c6](https://github.com/cedricziel/folio/commit/5d719c6789389fce955055ddf6bf41d5903880d5))
* **auth:** use current OS random source API ([7dfa39d](https://github.com/cedricziel/folio/commit/7dfa39dc0e02ddec9cc9a61e20e004d55f82c914))
* **auth:** warn when configured secret replaces stored key ([96eeaae](https://github.com/cedricziel/folio/commit/96eeaae0ce68a425409737e2a8d693bf378826d6))
* **config:** redact auth.secret_key in Debug output ([f6a8399](https://github.com/cedricziel/folio/commit/f6a83990da8bfdbf2b6e08668de851e3602783ed))
* **events:** broadcast before running workflows ([4fcd65b](https://github.com/cedricziel/folio/commit/4fcd65b310018b3c540131f3cb4af303ff257271))
* **events:** emit one event per metadata save ([5674108](https://github.com/cedricziel/folio/commit/56741080efd87b0e4ead4f1549aa3d76a26198c6))
* **events:** recover concurrent broadcasts safely ([#109](https://github.com/cedricziel/folio/issues/109)) ([81faf21](https://github.com/cedricziel/folio/commit/81faf211bb3cbee78804c78b28d409507365d0e7))
* **events:** recover overlapping and pending broadcasts ([3734a03](https://github.com/cedricziel/folio/commit/3734a03293fbcd0e5d7ab859cd5fa456f5ac9500))
* **events:** replay events in dispatch order ([9d01b4e](https://github.com/cedricziel/folio/commit/9d01b4e3261f0dfe53e8dab4282056a7923d6be9))
* **infra:** serialize metadata reads with their writes ([4588fd5](https://github.com/cedricziel/folio/commit/4588fd5ef3e63dfc3f9f42e44749fab74a715a50))
* **mcp:** address the MCP review follow-ups ([#107](https://github.com/cedricziel/folio/issues/107)) ([66583c8](https://github.com/cedricziel/folio/commit/66583c808d8b464296d1c79ff120e99c8071b0b3))
* **mcp:** clear historical failures after reprocessing ([929caa9](https://github.com/cedricziel/folio/commit/929caa9111eb453784082df3b17d5a4ac1ec3d0a))
* **mcp:** integrate the transactional outbox ([813c0a2](https://github.com/cedricziel/folio/commit/813c0a27bb5710b63c2a0bf448a26c6b7624327f))
* **mcp:** preserve failures in unrelated processing stages ([ca5cf8f](https://github.com/cedricziel/folio/commit/ca5cf8fa26b353643d291102f60af4032456c5bb))
* **mcp:** preserve failures in unrelated processing stages ([#110](https://github.com/cedricziel/folio/issues/110)) ([1ae557d](https://github.com/cedricziel/folio/commit/1ae557d3f2d40535a18ab170e1664cced272f0bf))
* **vaults:** complete scoped repository integration ([44c2da8](https://github.com/cedricziel/folio/commit/44c2da8ee2470c104367e0ad33d686b6fb965b5a))
* **vaults:** integrate account security page context ([b9dc506](https://github.com/cedricziel/folio/commit/b9dc506ff87b5af0fa4aff414d1bfe5787991ba8))
* **vaults:** integrate durable events and bootstrap access ([9ac88c8](https://github.com/cedricziel/folio/commit/9ac88c81a0fcf53b5ef259e2ce815a6992587be9))
* **vaults:** integrate scoped MCP search ([b52feed](https://github.com/cedricziel/folio/commit/b52feed78b513486b2942aac621cc1728f16c92b))
* **vaults:** pass default vault through existing routes ([8875c7b](https://github.com/cedricziel/folio/commit/8875c7bf7a24a22023cbda18b1910d0785f2ec1c))
* **vaults:** pass vault context to resource rendering ([8702475](https://github.com/cedricziel/folio/commit/87024755cbf12115f5ddfb00c3f74f9676953f0c))
* **vaults:** preserve deletion snapshots across upgrades ([f419678](https://github.com/cedricziel/folio/commit/f419678000524cbb94d77efd3e351178a740c649))
* **vaults:** resolve integration with current main ([c29078e](https://github.com/cedricziel/folio/commit/c29078e60b3d3890ce6c0403cbd542ff082bac19))


### Performance Improvements

* **mcp:** cache the mcp.enabled setting instead of reading it per request ([41f3c2b](https://github.com/cedricziel/folio/commit/41f3c2b3a38d34c6927dd0d0a264d8645881922f))
* **mcp:** search by ids and read the tag table once ([0147fed](https://github.com/cedricziel/folio/commit/0147fed4199df6e9bd966970ac53646c7827c352))

## [0.2.0](https://github.com/cedricziel/folio/compare/v0.1.1...v0.2.0) (2026-11-01)


### Features

* **apple:** link the deployment guide from onboarding ([6b1e5f9](https://github.com/cedricziel/folio/commit/6b1e5f96686af3e97266ca82364ff2c34262fdc3))


### Bug Fixes

* **api:** resolve API and worker backlog issues ([#94](https://github.com/cedricziel/folio/issues/94)) ([f0fc77e](https://github.com/cedricziel/folio/commit/f0fc77e1c1104861ab652cf7020d2cd40213c5af))
* **config:** let the shipped config follow auth for MCP ([70c2337](https://github.com/cedricziel/folio/commit/70c2337d3b6c587532fc24f5b724355fc1e98826))
* **config:** let the shipped config follow auth for MCP ([#96](https://github.com/cedricziel/folio/issues/96)) ([3f90966](https://github.com/cedricziel/folio/commit/3f909660584a87d1f4c51948d0bbf891f369e778))
* **docker:** install ghostscript and allow the PDF coder for the ImageMagick fallback ([18ace36](https://github.com/cedricziel/folio/commit/18ace3657e8fb4f51fc14e9eb4888dee982b2cf2))
* **docker:** make the PDF fallback work in the image ([#98](https://github.com/cedricziel/folio/issues/98)) ([0f5fe7a](https://github.com/cedricziel/folio/commit/0f5fe7a55e2787adbe5581efee523aa58d75b45f))

## [0.1.1](https://github.com/cedricziel/folio/compare/v0.1.0...v0.1.1) (2026-11-01)


### Bug Fixes

* **apple:** take app and build versions from the build settings ([#87](https://github.com/cedricziel/folio/issues/87)) ([42d4819](https://github.com/cedricziel/folio/commit/42d481982e6bc28f2ded0eb525a39c5957864359))

## [0.1.0](https://github.com/cedricziel/folio/compare/v0.1.0-alpha.2...v0.1.0) (2026-11-01)


### Features

* **api:** include tags and filters in the document list API ([#83](https://github.com/cedricziel/folio/issues/83)) ([839115c](https://github.com/cedricziel/folio/commit/839115ceeb2840c4a203bbb4536af98e6276ed3e))
* **api:** include tags and names in documents and filter the list by tag and date ([e29d70d](https://github.com/cedricziel/folio/commit/e29d70dc5fa625e24aa2fdbc9e32ef65fcee1e6e))
* **apple:** read tags from the API and filter documents on the server ([da3a6e3](https://github.com/cedricziel/folio/commit/da3a6e392d854f93427eec5315ae4ff1e70b8867))


### Bug Fixes

* **apple:** declare the File Provider document group ([bcb113b](https://github.com/cedricziel/folio/commit/bcb113b63c0ea48662d4d3dfe5cd1840cd2caacc))
* **apple:** declare the File Provider document group ([#74](https://github.com/cedricziel/folio/issues/74)) ([7253c60](https://github.com/cedricziel/folio/commit/7253c604b1263b4487395a9b2404e4ca51622bd3))
* **apple:** derive build numbers from the UTC build time ([2ce54c5](https://github.com/cedricziel/folio/commit/2ce54c5d9657ef8c859ef7208e4678aa090f39d9))
* **apple:** tell gym the macOS build targets the macosx SDK ([0e0fbfd](https://github.com/cedricziel/folio/commit/0e0fbfdb79161235af694acc06ab6e6020e6ac8b))
* **apple:** tell gym the macOS build targets the macosx SDK ([#75](https://github.com/cedricziel/folio/issues/75)) ([6a90528](https://github.com/cedricziel/folio/commit/6a90528428ba8daaa583a015a5d27678a937e130))
* **apple:** use the commit count as the build number ([52ac2de](https://github.com/cedricziel/folio/commit/52ac2defbe2f3fec60041105bb9f375eefe294ef))
* **apple:** use the commit count as the build number ([#85](https://github.com/cedricziel/folio/issues/85)) ([478a6f5](https://github.com/cedricziel/folio/commit/478a6f5eaae4ecfe1626bd389642061d570c1c80))
* **web:** address the UX review findings ([#77](https://github.com/cedricziel/folio/issues/77)) ([340b3fc](https://github.com/cedricziel/folio/commit/340b3fc81f5f57b3761440f9541dab96b181aeea))
* **web:** neutral generated placeholders, tidy palette dates and list meta ([46157ca](https://github.com/cedricziel/folio/commit/46157caca78d1879bb08db9e915fe85b26e70183))

## [0.1.0-alpha.2](https://github.com/cedricziel/folio/compare/v0.1.0-alpha.1...v0.1.0-alpha.2) (2026-11-01)


### Features

* add a workflow engine with signed webhooks ([#22](https://github.com/cedricziel/folio/issues/22)) ([cf28f10](https://github.com/cedricziel/folio/commit/cf28f10ae532922c4925958944ea6f24fae84902))
* **api:** add /healthz and /readyz probes and a container healthcheck ([#33](https://github.com/cedricziel/folio/issues/33)) ([c145374](https://github.com/cedricziel/folio/commit/c145374fbae9831a780a20ea6023cfc002f38ab4))
* **api:** expose workflows, test deliveries, and runs over the JSON API ([3268765](https://github.com/cedricziel/folio/commit/32687650c1f4d76257fa33e7f4abe3783093eca7))
* **api:** manage ingestion sources in the API and a Sources settings page ([1ab28b8](https://github.com/cedricziel/folio/commit/1ab28b88b9cd73950995718bc4e78573e0a702a8))
* **api:** serve unauthenticated /healthz and /readyz probes ([7574d98](https://github.com/cedricziel/folio/commit/7574d98ac9d079cb3ca93e12e9f15bb69620fe2a))
* **api:** stream downloads and thumbnails from the store and purge objects on delete ([8cd0789](https://github.com/cedricziel/folio/commit/8cd0789579a98f2a4020125167c2a7ebf7d2e7fb))
* **api:** trace and measure HTTP requests with semantic-convention attributes ([61c9371](https://github.com/cedricziel/folio/commit/61c93712f361bcf6bb19f687fca90c984ac05114))
* **app:** add healthcheck subcommand ([80255bc](https://github.com/cedricziel/folio/commit/80255bc2fa898818b5b6df61f838dcea27b7a934))
* **apple:** add App Intents and entities for documents, tags, correspondents and types ([319d1be](https://github.com/cedricziel/folio/commit/319d1be1a9b96b263cf60a49399571723b508b03))
* **apple:** add App Intents for Shortcuts and Siri ([#68](https://github.com/cedricziel/folio/issues/68)) ([7a3e181](https://github.com/cedricziel/folio/commit/7a3e1812168dd29652d642cb018a86bf8b89101e))
* **apple:** add DocumentCatalog for Shortcuts and Siri ([59c346d](https://github.com/cedricziel/folio/commit/59c346d5c6600a6b2ceb4c40e09e08668af2b07e))
* **apple:** add keyboard commands, an inbox scope and intent routing ([bf5ccdf](https://github.com/cedricziel/folio/commit/bf5ccdf166bb49a82c9879985a2e84178ddc9db2))
* **apple:** add removeTag to the Folio service ([1d4366c](https://github.com/cedricziel/folio/commit/1d4366c418c368a7c41b152c2627525f9ceb2ef1))
* **apple:** add share and file provider extensions ([#30](https://github.com/cedricziel/folio/issues/30)) ([a43413f](https://github.com/cedricziel/folio/commit/a43413fe918aec6e713ccdc6133b78f540b294f6))
* **apple:** add share and file provider extensions for iOS and macOS ([8b2914e](https://github.com/cedricziel/folio/commit/8b2914e9c15594c31e2c49c12e88bcb53593a5f6))
* **apple:** declare Siri phrases and index viewed documents in Spotlight ([de1a88c](https://github.com/cedricziel/folio/commit/de1a88c095e666912f61df15f8df7dd7d4685d9a))
* **apple:** scaffold native iOS and macOS app ([#25](https://github.com/cedricziel/folio/issues/25)) ([fb7c124](https://github.com/cedricziel/folio/commit/fb7c1247d488607866907a53133c018ff955ea2c))
* **apple:** sign in with OAuth 2.1 and PKCE ([3e3fa20](https://github.com/cedricziel/folio/commit/3e3fa20e4c068dc4e4b24113f20d3a721065776f))
* **apple:** sign in with OAuth 2.1 and PKCE ([#53](https://github.com/cedricziel/folio/issues/53)) ([3051cb4](https://github.com/cedricziel/folio/commit/3051cb4207877531703dde07e4612c4fe898e53c))
* **app:** serve MCP over HTTP at /mcp and stdio via 'mcp --stdio' ([0616288](https://github.com/cedricziel/folio/commit/0616288b0a96bfc1bf851cf88b267b4007d72db9))
* **app:** start telemetry on boot and flush it on shutdown ([2909ce4](https://github.com/cedricziel/folio/commit/2909ce4eca970cca85ce80edcec3fb03f3585b2a))
* **app:** wire ingestion, record sources on uploads, and show them on documents ([c78f706](https://github.com/cedricziel/folio/commit/c78f7061cba30e89f30ff273c0f8d60aee049640))
* **app:** wire the workflow engine, runner, and public URL setting ([d04cff8](https://github.com/cedricziel/folio/commit/d04cff87e37ddc02500ad3734feee4b336ef9e5e))
* **auth:** add forward auth login with trusted proxies and groups ([#52](https://github.com/cedricziel/folio/issues/52)) ([e161a3d](https://github.com/cedricziel/folio/commit/e161a3dafc39de2373aee2b97ed71b96835af8e3))
* **auth:** add forward auth tests, Postgres store and docs ([3dd5b43](https://github.com/cedricziel/folio/commit/3dd5b4324b75bf5186a47ff475b5ab6a57b42b9c))
* **auth:** add forward auth, external users and groups ([59fa724](https://github.com/cedricziel/folio/commit/59fa724db70bc6b5714c07ddc4f03d1c5628d9c1))
* **auth:** add the OpenID Connect relying party ([963cd59](https://github.com/cedricziel/folio/commit/963cd59cc6148fdf3071ef15d780fcd4666c9de1))
* **auth:** keep the OIDC ID token with the session ([fc3fe8b](https://github.com/cedricziel/folio/commit/fc3fe8b8633dcc9aa24fac82eebb2c993ab3fabd))
* **auth:** link forward-auth accounts by a stable id header when configured ([d3f2584](https://github.com/cedricziel/folio/commit/d3f258470e269ac366a75451dcf77b2bd3bcbba9))
* **auth:** optionally require a shared secret from the forward-auth proxy ([c69a83f](https://github.com/cedricziel/folio/commit/c69a83f2810c309136096b9a25dbaf1dd638cdf0))
* **auth:** require login for the web UI and API (auth.required) ([5100c70](https://github.com/cedricziel/folio/commit/5100c70b32adb2b4fcae5fbd491453a42fc7ff7d))
* **auth:** sign in with OpenID Connect ([12fddec](https://github.com/cedricziel/folio/commit/12fddec1693a25b25456c2ab1c2604dea30b3476))
* **auth:** sign in with OpenID Connect ([#54](https://github.com/cedricziel/folio/issues/54)) ([aecda2e](https://github.com/cedricziel/folio/commit/aecda2e6b9d1af02f952622e5f9ea0673aef0f58))
* **auth:** store in-flight OIDC logins ([93155a8](https://github.com/cedricziel/folio/commit/93155a8bbfbbae21d152823820e9042e26af3aa6))
* **config:** add auth.oidc settings ([1d641c9](https://github.com/cedricziel/folio/commit/1d641c90384b1eb47f161d66c06750f44abf64ad))
* **core:** add library service for assistant-facing archive operations ([f5ea9c5](https://github.com/cedricziel/folio/commit/f5ea9c5bb862360db100ae6689c19252e7705b28))
* **core:** add the DocumentStore port and object key layout ([3f49649](https://github.com/cedricziel/folio/commit/3f49649545ae8afef1bca7f36f1d4083348c1da1))
* **core:** add the workflow engine ([1c2901a](https://github.com/cedricziel/folio/commit/1c2901aa36d87f38654fe0dee071652482982eb3))
* **core:** add workflow domain model and ports ([90b8ac6](https://github.com/cedricziel/folio/commit/90b8ac6fec0bf8b65ad25e9613410427033dba67))
* **core:** emit document lifecycle events from the repository ([5231368](https://github.com/cedricziel/folio/commit/5231368e9de310415d6988ff02979c8b150ae305))
* **core:** model document sources, ingestion sources, and the import pipeline ([9fb897b](https://github.com/cedricziel/folio/commit/9fb897bb199fd2bd2ddad5f2ca56a57eada3459f))
* **db:** add PostgreSQL adapters selected by the database URL scheme ([64d9d6b](https://github.com/cedricziel/folio/commit/64d9d6beef6216c4a0fc67730bb0f525f2ed4130))
* **db:** support PostgreSQL alongside SQLite ([#38](https://github.com/cedricziel/folio/issues/38)) ([c9c95c6](https://github.com/cedricziel/folio/commit/c9c95c68c8ecc43b184a131a0370c87e0365a3cf))
* **e2e:** add Playwright suite for the document lifecycle ([ee096b8](https://github.com/cedricziel/folio/commit/ee096b850b3a60c0ba4995fb1eb6c32189adc5c4))
* **e2e:** add the verify skill and a Playwright e2e suite ([#36](https://github.com/cedricziel/folio/issues/36)) ([1144d24](https://github.com/cedricziel/folio/commit/1144d242344b4726e85a0a7d3a59867e572371e7))
* **infra:** add HealthCheck port and database adapter ([cdfb54b](https://github.com/cedricziel/folio/commit/cdfb54b772928cc36e5fb487fb2224478064acde))
* **infra:** add mcp configuration section ([19f74ee](https://github.com/cedricziel/folio/commit/19f74ee916916345fb7dd195f3d2975555187cc5))
* **infra:** honour tag, text, and date filters when listing documents ([f526ae6](https://github.com/cedricziel/folio/commit/f526ae6096517834669348ec863307d49be45c9c))
* **infra:** import attachments from IMAP mailboxes ([40c54f6](https://github.com/cedricziel/folio/commit/40c54f6a5481c42c8a76ad645b15b7f7d7454bf7))
* **infra:** import files from a watched consume folder ([dd21da5](https://github.com/cedricziel/folio/commit/dd21da594a2067a3a17663d79d64640b3693531f))
* **infra:** load ingestion sources from configuration and build adapters ([6837c3f](https://github.com/cedricziel/folio/commit/6837c3fdf280a12b636eb6d275aa13e3d78202b5))
* **infra:** persist document sources and the ingestion ledger ([9e6e00d](https://github.com/cedricziel/folio/commit/9e6e00d72381b9a91f38b1b85290266023550aa6))
* **infra:** persist workflows and deliver signed webhooks via the job queue ([9393bf0](https://github.com/cedricziel/folio/commit/9393bf0fa53d85611e26fdd5e73e513d3cd66e46))
* **infra:** require every search term to match a document ([c44ec02](https://github.com/cedricziel/folio/commit/c44ec0204874c00f4621cc62765d9de75ed0b729))
* **infra:** trace jobs, AI provider calls and storage operations ([c286dd7](https://github.com/cedricziel/folio/commit/c286dd7a545d06bea5adc77150b6c283002bb0a1))
* ingest documents from a consume folder and IMAP mailboxes ([#45](https://github.com/cedricziel/folio/issues/45)) ([85171c0](https://github.com/cedricziel/folio/commit/85171c0cc453b31939468f262bcca96869b5c182))
* **mcp:** add MCP server crate with archive tools and resources ([84c75ea](https://github.com/cedricziel/folio/commit/84c75ead11c778f6aef6ee0ffdbce9a3c30ab322))
* **mcp:** add MCP server for AI assistants ([#31](https://github.com/cedricziel/folio/issues/31)) ([7c158bf](https://github.com/cedricziel/folio/commit/7c158bf18a9d4642f6f660b490633b0fafc8ded9))
* **pipeline:** pluggable OCR and embedding providers ([#35](https://github.com/cedricziel/folio/issues/35)) ([324e8df](https://github.com/cedricziel/folio/commit/324e8df585a40675fce3e4049fdfdf3f2bf4e4ba))
* **skill:** add a screen tour with screenshots and axe checks to verify ([e2a97a6](https://github.com/cedricziel/folio/commit/e2a97a61b6864de9b1313c6b192c3c9cbe873645))
* **storage:** add a storage port with local and S3 adapters ([#43](https://github.com/cedricziel/folio/issues/43)) ([fba5375](https://github.com/cedricziel/folio/commit/fba5375b06e7367391a7e512ab3234da2e625750))
* **storage:** add local and S3 document stores over object_store ([51d6016](https://github.com/cedricziel/folio/commit/51d601690b10896f590a00a9cd5a7f9880e57102))
* **telemetry:** export traces, metrics and logs with OpenTelemetry ([#47](https://github.com/cedricziel/folio/issues/47)) ([509513e](https://github.com/cedricziel/folio/commit/509513e65c68375824924e2841174f8780bb6ef7))
* **telemetry:** initialise OTLP export from the standard OTEL variables ([bcda36a](https://github.com/cedricziel/folio/commit/bcda36af567ffe23c424b3464a396d429badb78b))
* **verify:** add scripts to run an ephemeral Folio instance ([a3cce68](https://github.com/cedricziel/folio/commit/a3cce68bbd09e025cb33aa342cf6e94149644007))
* **web:** add a workflows page with create, edit, test, and run history ([211af76](https://github.com/cedricziel/folio/commit/211af769cf15f95596a251c4d697e3fbf4838274))
* **web:** add keyboard shortcuts ([#67](https://github.com/cedricziel/folio/issues/67)) ([189e1b8](https://github.com/cedricziel/folio/commit/189e1b8d32b002bff4a2c84c84236213fc6ceb83))
* **web:** add keyboard shortcuts with a help dialog and in-page delete confirm ([98ea0c3](https://github.com/cedricziel/folio/commit/98ea0c37f8a834c9e738eeab23473670e8b1ee52))
* **web:** reskin the web UI with the Folio design system and ship it as a PWA ([#34](https://github.com/cedricziel/folio/issues/34)) ([988022e](https://github.com/cedricziel/folio/commit/988022ec0fd64942912ada11e6bb84d375a82e8e))


### Bug Fixes

* **api:** mask webhook header values, hide URL credentials, and use axum 0.8 routes ([d685906](https://github.com/cedricziel/folio/commit/d685906348efcea45831fbcbfd34415b2c42404f))
* **api:** serve uploaded files with nosniff, a sandbox CSP and attachment for active types ([7e76361](https://github.com/cedricziel/folio/commit/7e7636132ea799deb2ae2642d12f598e7c491229)), closes [#48](https://github.com/cedricziel/folio/issues/48)
* **apple:** add the app icon and export macOS builds as pkg ([#69](https://github.com/cedricziel/folio/issues/69)) ([92aa948](https://github.com/cedricziel/folio/commit/92aa948a07b06cbb733b5754ae40ddbf251afb47))
* **apple:** add the app icon generated from the design mark ([171d1dd](https://github.com/cedricziel/folio/commit/171d1dd9c0682dd445a9ff715cdc4312e3f9d361))
* **apple:** export macOS builds as a sandboxed installer pkg ([fcf67dd](https://github.com/cedricziel/folio/commit/fcf67ddd2f0f29090305f5a1e9ab4d391bdccfdf))
* **apple:** harden downloads, search state, uploads and file picking ([93bcd6a](https://github.com/cedricziel/folio/commit/93bcd6af0fad3ebfd18b2edd1551746c7fddc94a))
* **apple:** support all iPad orientations for multitasking ([0952631](https://github.com/cedricziel/folio/commit/09526319f7c9bc2799028fe99f90931e643801b7))
* **apple:** support all iPad orientations for multitasking ([#57](https://github.com/cedricziel/folio/issues/57)) ([429b777](https://github.com/cedricziel/folio/commit/429b7778822f0cb7982bc55a2908d467fe5c3fd9))
* **apple:** wire AppIcon into the Folio target ([b540226](https://github.com/cedricziel/folio/commit/b54022694dc4bbfcc6121a1181edbca41f398db7))
* **auth:** add the RFC 9207 iss parameter to authorization responses ([2e68e9c](https://github.com/cedricziel/folio/commit/2e68e9c16ca505acd121897b67d7075f94b085f9))
* **auth:** bind CSRF tokens to the session with an HMAC and use __Host- cookies over https ([8b9ff88](https://github.com/cedricziel/folio/commit/8b9ff888be251c1b27edc061b18bc542351c8d9f))
* **auth:** bound dynamic client registration and purge unused clients periodically ([f10ef6c](https://github.com/cedricziel/folio/commit/f10ef6c7b7c952c8a79b2ea80b8d61f8c47f6efd))
* **auth:** configurable group separator and a warning for catch-all trusted proxies ([44c1a40](https://github.com/cedricziel/folio/commit/44c1a4073d7fdb1c85a60d94fb5db5f12b7d6371))
* **auth:** create the first external user atomically ([bc3a096](https://github.com/cedricziel/folio/commit/bc3a0968f269855bafa0c753f2fa0c1aea495236))
* **auth:** delete the previous session on login ([000d2ff](https://github.com/cedricziel/folio/commit/000d2ff7c5ceeb0310b1c6e0ca3654d13ec45370))
* **auth:** derive issuer and cookie flags from public_url only and require it when exposed ([e203120](https://github.com/cedricziel/folio/commit/e20312023b9e2a89289c8ecc305054a635bb93e9))
* **auth:** enforce documents:read/write scopes on bearer tokens for /api ([4bd7850](https://github.com/cedricziel/folio/commit/4bd78502dc7cc449f90285486a09c0d73bc29006))
* **auth:** forbid framing and caching of login, setup and consent pages ([004984c](https://github.com/cedricziel/folio/commit/004984cf254addd9e30e5599c589d2f42136289c))
* **auth:** group tokens into grants; revoke the family on refresh reuse and revoke ([dc4dd1d](https://github.com/cedricziel/folio/commit/dc4dd1dfe50e40a104ef3cbc5667330d64c4157d))
* **auth:** harden OIDC token validation and discovery ([8612301](https://github.com/cedricziel/folio/commit/86123019f8dc0a3fa8c46eabde49ef5f0718efdb))
* **auth:** harden the authorization server and login ([#55](https://github.com/cedricziel/folio/issues/55)) ([19312ba](https://github.com/cedricziel/folio/commit/19312bab207e28442e907d69253476cf9afc1a75)), closes [#48](https://github.com/cedricziel/folio/issues/48)
* **auth:** ignore forward headers on OAuth, MCP and bearer-token requests ([97e6215](https://github.com/cedricziel/folio/commit/97e621598ef6a91f266aab01cd5fb7078f25a3e6))
* **auth:** ignore forwarded groups unless configured and add role-based admin ([d50b486](https://github.com/cedricziel/folio/commit/d50b48614fce39b2bcb8376ae9244112001de555))
* **auth:** keep a permanent owner that group and role mappings cannot demote ([26ab192](https://github.com/cedricziel/folio/commit/26ab192021861c202d8e34b9bad3d4634bc2e658))
* **auth:** keep PWA assets public, refuse anonymous shares, serve /api resource metadata ([72bb5a9](https://github.com/cedricziel/folio/commit/72bb5a99162015f87c1372bd063db18c06bec407))
* **auth:** let concurrent first forward-auth requests share one account ([03451f7](https://github.com/cedricziel/folio/commit/03451f7941f5e6d65935b5ba120af3ee3752fd60))
* **auth:** make first-user creation atomic on PostgreSQL ([67053e1](https://github.com/cedricziel/folio/commit/67053e17207907762a6b15b50f2445f6bd9a1539))
* **auth:** move the owner migration to 20261101220000 to avoid version clashes ([4b77b2b](https://github.com/cedricziel/folio/commit/4b77b2b9ba977c8cac1bb4165d43e09e99b61b5e))
* **auth:** purge expired auth data in the background ([44e3fef](https://github.com/cedricziel/folio/commit/44e3feff03ce99a9443d5f39019aad291cab84fc))
* **auth:** rate limit OIDC starts, bind the cookie to the host and hint the provider at logout ([469533f](https://github.com/cedricziel/folio/commit/469533f9afb26a3b28efc07976338d9346975c34))
* **auth:** reject cross-origin writes that rely on the session cookie ([926a882](https://github.com/cedricziel/folio/commit/926a88295621d81bb47c1699d372c8b88e5b3bed))
* **auth:** reject requests that repeat a forward identity header ([cd3ea1a](https://github.com/cedricziel/folio/commit/cd3ea1a56a99171a71d8cf34fe0828d574289b8b))
* **auth:** renumber the OIDC migrations after main's ingestion ([0693a2d](https://github.com/cedricziel/folio/commit/0693a2d534c4183371541d4aa550ba6eb07ec41a))
* **auth:** renumber the owner migration past main's ingestion one and satisfy newer clippy ([7421ed2](https://github.com/cedricziel/folio/commit/7421ed27cd2c4358bffa61053783ee47593ddaf3))
* **auth:** require a one-time setup token while no user exists ([25ac050](https://github.com/cedricziel/folio/commit/25ac05011b8edc24bd29e347eb252b32f46842c0))
* **auth:** serialise external group syncs per user ([0f27067](https://github.com/cedricziel/folio/commit/0f27067d82f36d64640cf0f81275d6174e8bc6db))
* **auth:** shorter, reused and revocable forward-auth sessions ([293fe2f](https://github.com/cedricziel/folio/commit/293fe2f0df222d94e5baaf1bdb67d8c5ee56bee4))
* **auth:** strip every forward identity header from untrusted peers ([5271c46](https://github.com/cedricziel/folio/commit/5271c4638f14a077bb4067ccc8ea4e03fb2d1a0c))
* **auth:** throttle failed logins, cap password length and concurrent hashing ([712c6b8](https://github.com/cedricziel/folio/commit/712c6b8064b6d19562a6bec0904cc245cb643090))
* **config:** read nested settings from double-underscore env vars ([685f920](https://github.com/cedricziel/folio/commit/685f9200aa32036adcc9d58cce68915ffe0d0b13))
* **config:** read nested settings from double-underscore env vars ([#26](https://github.com/cedricziel/folio/issues/26)) ([b6f24ab](https://github.com/cedricziel/folio/commit/b6f24abcd989934ac719ba2242aae1fb57afa774))
* **db:** drop duplicate health module declaration after rebase ([551cf93](https://github.com/cedricziel/folio/commit/551cf9320e9f6b3fcf64cec37e69fc862e55d5a7))
* **db:** give the OAuth grants migration a unique version ([72c1285](https://github.com/cedricziel/folio/commit/72c1285e437f9a8cc0fa96fb7666878daad9f557))
* **infra:** dedupe once-per-document runs with a unique index ([b7488dd](https://github.com/cedricziel/folio/commit/b7488dd43da2f920e3404f7524d0a720e1735415))
* **mcp:** disable the endpoint by default and warn when it is enabled ([cf44377](https://github.com/cedricziel/folio/commit/cf44377a0659f9d696f8b6e5854f0b65b3a0f8c0))
* **mcp:** rank exact matches first, validate tag names before writing, and report server identity ([8502fe2](https://github.com/cedricziel/folio/commit/8502fe2716c5596cd877dfacf2f78076307a326f))
* **pipeline:** adapt to sqlx 0.9 and clippy, simplify test outcome template ([02a3687](https://github.com/cedricziel/folio/commit/02a36875f8978d0b40e2e41163e35d6b86ca7476))
* **search:** fall back to text matching and drive the badge from stage status ([21a80da](https://github.com/cedricziel/folio/commit/21a80da594993fabbe7fa483265d872e8e7b2ef3))
* **skill:** poll page_count and export VERIFY_DIR ([034c89c](https://github.com/cedricziel/folio/commit/034c89c47277e15164ddd528d350b64e89908e53))
* **web:** call macros with the askama 0.16 syntax ([4be80a8](https://github.com/cedricziel/folio/commit/4be80a8f1dd3344872b29309cd9b1db90ff4e1f2))

## [0.1.0-alpha.1](https://github.com/cedricziel/folio/compare/v0.1.0...v0.1.0-alpha.1) (2026-11-01)


### Features

* add background job queue and document thumbnails ([697dba9](https://github.com/cedricziel/folio/commit/697dba917386ee01ece30ed67f1898b9f7fae8f5))
* add command palette with ⌘K shortcut ([b47d570](https://github.com/cedricziel/folio/commit/b47d570aec6099ed47761cb66768cf70d928ec9d))
* add database-stored settings with AI configuration UI ([32358d5](https://github.com/cedricziel/folio/commit/32358d57f041994edc18849e04e1627e7e312ec8))
* add document metadata management with correspondents, types, and tags ([492d4bc](https://github.com/cedricziel/folio/commit/492d4bc7fbc33058cb27a9b026c3d4bc9a7f6d4f))
* add drag-and-drop upload and livereload support ([8b00a8b](https://github.com/cedricziel/folio/commit/8b00a8b8760c611f2dbdbcf1aed25e0718b7f49f))
* add dynamic AI services that read from database settings ([c88e2ee](https://github.com/cedricziel/folio/commit/c88e2ee13eff40ee37d7f6edec5e5f9842798900))
* add full-page document view with editorial design ([2b661c5](https://github.com/cedricziel/folio/commit/2b661c5830d825e6b6ab8f1089cfcd730cda11bc))
* add live search with debouncing ([8286d79](https://github.com/cedricziel/folio/commit/8286d79145d070e2f3b9926263514300e7a4346e))
* add multi-page document viewing and reprocess button ([ac7f1f3](https://github.com/cedricziel/folio/commit/ac7f1f3991eb53aa613bffe583624de3e364c667))
* add SSE for real-time document updates ([85fa8f4](https://github.com/cedricziel/folio/commit/85fa8f4c93bf8b42ce9493a712d605f9d5f72c63))
* **api:** publish OpenAPI spec for the JSON API ([6dca93c](https://github.com/cedricziel/folio/commit/6dca93cc3aefa409f6de4d4bc7e7ba948e9ce49a))
* bootstrap folio document management system ([2ee8ab9](https://github.com/cedricziel/folio/commit/2ee8ab97e752f116464e277c2d565c68b12eafbf))
* **core:** derive ToSchema on API-facing types behind an openapi feature ([892660b](https://github.com/cedricziel/folio/commit/892660b84387038db1235025219aba6708428ec3))
* **design:** add Folio design tokens and component styles ([aadebd4](https://github.com/cedricziel/folio/commit/aadebd406dc5ad8a116050dc2d1915fb8cd9533a))
* improve UX with layout redesign and self-repair mechanism ([7f170a7](https://github.com/cedricziel/folio/commit/7f170a737847b41018803ee43b1b8430cf498010))
* increase PDF OCR resolution to 300 DPI for better text extraction ([9609588](https://github.com/cedricziel/folio/commit/9609588d0f2a096970c5d5d4e970d5ac933ca05c))


### Bug Fixes

* **api:** clear tag color when color is null ([237260b](https://github.com/cedricziel/folio/commit/237260b0a052d080ad647cd14aea71ad2fcc109e))
* **api:** point the spec drift message at the right command ([c9448ba](https://github.com/cedricziel/folio/commit/c9448ba7c2aa09eebcc9de93ff8e6040f189123a))
* **api:** use axum 0.7 path syntax for id routes ([f2d50d2](https://github.com/cedricziel/folio/commit/f2d50d25e8914d9bbfade60e6f5b3a66bb02c40c))
* **api:** use axum 0.8 path syntax for id routes ([5f5883a](https://github.com/cedricziel/folio/commit/5f5883ace2ca5b66a42f9eb9b97087a0b7cf0942))
* convert PDFs to images before OCR ([a99298c](https://github.com/cedricziel/folio/commit/a99298c1de51d58464f59ad1c3fd6074d97473af))
* handle pdftoppm zero-padded output filenames ([cd9065f](https://github.com/cedricziel/folio/commit/cd9065fb0b4d933f4ee04bdb48717491487bdf0e))
