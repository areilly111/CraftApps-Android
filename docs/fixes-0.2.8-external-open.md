# PdfCraft 0.2.8: open PDFs sent by other Android apps

## Trigger and cause

Choosing PdfCraft in a file manager's Open with menu launched the activity but
left the home screen or previous document visible. The manifest advertised
ACTION_VIEW while the activity implemented neither initial-intent import nor
onNewIntent handling. The ordinary SAF picker followed a different working path.

## Changes

- `PdfCraftActivity.java`: handle ACTION_VIEW on cold launch and onNewIntent.
  Copy the granted content/file URI to a unique directory under private storage
  on a serial worker, retaining the display name. Publish a complete `.open`
  or `.error` event with an atomic rename. Do not reuse picker request IDs,
  request.json or response.json. Register the native library with the Java
  class loader (`System.loadLibrary`); NativeActivity's dlopen alone does not
  let Java resolve the new native wake method on the tested Honor.
- `incoming.rs`: consume one queued event at a time, leave unfinished `.tmp`
  events alone, and restrict document paths to the private import directory.
  Surface import errors instead of silently showing the old document.
- `lib.rs` / `shell.rs`: install a render-context waker, open completed private
  copies through the normal PdfCraft open worker, and drain additional events.
  No repeating render timer is needed while the app is idle.
- Portuguese catalog: add the external-document failure message.

The original document is never written by import. Save As remains the way to
export a changed copy. No broad storage permission is added. ACTION_SEND/share
and remote HTTP URLs are outside this change.

## Verification

The standalone Rust queue tests exercise single consumption, separation from
picker responses, pending events, ordered multiple events, rejection of paths
outside the document inbox and missing files. Run with `rustc --edition=2024
--test pdfcraft/upstream/apps/pdfcraft-android/src/incoming.rs`.

Device validation targets Honor Magic V5 / Android 16: reproduce the silent
failure on 0.2.6, then test file-manager Open with with the app stopped and with
it already running. Use both a synthetic PDF and a user-provided 6.2 MB PDF.
The latter and all device screenshots remain private and are not distributed.
Recheck the normal picker, original-file integrity and absent all-files access.
Final observed results accompany the APK validation report.
