package app.pdfcraft.android;

import android.app.NativeActivity;
import android.content.Intent;
import android.view.KeyEvent;

/**
 * The app's activity: a {@link NativeActivity} subclass so the Storage Access
 * Framework pickers ({@code ACTION_OPEN_DOCUMENT}, {@code ACTION_CREATE_DOCUMENT},
 * {@code ACTION_OPEN_DOCUMENT_TREE}) can report back. NativeActivity does not
 * forward {@code onActivityResult} to native code, so this override hands the
 * result to the Rust shell, which answers the pending file dialog or folder
 * choice from it.
 *
 * The native library is loaded by the framework via the
 * {@code android.app.lib_name} meta-data, exactly as with a plain
 * NativeActivity; the mangled JNI symbol
 * {@code Java_app_pdfcraft_android_PdfCraftActivity_nativeOnActivityResult}
 * is resolved on first call, so no explicit registration is needed.
 */
public class PdfCraftActivity extends NativeActivity {
    // ---- Bluetooth keyboard modifier tracking ---------------------------------
    // winit's Android backend doesn't reliably report Ctrl/Shift/Alt meta state
    // from Bluetooth keyboards. We track modifier keys here and synthesize
    // corrected KeyEvents with proper meta state.

    /** Currently pressed modifier keys (bitmask: 1=Ctrl, 2=Shift, 4=Alt). */
    private int modifierState = 0;

    @Override
    public boolean dispatchKeyEvent(KeyEvent event) {
        int keyCode = event.getKeyCode();
        int action = event.getAction();
        boolean down = action == KeyEvent.ACTION_DOWN;

        // Track modifier keys
        int mask = 0;
        if (keyCode == KeyEvent.KEYCODE_CTRL_LEFT || keyCode == KeyEvent.KEYCODE_CTRL_RIGHT) {
            mask = 1;
        } else if (keyCode == KeyEvent.KEYCODE_SHIFT_LEFT || keyCode == KeyEvent.KEYCODE_SHIFT_RIGHT) {
            mask = 2;
        } else if (keyCode == KeyEvent.KEYCODE_ALT_LEFT || keyCode == KeyEvent.KEYCODE_ALT_RIGHT) {
            mask = 4;
        }

        if (mask != 0) {
            if (down) {
                modifierState |= mask;
            } else {
                modifierState &= ~mask;
            }
            return super.dispatchKeyEvent(event);
        } else {
            int eventMeta = event.getMetaState();
            int neededMeta = 0;
            if ((modifierState & 1) != 0) neededMeta |= KeyEvent.META_CTRL_ON | KeyEvent.META_CTRL_LEFT_ON;
            if ((modifierState & 2) != 0) neededMeta |= KeyEvent.META_SHIFT_ON | KeyEvent.META_SHIFT_LEFT_ON;
            if ((modifierState & 4) != 0) neededMeta |= KeyEvent.META_ALT_ON | KeyEvent.META_ALT_LEFT_ON;

            if ((eventMeta & neededMeta) != neededMeta) {
                KeyEvent corrected = new KeyEvent(
                    event.getDownTime(),
                    event.getEventTime(),
                    action,
                    keyCode,
                    event.getRepeatCount(),
                    eventMeta | neededMeta,
                    event.getDeviceId(),
                    event.getScanCode(),
                    event.getFlags() | KeyEvent.FLAG_SOFT_KEYBOARD,
                    event.getSource()
                );
                return super.dispatchKeyEvent(corrected);
            }
            return super.dispatchKeyEvent(event);
        }
    }

    /**
     * Starts a picker for result. Must be called on the Activity itself:
     * Rust calls this on the bridge's activity ref (which *is* this
     * activity), never via {@code find_class} (attached threads can't
     * see app classes).
     */
    public void launchForResult(Intent intent, int requestCode) {
        startActivityForResult(intent, requestCode);
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        nativeOnActivityResult(requestCode, resultCode, data);
    }

    private native void nativeOnActivityResult(int requestCode, int resultCode, Intent data);
}
