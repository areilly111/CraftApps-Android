# PdfCraft: permission-free startup, external open and compact Android UI

## Problems

- Startup tried to create `/storage/emulated/0/CraftApps/Media` before showing
  the UI. It failed with permission denied when all-files access was refused.
- The PDF ACTION_VIEW intent filter existed, but the activity did not import
  the document on launch or in onNewIntent. Open with showed home/old content.
- Desktop toolbars and side panels competed for a phone's narrow viewport.
  Preferences rows, theme columns and the command palette could exceed it.

## Implementation

- `shared/src/storage.rs`: explicit-root constructor; the original shared-root
  constructor remains available to other ports. Unit tests cover private-root
  startup/reinitialization and an unusable root.
- `shared/src/jni_bridge.rs`: app files-directory accessor.
- PdfCraft `services.rs`, `lib.rs`, activity and manifest: persistent settings,
  media and picker bridge under getFilesDir(); remove broad/legacy storage
  permission requests; restore saved app preferences through eframe storage.
- PdfCraft `incoming.rs` and activity: separate, atomically published import
  events for granted ACTION_VIEW URIs, background copy, cold/warm delivery and
  native repaint notification. See fixes-0.2.8-external-open.md.
- UI `mobile.rs` and `lib.rs`: Android-only compact workspace below 900 logical
  points, large touch targets, short header, document selector, bottom menus,
  full-width tool/inspector sheets, fit-width single-page and hand-tool defaults.
- `canvas.rs`: smaller compact gutters and no desktop floating tool strip;
  queue search text even when a full-screen inspector replaces the canvas.
- `panels.rs`, `search_ui.rs`, `palette.rs`: touch rows, result-to-document
  navigation, wrapped search options, bounded palette and a visible Close button.
- `widgets.rs`, `dialogs.rs`, `js_ui.rs`, `appearance.rs`: bound dialog sizes;
  stack preferences, date/identity controls and theme cards on compact Android.
- `i18n/pt-br.tsv`: translate new mobile labels and external-open failure text.
  The existing Portuguese catalog remains selectable. Explicit language
  preferences are preserved; this contribution does not force a locale.
- Build checks: register the Android shell as an application in the layering
  table, keep the explicit WASM eframe dependency when splitting platform
  dependencies, and update the changed translation catalog's attribution hash.
  Small Android-shim type/lint cleanups preserve clipboard/error behavior.

## Device verification

Honor Magic V5 MBH-N49, Android 16, arm64, 1060 x 2376 outer display, 25 Oct 2026.
Tests used an independently signed/package-suffixed local build to preserve
the user's original installation. That packaging, signing material and branding
are excluded from the contribution; the upstream package/build configuration
and branding are retained here.

- Before: startup denied without broad permission; after: private startup works.
- Reading layout exercised in portrait and landscape during mobile adaptation.
- Main tools/edit menus, document selector, Pages, Search, More and palette checked.
- Preferences checked in English and Brazilian Portuguese, including vertical
  theme cards, scrolling to the end and the reachable OK button.
- Synthetic PDF open, search with one known result, result navigation, Save As
  and reopening the saved copy passed.
- File-manager Open with reproduced the silent failure on 0.2.6. The final
  0.2.8 build opened a user-provided 6.2 MB / 5-page PDF from a stopped app, then
  opened a different synthetic PDF in the same already-running process.
- The in-app picker still opened its synthetic document afterward. An invalid
  provider URI produced a visible error and kept the previous document open.
- Original user PDF hash remained unchanged. All-files access was not granted;
  the final APK manifest lacks broad and legacy external-storage permissions.
- Four standalone Rust external-queue tests passed. The two private-storage
  regression tests passed, with a failing shared-root control demonstrated.

No personal PDF, device identifier, credentials, signing key or personal-content
screenshot is included. Advanced editing/export tools were not exhaustively
tested; this is targeted startup, file-opening and mobile-layout validation.

## Compatibility and remaining limits

Existing shared-storage settings/documents are not automatically migrated.
They are left in place and can be selected through Android's document picker.
An external URI is opened as a private copy; use Save As to export changes.
ACTION_SEND/share and HTTP document URLs are outside this change. Desktop and
wide-screen docking retain their original layouts. Other ports still use the
shared-root storage constructor.
