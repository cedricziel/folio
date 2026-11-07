Folio keeps a household's or a small office's paperwork: letters, invoices, contracts, payslips. People open it to find one document fast or to file a new one, then leave. The interface stays quiet so the documents are the loudest thing on screen.

## Principles

- **The document is the hero.** Sheets and thumbnails sit on `paper-raised` with `shadow-sheet`; nothing else floats at rest. Chrome around them is flat `paper` and `paper-sunken`, divided by `rule` hairlines.
- **One accent.** `archive` marks what you can act on and where you are: the primary button, links, the selected row, focus. Never use it for decoration or headings.
- **Records look like records.** Anything a person compares across rows (archive serial numbers, dates, amounts, page counts) is set in `meta` (mono, tabular). Prose is set in `body`.
- **Calm states.** Processing, review and failure are words first, color second: `pending` for "Reading 3 pages…", "Needs a look"; `danger` for "Couldn't read this file". No spinners longer than a line, no toasts that shout.

## Voice

- Plain, short, second person. "Drop files here or choose them." "Filed under Insurance."
- Sentence case everywhere, uppercase only in `label`.
- Name things the way people do: _correspondent_ is shown as "From", _document type_ as "Type", _ASN_ as "Archive no." in body copy (the `meta` value itself keeps the `ASN 000142` form).
- AI suggestions say what they are and ask: "Suggested: Invoice from Stadtwerke Köln. Accept?" Never present a guess as fact.
- No emoji, no exclamation marks, no "Oops".
- Dates in lists use ISO (`2025-03-14`) in `meta`; dates in sentences use the reader's locale ("14 March 2025").

## Color

- Grounds: `paper` (page), `paper-raised` (sheets, cards, palette), `paper-sunken` (sidebar, inputs, dropzone). Text is `ink`, secondary text `ink-muted`; both pass 4.5:1 on all three grounds in both themes.
- `ink-faint` is for placeholders and disabled labels only.
- `archive` fills take `on-archive` text. Hover and pressed use `archive-strong`. A selected row is `archive-tint` with `ink` text and a 2px `archive` bar on its leading edge inside the row (not a bordered card).
- `manila` appears at most once per view: the inbox count, an index tab. It is the brand's only warm note.
- Tags take one of `tag-sage`, `tag-slate`, `tag-clay`, `tag-ochre`, `tag-plum`, `tag-sky` as fill with `ink` text. A tag's color is stable for its lifetime; assign in that order for new tags.
- `danger` and `pending` always come with a word or icon. Their tints (`danger-tint`, `pending-tint`) are notice backgrounds.
- Focus: 2px solid `focus` outline, 2px offset, on every interactive element. It holds 3:1 on every ground.

## Type

- Geist for interface, Geist Mono for records, both bundled with the app as self-hosted variable fonts. On Apple platforms use SF Pro and SF Mono instead (the stacks fall back to them) and map styles to Dynamic Type: `display` → `.largeTitle`, `title` → `.title2`, `heading` → `.headline`, `body` → `.body`, `body-sm` → `.subheadline`, `label` → `.caption` (uppercase, semibold), `meta` and `ocr` → `.caption` / `.callout` in `.monospaced()` design with monospaced digits.
- One `title` per screen. `display` only in empty states and onboarding.
- Use `font-variant-numeric: tabular-nums` (SwiftUI `.monospacedDigit()`) wherever numbers stack.

## Space, radius, elevation

- 4px base: `space-1` … `space-12`. Desktop gutter `space-6`, phone gutter `space-4`.
- `radius-md` on controls, `radius-lg` on cards and dialogs, `radius-sm` on tags and on document sheets (paper has near-square corners).
- `shadow-sheet` for documents, `shadow-pop` for the palette, menus and dialogs. Nothing else casts a shadow.

## Iconography

Lucide icons at 16px (lists, buttons) and 20px (navigation), 1.5px stroke, colored `ink-muted` at rest and `ink` on hover. On Apple platforms use the matching SF Symbol (`doc.text`, `tray.and.arrow.down`, `tag`, `person.crop.square`, `magnifyingglass`, `doc.viewfinder` for scan). No logo exists yet: set the name "Folio" in `heading` weight 600.

## Motion

Short and functional: 120ms ease-out for hover and selection, 200ms for the palette and dialogs. A newly filed document fades into its row once. Respect reduced motion by dropping all transitions.

## Components

CSS classes live in `components/bundle.css` with the `f-` prefix and read only these tokens, so the web app can ship that file as is. There is no JavaScript bundle: the web UI is server-rendered (Askama + Hotwire) and the Apple app is SwiftUI.

- **Status** (`f-status`): a dot and a word. `-done` for finished, `-pending` for running or needs review, `-failed` for errors, `-idle` (a ring) for queued, cancelled or paused.
- **Notice** (`f-notice`): `pending-tint` by default for warnings, `-danger` for errors, `-info` (`paper-sunken`) for neutral facts such as "Nothing in this vault changed".
- **Form field** (`f-field`): `f-field-label`, an `f-input`, then `f-error` (with `aria-invalid="true"` on the input and `aria-describedby` pointing at it) and `f-help`.
- **Switch** (`f-switch`): an `<input type="checkbox" role="switch">` followed by `f-switch-track`. Always wrap it in a `<label>` with the visible text: the input is transparent and only the label gives it a name and a larger click target.
- **Progress** (`f-progress`): segments sized by `flex-grow` from the counts, `-done`, `-skipped`, `-failed`, and a plain `<span>` for what's still waiting. Leave out segments whose count is zero; each segment keeps a 2px minimum width and would draw a sliver. `f-progress-sm` is the thinner bar for rows and phone cards. The bar is `aria-hidden`; an `f-stats` list (`<dl>` of `f-stat`, each `dt` with an `f-swatch`: `-done`, `-skipped`, `-failed`, or `-waiting` for the outlined empty swatch) carries the numbers.
- **Table** (`f-table` in `f-table-wrap`): real `<table>` markup for records people compare, such as failed import items. `f-num` on a `th` or `td` right-aligns numbers. The wrapper scrolls sideways at narrow widths, so give it `tabindex="0" role="region" aria-label="…"` to let keyboard users scroll it.
- **Filter chips** (`f-chips`, `f-chip` with `aria-pressed`): switch between subsets of one list.
- **Breadcrumb** (`f-crumbs`): `<nav aria-label="Breadcrumb"><ol>` with the current page as `aria-current="page"`.

## Native apps

`tokens.json` is the single source for both platforms. Generate CSS custom properties for the web and an asset catalog of named colors (light and dark appearances) plus a `FolioTheme` Swift file for the SwiftUI app. Token names carry over unchanged in camelCase: `paper-raised` → `Color.paperRaised`.
