package app.craftlauncher.android;

import android.app.NativeActivity;
import android.content.Intent;

/**
 * The launcher's activity: a {@link NativeActivity} subclass. The Rust side
 * drives Install / Open entirely through the JNI bridge's activity ref
 * (which *is* this activity) — {@code startActivity} for opening suite apps
 * and the package installer — so no {@code onActivityResult} plumbing is
 * needed here. {@link #launchForResult} is kept for parity with the suite
 * ports' activities.
 *
 * The native library is loaded by the framework via the
 * {@code android.app.lib_name} meta-data, exactly as with a plain
 * NativeActivity.
 */
public class CraftLauncherActivity extends NativeActivity {
    /**
     * Starts an activity for result. Must be called on the Activity itself:
     * Rust calls this on the bridge's activity ref (which *is* this
     * activity), never via {@code find_class} (attached threads can't
     * see app classes).
     */
    public void launchForResult(Intent intent, int requestCode) {
        startActivityForResult(intent, requestCode);
    }
}
