package app.pdfcraft.android;

import android.app.NativeActivity;
import android.content.ClipData;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.provider.OpenableColumns;
import android.view.KeyEvent;
import android.view.MotionEvent;

import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.ArrayList;

/**
 * The app's activity: a {@link NativeActivity} subclass.
 *
 * <p>Two jobs:
 *
 * <ol>
 *   <li><b>The file picker bridge.</b> The Storage Access Framework pickers
 *       ({@code ACTION_OPEN_DOCUMENT}, {@code ACTION_CREATE_DOCUMENT}) can only
 *       be launched from the UI thread, and the Rust side calls from the
 *       eframe render thread, where a JNI call into the Activity fails
 *       silently. So Rust writes a request file and this Activity polls for it
 *       with a {@link Handler}, launches the picker itself and writes the
 *       result back. See {@code shared/src/picker_bridge.rs}.
 *   <li><b>The pen side-channel</b> (see below).
 * </ol>
 *
 * <p><b>Input caveat (important).</b> {@code NativeActivity} calls
 * {@code getWindow().takeInputQueue(this)} in {@code onCreate}, and AOSP
 * documents that as "the window will no longer read and dispatch input events
 * from the queue; it is your responsibility to do so". The native side (winit)
 * owns the input queue, so the Java view hierarchy never sees a touch, key or
 * generic-motion event: {@link #dispatchTouchEvent}, {@link #dispatchKeyEvent}
 * and {@link #dispatchGenericMotionEvent} below are <i>not</i> called on this
 * Activity. The multi-finger "stylus mode" gestures are therefore detected in
 * Rust from egui's touch events (see {@code gestures.rs}); the pen side-channel
 * is kept because it is the only path to the S Pen's barrel button if a
 * Bluetooth S Pen ever reports one, but on a passive (non-Bluetooth) S Pen
 * nothing ever arrives.
 */
public class PdfCraftActivity extends NativeActivity {
    // NativeActivity's dlopen is not a Java ClassLoader native-library registration.
    // Register the same library for Java's native method lookup as well.
    static { System.loadLibrary("pdfcraft_android"); }
    /** Starts a picker for result. May be called from any thread. */
    public static void launchForResult(android.app.Activity activity, Intent intent, int requestCode) {
        activity.runOnUiThread(() -> activity.startActivityForResult(intent, requestCode));
    }

    // ---- Pen side-channel -------------------------------------------------
    // winit's Android backend turns every pointer into a generic Touch and
    // drops the tool type (stylus/finger/eraser/palm) and the barrel-button
    // state. The Java side-channel would report those, but on a NativeActivity
    // the input queue belongs to the native side, so these hooks do not run
    // (see the class comment). Kept for the day the app stops being a
    // NativeActivity: the protocol is documented in
    // craftapps-shared/src/pen.rs.

    // TOOL_TYPE_PALM was added in API 35; we compile against 34, so define it locally.
    private static final int TOOL_TYPE_PALM = 5;

    @Override
    public boolean dispatchTouchEvent(MotionEvent event) {
        int action = event.getActionMasked();
        int tool = event.getToolType(event.getActionIndex());
        int buttons = event.getButtonState();
        boolean penRelevant =
                tool == MotionEvent.TOOL_TYPE_STYLUS
                        || tool == MotionEvent.TOOL_TYPE_ERASER
                        || tool == TOOL_TYPE_PALM
                        || (buttons
                                        & (MotionEvent.BUTTON_STYLUS_PRIMARY
                                                | MotionEvent.BUTTON_STYLUS_SECONDARY))
                                != 0
                        || action == MotionEvent.ACTION_BUTTON_PRESS
                        || action == MotionEvent.ACTION_BUTTON_RELEASE;
        if (penRelevant) {
            nativeOnPenEvent(action, tool, buttons, event.getActionButton(), event.getX(), event.getY());
        }
        return super.dispatchTouchEvent(event);
    }

    private native void nativeOnPenEvent(int action, int toolType, int buttonState, int actionButton, float x, float y);

    // ---- Stylus button via generic motion / key events --------------------
    // Some devices deliver the S Pen's barrel button as a generic motion event
    // (SOURCE_STYLUS) or as KEYCODE_STYLUS_BUTTON_PRIMARY.

    @Override
    public boolean dispatchGenericMotionEvent(MotionEvent event) {
        int source = event.getSource();
        if ((source & android.view.InputDevice.SOURCE_STYLUS) != 0) {
            int action = event.getActionMasked();
            int tool = event.getToolType(event.getActionIndex());
            int buttons = event.getButtonState();
            if (action == MotionEvent.ACTION_BUTTON_PRESS
                    || action == MotionEvent.ACTION_BUTTON_RELEASE
                    || (buttons
                                    & (MotionEvent.BUTTON_STYLUS_PRIMARY
                                            | MotionEvent.BUTTON_STYLUS_SECONDARY))
                            != 0) {
                nativeOnPenEvent(action, tool, buttons, event.getActionButton(), event.getX(), event.getY());
            }
        }
        return super.dispatchGenericMotionEvent(event);
    }

    // ---- Bluetooth keyboard modifiers ------------------------------------
    // winit's Android backend never sends `ModifiersChanged`, so egui's
    // modifier state is always empty. The Rust shell tracks Ctrl/Shift/Alt from
    // the key events themselves; this hook is only a fallback for a future
    // non-NativeActivity shell.

    private int modifierState = 0;

    @Override
    public boolean dispatchKeyEvent(KeyEvent event) {
        int keyCode = event.getKeyCode();
        boolean down = event.getAction() == KeyEvent.ACTION_DOWN;
        if (keyCode == KeyEvent.KEYCODE_STYLUS_BUTTON_PRIMARY) {
            int action =
                    down ? MotionEvent.ACTION_BUTTON_PRESS : MotionEvent.ACTION_BUTTON_RELEASE;
            nativeOnPenEvent(
                    action,
                    MotionEvent.TOOL_TYPE_STYLUS,
                    down ? MotionEvent.BUTTON_STYLUS_PRIMARY : 0,
                    MotionEvent.BUTTON_STYLUS_PRIMARY,
                    0, 0);
            return super.dispatchKeyEvent(event);
        }
        int bit = 0;
        if (keyCode == KeyEvent.KEYCODE_CTRL_LEFT || keyCode == KeyEvent.KEYCODE_CTRL_RIGHT) {
            bit = 1;
        } else if (keyCode == KeyEvent.KEYCODE_SHIFT_LEFT || keyCode == KeyEvent.KEYCODE_SHIFT_RIGHT) {
            bit = 2;
        } else if (keyCode == KeyEvent.KEYCODE_ALT_LEFT || keyCode == KeyEvent.KEYCODE_ALT_RIGHT) {
            bit = 4;
        }
        if (bit != 0) {
            if (down) {
                modifierState |= bit;
            } else {
                modifierState &= ~bit;
            }
            nativeOnModifierState(modifierState);
        }
        return super.dispatchKeyEvent(event);
    }

    private native void nativeOnModifierState(int modifiers);

    // ---- File picker bridge ----------------------------------------------
    // Rust writes <bridge>/request.json, this Activity polls for it, launches
    // the system picker and writes <bridge>/response.json.

    private static final int REQ_BRIDGE_OPEN = 1001;
    private static final int REQ_BRIDGE_SAVE = 1002;
    private static final int REQ_BRIDGE_FOLDER = 1003;
    private static final long POLL_MS = 400;

    private final Handler pickerHandler = new Handler(Looper.getMainLooper());
    private boolean pickerBridgeStarted = false;
    /** The request the open picker answers (-1 = none). */
    private long bridgeRequestId = -1;
    /** "open" or "save". */
    private String bridgeRequestKind = "";

    private static final java.util.concurrent.ExecutorService DOCUMENT_IMPORTS =
            java.util.concurrent.Executors.newSingleThreadExecutor();
    private native void nativeOnDocumentReady();

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        // NativeActivity.onCreate loads the native library and takes over the
        // window's surface and input queue; the bridge starts after it.
        super.onCreate(savedInstanceState);
        applyImmersiveFullscreen();
        startPickerBridge();
        if (savedInstanceState == null) {
            importIncomingDocument(getIntent());
        }
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        importIncomingDocument(intent);
    }

    /** ACTION_VIEW grants access to one document, not the whole filesystem. */
    private void importIncomingDocument(Intent intent) {
        if (intent == null || !Intent.ACTION_VIEW.equals(intent.getAction())) {
            return;
        }
        final Uri uri = intent.getData();
        DOCUMENT_IMPORTS.execute(() -> {
            String token = String.format(java.util.Locale.ROOT, "%019d-", System.currentTimeMillis())
                    + java.util.UUID.randomUUID();
            File incoming = new File(bridgeDir(), "incoming");
            try {
                if (uri == null || !("content".equals(uri.getScheme()) || "file".equals(uri.getScheme()))) {
                    throw new java.io.IOException("No readable document URI was supplied");
                }
                File directory = new File(incoming, "documents/" + token);
                String path = copyDocument(uri, directory, "");
                publishIncoming(incoming, token + ".open", path);
            } catch (Exception error) {
                android.util.Log.w("PdfCraftActivity", "External document import failed", error);
                try {
                    publishIncoming(incoming, token + ".error", "The document could not be read. "
                            + String.valueOf(error.getMessage()));
                } catch (Exception publishError) {
                    runOnUiThread(() -> android.widget.Toast.makeText(this,
                            "Couldn't open document from another app", android.widget.Toast.LENGTH_LONG).show());
                }
            }
        });
    }

    private void publishIncoming(File incoming, String name, String value) throws Exception {
        File queue = new File(incoming, "queue");
        if (!queue.isDirectory() && !queue.mkdirs()) {
            throw new java.io.IOException("Couldn't create external document queue");
        }
        File temporary = new File(queue, name + ".tmp");
        java.nio.file.Files.write(temporary.toPath(), value.getBytes(java.nio.charset.StandardCharsets.UTF_8));
        java.nio.file.Files.move(temporary.toPath(), new File(queue, name).toPath(),
                java.nio.file.StandardCopyOption.ATOMIC_MOVE);
        nativeOnDocumentReady();
    }


    /** Force true fullscreen: hide the status bar and the system navigation
     *  bar / tablet taskbar ("dock") so they never overlay the app. Sticky
     *  immersive: a swipe from a screen edge reveals the bars briefly. */
    private void applyImmersiveFullscreen() {
        try {
            if (android.os.Build.VERSION.SDK_INT >= 30) {
                getWindow().setDecorFitsSystemWindows(false);
                android.view.WindowInsetsController c = getWindow().getInsetsController();
                if (c != null) {
                    c.hide(android.view.WindowInsets.Type.systemBars());
                    c.setSystemBarsBehavior(
                        android.view.WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
                }
            } else {
                getWindow().getDecorView().setSystemUiVisibility(
                    android.view.View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                    | android.view.View.SYSTEM_UI_FLAG_FULLSCREEN
                    | android.view.View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                    | android.view.View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                    | android.view.View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                    | android.view.View.SYSTEM_UI_FLAG_LAYOUT_STABLE);
            }
        } catch (Exception ignored) {
            // Fullscreen is cosmetic; never let it break start-up.
        }
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) {
            // The system restores the bars on focus changes (dialogs, the
            // picker returning); re-apply so the app stays fullscreen.
            applyImmersiveFullscreen();
        }
    }

    private void startPickerBridge() {
        if (pickerBridgeStarted) {
            return;
        }
        pickerBridgeStarted = true;
        // A request left over from a previous run would open the system picker
        // at start-up; drop it.
        try {
            File stale = new File(bridgeDir(), "request.json");
            if (stale.isFile()) {
                stale.delete();
            }
        } catch (Exception ignored) {
            // Not fatal: the poll below reports the problem.
        }
        pickerHandler.postDelayed(new Runnable() {
            @Override
            public void run() {
                try {
                    checkPickerRequest();
                } catch (Throwable ignored) {
                    // Keep polling: a half-written request file is not fatal.
                }
                pickerHandler.postDelayed(this, POLL_MS);
            }
        }, POLL_MS);
    }

    /** Where the bridge files live: the app's private persistent files dir. */
    private File bridgeDir() {
        return new File(getFilesDir(), ".picker");
    }

    private void checkPickerRequest() {
        try {
            File req = new File(bridgeDir(), "request.json");
            if (!req.isFile()) {
                return;
            }
            String text = new String(java.nio.file.Files.readAllBytes(req.toPath()));
            if (!req.delete()) {
                // Another poll already took it.
                return;
            }
            org.json.JSONObject r = new org.json.JSONObject(text);
            long id = r.optLong("id", -1);
            String kind = r.optString("kind", "open");
            boolean multiple = r.optBoolean("multiple", false);
            String name = r.optString("name", "");
            org.json.JSONArray mimes = r.optJSONArray("mimes");

            Intent intent;
            int code;
            if ("save".equals(kind)) {
                intent = new Intent(Intent.ACTION_CREATE_DOCUMENT)
                        .addCategory(Intent.CATEGORY_OPENABLE)
                        .putExtra(Intent.EXTRA_TITLE, name.isEmpty() ? "file" : name);
                intent.setType(mimeForName(name));
                intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION
                        | Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                        | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
                code = REQ_BRIDGE_SAVE;
            } else if ("folder".equals(kind)) {
                intent = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
                intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION
                        | Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                        | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
                code = REQ_BRIDGE_FOLDER;
            } else {
                intent = new Intent(Intent.ACTION_OPEN_DOCUMENT)
                        .addCategory(Intent.CATEGORY_OPENABLE)
                        .setType("*/*");
                if (mimes != null && mimes.length() > 0) {
                    String[] types = new String[mimes.length()];
                    for (int i = 0; i < mimes.length(); i++) {
                        types[i] = mimes.optString(i, "*/*");
                    }
                    intent.putExtra(Intent.EXTRA_MIME_TYPES, types);
                }
                if (multiple) {
                    intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true);
                }
                intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION
                        | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
                code = REQ_BRIDGE_OPEN;
            }
            // Only now is the request ours: a result arriving before this
            // would be misattributed.
            bridgeRequestId = id;
            bridgeRequestKind = kind;
            try {
                startActivityForResult(intent, code);
            } catch (Exception e) {
                bridgeRequestId = -1;
                bridgeRequestKind = "";
                writePickerResponse(id, null, null, true, null);
            }
        } catch (Exception e) {
            // A malformed or half-written request: ignore it and keep polling.
        }
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == REQ_BRIDGE_OPEN
                || requestCode == REQ_BRIDGE_SAVE
                || requestCode == REQ_BRIDGE_FOLDER) {
            handleBridgeResult(resultCode, data);
            return;
        }
        nativeOnActivityResult(requestCode, resultCode, data);
    }

    private void handleBridgeResult(int resultCode, Intent data) {
        long id = bridgeRequestId;
        String kind = bridgeRequestKind;
        bridgeRequestId = -1;
        bridgeRequestKind = "";
        if (id < 0) {
            return;
        }
        if (resultCode != RESULT_OK || data == null) {
            writePickerResponse(id, null, null, true, null);
            return;
        }
        try {
            if ("save".equals(kind)) {
                // A save: hand back the document URI so the app writes through
                // SAF (the file lands where the user chose).
                Uri uri = data.getData();
                if (uri == null) {
                    writePickerResponse(id, null, null, true, null);
                    return;
                }
                takePersistableWrite(uri);
                writePickerResponse(id, null, uri.toString(), false, displayName(uri));
            } else if ("folder".equals(kind)) {
                // A folder: hand back the tree URI.
                Uri uri = data.getData();
                if (uri == null) {
                    writePickerResponse(id, null, null, true, null);
                    return;
                }
                takePersistableWrite(uri);
                writePickerResponse(id, null, uri.toString(), false, displayName(uri));
            } else {
                // An open: copy the picked files into the app's own storage so
                // the app can read them as ordinary files.
                ArrayList<Uri> uris = new ArrayList<>();
                ClipData clip = data.getClipData();
                if (clip != null) {
                    for (int i = 0; i < clip.getItemCount(); i++) {
                        Uri u = clip.getItemAt(i).getUri();
                        if (u != null) {
                            uris.add(u);
                        }
                    }
                } else if (data.getData() != null) {
                    uris.add(data.getData());
                }
                if (uris.isEmpty()) {
                    writePickerResponse(id, null, null, true, null);
                    return;
                }
                String[] paths = new String[uris.size()];
                for (int i = 0; i < uris.size(); i++) {
                    paths[i] = copyIntoInbox(id, i, uris.get(i));
                }
                writePickerResponse(id, paths, null, false, null);
            }
        } catch (Exception e) {
            writePickerResponse(id, null, null, true, null);
        }
    }

    /** Keep read/write access to a document across restarts when the provider allows it. */
    private void takePersistableWrite(Uri uri) {
        try {
            getContentResolver().takePersistableUriPermission(uri,
                    Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
        } catch (Exception ignored) {
            // The grant lasts for this session anyway.
        }
    }

    /** Copy a picked document into <bridge>/inbox/<id>/ and return its path. */
    private String copyIntoInbox(long id, int index, Uri uri) throws Exception {
        File inbox = new File(bridgeDir(), "inbox/" + id);
        return copyDocument(uri, inbox, index + "-");
    }

    private String copyDocument(Uri uri, File inbox, String prefix) throws Exception {
        if (!inbox.isDirectory() && !inbox.mkdirs()) {
            throw new java.io.IOException("Couldn't create private document directory");
        }
        String name = displayName(uri);
        if (name == null || name.trim().isEmpty()) {
            name = "document.pdf";
        }
        name = name.replace('/', '_').replace('\\', '_');
        if (".".equals(name) || "..".equals(name)) { name = "document.pdf"; }
        File out = new File(inbox, prefix + name);
        try (InputStream in = getContentResolver().openInputStream(uri);
             OutputStream os = new FileOutputStream(out)) {
            if (in == null) {
                throw new IllegalStateException("couldn't open " + name);
            }
            byte[] buf = new byte[16384];
            int n;
            while ((n = in.read(buf)) > 0) {
                os.write(buf, 0, n);
            }
        }
        return out.getAbsolutePath();
    }

    /** The document's display name, or its last path segment. */
    private String displayName(Uri uri) {
        try (Cursor cur = getContentResolver().query(uri, null, null, null, null)) {
            if (cur != null && cur.moveToFirst()) {
                int idx = cur.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                if (idx >= 0) {
                    String n = cur.getString(idx);
                    if (n != null && !n.trim().isEmpty()) {
                        return n;
                    }
                }
            }
        } catch (Exception ignored) {
            // Fall through to the path segment.
        }
        return uri.getLastPathSegment();
    }

    private void writePickerResponse(long id, String[] paths, String uri, boolean cancelled, String name) {
        try {
            org.json.JSONObject resp = new org.json.JSONObject();
            resp.put("id", id);
            resp.put("cancelled", cancelled);
            if (paths != null) {
                org.json.JSONArray arr = new org.json.JSONArray();
                for (String p : paths) {
                    arr.put(p);
                }
                resp.put("paths", arr);
            }
            if (uri != null) {
                resp.put("uri", uri);
            }
            if (name != null) {
                resp.put("name", name);
            }
            File dir = bridgeDir();
            dir.mkdirs();
            File tmp = new File(dir, "response.json.tmp");
            java.nio.file.Files.write(tmp.toPath(), resp.toString().getBytes());
            tmp.renameTo(new File(dir, "response.json"));
        } catch (Exception ignored) {
            // The Rust side will time the request out; nothing else to do.
        }
    }

    /** The MIME type for a file name (the save picker's type). */
    private static String mimeForName(String name) {
        String lower = name == null ? "" : name.toLowerCase();
        if (lower.endsWith(".psd") || lower.endsWith(".psb")) {
            return "image/vnd.adobe.photoshop";
        }
        if (lower.endsWith(".png")) {
            return "image/png";
        }
        if (lower.endsWith(".jpg") || lower.endsWith(".jpeg")) {
            return "image/jpeg";
        }
        if (lower.endsWith(".webp")) {
            return "image/webp";
        }
        if (lower.endsWith(".gif")) {
            return "image/gif";
        }
        if (lower.endsWith(".bmp")) {
            return "image/bmp";
        }
        if (lower.endsWith(".tif") || lower.endsWith(".tiff")) {
            return "image/tiff";
        }
        if (lower.endsWith(".svg")) {
            return "image/svg+xml";
        }
        if (lower.endsWith(".pdf")) {
            return "application/pdf";
        }
        return "application/octet-stream";
    }

    private native void nativeOnActivityResult(int requestCode, int resultCode, Intent data);
}
