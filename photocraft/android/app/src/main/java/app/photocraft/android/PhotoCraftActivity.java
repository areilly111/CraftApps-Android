package app.photocraft.android;

import android.app.NativeActivity;
import android.content.Intent;
import android.view.KeyEvent;
import android.view.MotionEvent;

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
 * {@code Java_app_photocraft_android_PhotoCraftActivity_nativeOnActivityResult}
 * is resolved on first call, so no explicit registration is needed.
 *
 * {@code startActivityForResult} must be invoked on the Activity itself — the
 * JNI bridge only holds the application context, which has no such method —
 * so Rust launches pickers through {@link #launchForResult} on the activity
 * instance the bridge already holds (never via {@code find_class}: from an
 * attached thread Android cannot resolve app classes that way).
 */
public class PhotoCraftActivity extends NativeActivity {
    /** Starts a picker for result. Must be called on the Activity. */
    public void launchForResult(Intent intent, int requestCode) {
        startActivityForResult(intent, requestCode);
    }

    // ---- Pen side-channel -------------------------------------------------
    // winit's Android backend turns every pointer into a generic Touch and
    // drops the tool type (stylus/finger/eraser/palm) and the barrel-button
    // state. dispatchTouchEvent still fires for NativeActivity subclasses, so
    // we forward pen-relevant *state changes* to Rust here. Throttled: only
    // tool transitions and button press/release cross the JNI boundary.

    /** Last tool type / button state reported, to throttle JNI calls. */
    private int lastPenTool = -1;
    private int lastPenButtons = -1;

    @Override
    public boolean dispatchTouchEvent(MotionEvent event) {
        int action = event.getActionMasked();
        int tool = event.getToolType(event.getActionIndex());
        int buttons = event.getButtonState();
        boolean penRelevant =
                tool == MotionEvent.TOOL_TYPE_STYLUS
                        || tool == MotionEvent.TOOL_TYPE_ERASER
                        || (buttons
                                        & (MotionEvent.BUTTON_STYLUS_PRIMARY
                                                | MotionEvent.BUTTON_STYLUS_SECONDARY))
                                != 0
                        || action == MotionEvent.ACTION_BUTTON_PRESS
                        || action == MotionEvent.ACTION_BUTTON_RELEASE;
        if (penRelevant && (tool != lastPenTool || buttons != lastPenButtons)) {
            lastPenTool = tool;
            lastPenButtons = buttons;
            nativeOnPenEvent(action, tool, buttons, event.getActionButton());
        }
        return super.dispatchTouchEvent(event);
    }

    private native void nativeOnPenEvent(int action, int toolType, int buttonState, int actionButton);

    // ---- Bluetooth keyboard modifier tracking ---------------------------------
    // winit's Android backend doesn't reliably report Ctrl/Shift/Alt meta state
    // from Bluetooth keyboards. We track modifier keys here and forward the
    // combined state to Rust, which synthesizes the correct egui modifiers.

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
            // Let the modifier event through normally
            return super.dispatchKeyEvent(event);
        } else {
            // For non-modifier keys: if our tracked state has modifiers that the
            // event's meta state is missing (Bluetooth keyboard bug), synthesize
            // a corrected event with the proper meta state.
            int eventMeta = event.getMetaState();
            int neededMeta = 0;
            if ((modifierState & 1) != 0) neededMeta |= KeyEvent.META_CTRL_ON | KeyEvent.META_CTRL_LEFT_ON;
            if ((modifierState & 2) != 0) neededMeta |= KeyEvent.META_SHIFT_ON | KeyEvent.META_SHIFT_LEFT_ON;
            if ((modifierState & 4) != 0) neededMeta |= KeyEvent.META_ALT_ON | KeyEvent.META_ALT_LEFT_ON;

            if ((eventMeta & neededMeta) != neededMeta) {
                // Synthesize a corrected event
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

    private native void nativeOnModifierState(int modifiers);

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        nativeOnActivityResult(requestCode, resultCode, data);
    }

    private native void nativeOnActivityResult(int requestCode, int resultCode, Intent data);
}
