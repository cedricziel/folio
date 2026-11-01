# UX review, November 2026

An independent review of the Folio web app (Askama + Hotwire, installable PWA) against the Folio design system (`design/README.md`, `design/tokens.json`, `design/components.css`) and the screens on the design canvas (Main, Document, Login, Setup, Workflows, Processing, PwaInbox, InboxDark, EmptyInbox, CommandPalette). The canvas was used for intent, not pixels.

Reviewed on `main` at `a87944f`; the tour and the e2e checks were re-run after rebasing onto `e161a3d` (auth hardening, setup token, forward auth).

## How it was done

- Two ephemeral instances from the verify skill: auth off, and auth on (`FOLIO_VERIFY_AUTH_REQUIRED=true`) for setup, sign in, sign-in error, OAuth consent (dynamic client registration plus `/oauth/authorize`) and the OAuth error page.
- Deterministic demo data from `.claude/skills/verify/scripts/seed.cjs`: four correspondents, four types, six tags, seven generated PDFs (one with three pages, one with an 89-character title), a workflow with tag and correspondent actions, a disabled folder source, and one upload that is not a PDF, so its processing fails.
- The screen tour (`.claude/skills/verify/scripts/screens.mjs`, screens in `.claude/skills/verify/screens.json`) captured 25 screens at 1280×800, 768×1024 and 390×844 in light and dark, 290 captures in total, each with an axe-core 4.14 run (WCAG 2.0/2.1/2.2 A and AA tags), a horizontal overflow check and the console errors.
- A keyboard pass at 1280px and 390px: tab order, focus visibility, the skip link, ⌘K, the upload and tag dialogs, and the mobile drawer.

Screenshot names below follow the tour's `<screen>--<viewport>--<theme>.png` pattern. To reproduce them:

```bash
S=.claude/skills/verify/scripts
$S/with-instance.sh bash -c "node $S/screens.mjs \$FOLIO_VERIFY_ENV --seed --out /tmp/folio-screens"
FOLIO_VERIFY_AUTH_REQUIRED=true $S/with-instance.sh bash -c "node $S/screens.mjs \$FOLIO_VERIFY_ENV --seed --out /tmp/folio-screens-auth"
```

## Summary

| Severity | Count | Issues                                                     |
| -------- | ----- | ---------------------------------------------------------- |
| Blocker  | 2     | #58, #59                                                   |
| Major    | 6     | #60, #61, #62, #63, #64, #65                               |
| Minor    | 12    | none filed, fix with the nearest issue or in a polish pass |
| Polish   | 9     | none filed                                                 |

What works: the inbox, the document page and Manage follow the system closely. One `title` per screen, `archive` only on primary actions, links, the selected nav item and focus, `manila` only on the inbox count, ISO dates in `meta`, statuses as words with a dot, sentence case throughout. Every interactive element shows a 2px `archive` focus ring with offset, the skip link works, mobile controls in the top bar and the drawer reach 44px, nothing scrolls horizontally at 390px, and dark mode has no axe contrast failures.

## Blockers

### 1. A file Folio could not read is shown as "Needs a look" and "Reading…" forever (#58)

- Screens: `inbox`, `all-documents`, `document-failed` (all viewports and themes).
- What you see: the broken upload `scan_0042` shows **Needs a look** in the list and counts toward "2 documents need a look. Folio read them but could not tell where to file them." The document page says "1 page", **Reading 1 page…** and "The text appears here once Folio has read the file." indefinitely. The page sheet shows the generic red file icon at page size.
- Rules broken: "Calm states … `danger` for 'Couldn't read this file'"; AI and status copy must not state things that are not true.
- Fix: add `DocumentStatus::Failed` in `folio-core` when `ocr_status` is failed; render `f-status f-status-danger` "Couldn't read this file" in `partials/document_row.html`; exclude failed rows from `counts::review_notice`; on the document page show an `f-notice f-notice-danger` and make **Read again** the primary action; hide the page count when unknown; use a neutral `paper-sunken` thumbnail fallback.

### 2. Workflow and source lists overlap and use unstyled links (#59)

- Screens: `workflows`, `sources` (all viewports and themes).
- What you see: the row text sits in the 48px thumbnail column of `.f-row`, so the trigger reads "docu…" and the source "Fold… / Not …"; **Turn off** and **Test** overlap the title. Titles are default browser links (#0000ee, underlined). "New workflow" is a full-width primary bar with underlined text; Sources has two primary buttons side by side.
- Rules broken: one accent action per view; `archive` for links only, no browser defaults; hierarchy.
- Fix: a `.f-row-plain { grid-template-columns: minmax(0, 1fr) auto; }` modifier in `design/components.css`; `a.f-row-title { color: var(--ink); text-decoration: none; }`; `a.f-btn { text-decoration: none; }`; `.page-head` as a wrapping flex row with the action pushed right; a single primary on Sources.

## Major

### 3. HTML routes return JSON or an empty body for 404 (#60)

- Screens: `not-found` (`/documents/<unknown id>`), `unknown-route` (`/does-not-exist`).
- What you see: `{"error":"Document not found"}` in the browser's JSON viewer, or the browser's own error page for an empty 404. axe: `document-title` and `html-has-lang` (serious). With auth on, an unknown route answers 401 instead of redirecting to sign in.
- Fix: an `error.html` template extending `base.html` ("This document isn't here. It may have been deleted." with a link to the inbox), HTML for non-`/api` requests that accept `text/html`, and a router fallback for unknown page routes.

### 4. Dialogs, the palette and the mobile drawer do not manage focus (#61)

- Screens: `upload`, `document-tags`, `palette`, `menu`; keyboard pass.
- What you see: opening Upload or Edit tags leaves focus on the trigger; Tab walks out of the dialog into the page behind it; Escape does not close either dialog; focus is not returned on close. Tab leaves the palette into the sidebar. At 390px the closed drawer's 16 links are the first Tab stops while invisible; "Open menu" has no `aria-expanded` and does not move focus into the drawer.
- Rules broken: keyboard operability (WCAG 2.1.2, 2.4.3, 2.4.11); "dialogs trap and escape".
- Fix: native `<dialog>` with `showModal()` for upload, tags and palette (focus, inertness, Escape and backdrop for free; restore focus on `close`); `inert` on `#sidebar` while the drawer is closed below the breakpoint, `aria-expanded`/`aria-controls` on the menu button, focus the first link on open.

### 5. The command palette is not a combobox and shows MIME types (#62)

- Screens: `palette` (all).
- What you see: axe critical `aria-required-children` and `aria-required-parent`, serious `listitem` and `aria-input-field-name` on every viewport and theme. The input has no combobox role or `aria-activedescendant`, so arrow selection is silent. Each result's `meta` is `application/pdf`, which crowds titles to "Jahresabrechnung Strom 2…" on mobile. The canvas shows From · Type, date and ASN, grouped into Documents, From and Actions.
- Fix: options as direct children of `role="listbox"` with an `aria-label`; `role="combobox"`, `aria-controls`, `aria-expanded` and `aria-activedescendant` on the input; preselect the first result; From · Type in the sub line and date/ASN in `meta`; hide the keyboard legend on touch devices.

### 6. The workflow editor has unlabeled controls, raw event ids and hides actions (#63)

- Screens: `workflow-new`, `workflow-edit` (all).
- What you see: axe critical `select-name` (three match-mode selects) and `label` (`email_to`). The trigger reads "document.processed"; a hint says "Not used for ingest.received". The seeded workflow's actions (set From to Stadtwerke Köln, add tag Utilities) are not shown anywhere; only the webhook fieldset exists. Hints and "Nothing has run yet." are set in mono. Seventeen controls in one column, email fields shown even for folder sources.
- Rules broken: voice ("name things the way people do"); `meta` only for records; labels on every control.
- Fix: labels or `aria-label` for each mode select and value input; trigger labels as sentences; one row per action with its own controls and "Add an action"; hints in `hint` style; show email conditions only when the source is a mailbox; back link above the title; Save, Send test and Delete in one action row.

### 7. Search label contrast and small tag targets (#64)

- Screens: every app screen in light mode (`color-contrast`), every desktop and tablet screen (`target-size`).
- What you see: the top-bar "Search documents" label is `ink-faint` on `paper-sunken`, 3.46:1. Sidebar tag chips are 22px tall with 6px gaps; the tag remove "×" buttons are 10×12px on desktop; on mobile "Review now" in the notice is 18px tall.
- Rules broken: contrast ≥ 4.5:1; `ink-faint` only for placeholders and disabled labels; 44px touch targets on mobile; WCAG 2.5.8.
- Fix: `.search-label { color: var(--ink-muted); }` (5.6:1); `min-height: 24px` or wider gaps on `.side-tags .f-tag`; a 24px box for `.tag-remove`; a padded inline-block link in notices under the phone breakpoint.

### 8. Sign-in, setup and consent drift from the design system (#65)

- Screens: `setup`, `login`, `login-error`, `consent`, `oauth-error` (auth on).
- What you see: the pages render in the system font because `auth/layout.html` never loads `fonts.css`; `auth_theme.css` carries a hand-copied token block instead of `tokens.css`; the theme follows the OS only, ignoring the person's Light/Dark choice; field labels use the uppercase `f-label`; the password hint is mono; "Folio" is small body text with no mark. axe is clean on all of them.
- Fix: link `/static/fonts/fonts.css` and `/static/tokens.css` from the auth layout and keep only layout rules in `auth_theme.css`; apply the stored theme with the `base.html` script; sentence-case 13px field labels; mark plus name centered above the card, as on the Login and Setup canvases.

## Minor

9. **Document facts show the MIME type** (`document`, palette): "3 pages · application/pdf · added …". Say "PDF" or "Photo"; the File row already names the file. Template: `facts` in `routes/pages.rs`.
10. **No archive number, amount or due date anywhere** (`inbox`, `document`): the canvas uses `ASN 000142` in row `meta` and an Archive no. row in Details. Rows show only the date. Feature gap more than a bug; track with the ASN work.
11. **Pager affordance is inverted** (`document`): the disabled "Previous" has a filled background and border, the enabled "Next" is a quiet text button. Style `:disabled` as `ink-faint` text without a fill; give both the same variant.
12. **Status on every row** (`all-documents`): "Filed" under every filed row adds a third line and makes rows 117px tall. Show status only for Processing, Needs a look and Couldn't read; the canvas folds status into the sub line.
13. **Tag colors are random and collide** (all screens with tags): tone is a hash of the tag id, so two of six tags share `ochre` and the same names get different colors on another instance. The system says to assign tones in order. Store the tone on creation (next unused in palette order). The visual snapshots mask `.f-tag` until this is fixed.
14. **Empty inbox offers three uploads** (`inbox-empty`): top-bar Upload, "Upload a document" in the empty state, and "Choose files" in the dropzone. The canvas keeps the dropzone and adds "Other ways in" (phone app, consume folder, connecting Claude). Drop the empty-state button, add those entry points.
15. **Settings throws on load** (`settings`, every capture): `Missing target element "provider" for "settings-form" controller`; the controller stops before it can toggle the provider-dependent fields. Add the target or guard with `hasProviderTarget`.
16. **Settings shows raw provider ids** (`settings`): "tesseract", "none" and hints that start lowercase ("none turns off search by meaning"). Use "Tesseract (on this server)", "Off", and sentence-case hints, as on the Processing canvas.
17. **Native form controls ignore the accent** (`source-new`, `workflow-*`, `settings`): checkboxes are the browser's blue; fieldset borders are the UA groove, near white in dark mode. `input[type=checkbox] { accent-color: var(--archive); } fieldset { border: 1px solid var(--rule); border-radius: var(--radius-md); }`.
18. **Back links are inconsistent**: the document page uses "← Inbox" above the title; workflows and sources use an indented, underlined "Back to …" below it. Use the document page pattern.
19. **Tablet document page stacks too early** (`document--tablet`): at 768px the page sheet is about 800px tall before Details. Cap the sheet at `max-height: 70vh` (object-fit: contain) or keep two columns from 720px.
20. **Auth pages request a resource that 401s** (all auth screens, console): the unauthenticated favicon or manifest request answers 401. Serve `/static/icons/*` and the manifest without auth and link the icon from `auth/layout.html`.

## Polish

21. **"Edit tags" looks like body text** (`document`): a quiet button with no icon, indented under the tags. Add the pencil icon or make it a `+ Add tag` chip at the end of the tag list.
22. **Select arrows sit on the border** (`document`, forms): the native arrow touches the right edge. `padding-inline-end: var(--space-8)` with a custom chevron background.
23. **Dark-mode scrim lightens the page** (`palette--*--dark`, `upload--*--dark`): the overlay is a light grey wash. Use `rgba(0, 0, 0, 0.5)` in dark.
24. **Bright thumbnails in dark mode** (`inbox--*--dark`): white page thumbnails glare on `paper`. Accept (paper is white) or dim with `filter: brightness(0.92)` in dark.
25. **Monospace for prose**: "Nothing has run yet.", "At least 12 characters.", "Leave a field empty to match any document." are in `meta`. Use `hint`.
26. **Theme toggle hides its options** (sidebar): "Theme: Light" cycles System → Light → Dark on click with no hint. A three-option segmented control or a menu says what will happen.
27. **"Searching..." with three dots** (palette): use "Searching…" to match "Reading…".
28. **Jargon in source forms** (`source-new`): "Consume folder", "Test connection" for a folder. "Folder Folio watches", "Check folder".
29. **No mobile tab bar** (`inbox--mobile`): the PwaInbox canvas has a bottom bar (Inbox, Documents, Upload, Settings); the app uses a drawer only. Worth deciding before the PWA is promoted.

## Guardrails added with this review

- `e2e/tests/a11y.spec.ts` runs axe on every screen in `screens.json` that applies with auth on, at desktop and mobile, light and dark, and fails on serious or critical violations and on horizontal overflow. The violations above are listed with their issue links and reported as `fixme` tests; remove an entry with its fix.
- `e2e/tests/visual.spec.ts` compares full-page snapshots of the inbox, the document page and sign in, desktop and mobile, light and dark, with dates, toasts and tags masked. Baselines live in `e2e/tests/__screenshots__/` and are Linux-only: generate them with `make e2e-update-snapshots` (Rust container plus the Playwright image pinned to `@playwright/test`), or take them from the `e2e-report` artifact of a CI run. The spec is skipped on macOS and Windows.
- The verify skill's screen tour reuses the same `screens.json`, so a new screen is added once for the tour, the axe spec and, with `"snapshot": true`, the visual spec.
