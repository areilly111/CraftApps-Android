package app.effectcraft.android;

import android.app.NativeActivity;
import android.content.Intent;

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
 * {@code Java_app_effectcraft_android_EffectCraftActivity_nativeOnActivityResult}
 * is resolved on first call, so no explicit registration is needed.
 */
public class EffectCraftActivity extends NativeActivity {
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
