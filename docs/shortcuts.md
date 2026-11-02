# Keyboard shortcuts

One key map for every client. Web and the Apple app (iPad with a keyboard, Mac) use the same bindings where the platform has the same action. Press `?` in the web UI, or hold Cmd on iPad, to see the list in place.

On Windows and Linux, read Cmd as Ctrl.

| Action                          | Web                 | iPad / Mac                  |
| ------------------------------- | ------------------- | --------------------------- |
| Search documents                | `Cmd K`, `/`        | `Cmd F` (Find)              |
| Upload a document               | `u`                 | `Cmd O` (File > Upload...)  |
| Scan a document                 | -                   | `Cmd Shift S` (iPad)        |
| Go to inbox                     | `g` then `i`        | `Cmd 1` (View > Inbox)      |
| Go to all documents             | `g` then `a`        | `Cmd 2` (View > All documents) |
| Go to workflows                 | `g` then `w`        | -                           |
| Go to settings                  | `g` then `s`        | -                           |
| Next / previous document        | `j` / `k`           | Down / Up arrow             |
| Open the selected document      | `Enter`             | Return                      |
| Edit details                    | `e`                 | -                           |
| Edit tags                       | `t`                 | -                           |
| Move the document to Trash             | `#`, `Cmd Backspace` | `Cmd Backspace`            |
| Undo Move to Trash              | -                   | `Cmd Z` (Mac)               |
| Redo Move to Trash              | -                   | `Cmd Shift Z` (Mac)         |
| Lock Folio now                  | -                   | `Cmd Shift L` (App lock enabled) |
| Show the shortcut list          | `?`                 | hold `Cmd` (iPad)           |

## Web rules

- Single-key shortcuts pause while focus is in an input, textarea, select or editable text, and while a dialog or the palette is open. `Cmd Backspace` follows the same rule, so it never deletes a document while you edit text.
- `g` starts a two-key chord: press the second key within a second, or the chord is dropped.
- `j` / `k` select a row in the document list; `Enter` opens it. On a list, `e`, `t` and `#` act on the selected row: `e` and `t` open the document, `#` asks to move it to Trash.
- Move to Trash always asks first in an in-page dialog. `Keep` is the default button.
- Keys that exist as a hint are shown as `f-kbd` in the palette footer, the shortcut dialog and tooltips (`title` and `aria-keyshortcuts`).

## Where it lives

- Web: the `hotkeys` Stimulus controller in `crates/folio-api/templates/base.html`; the dialogs sit next to the body. Covered by `e2e/tests/shortcuts.spec.ts`.
- Apple: `.keyboardShortcut` modifiers and the app `commands` in `apple/Folio/Sources`.

Change a binding here first, then in each client.
