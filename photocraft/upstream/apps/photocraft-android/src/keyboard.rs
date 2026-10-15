//! Bluetooth keyboard modifier JNI bridge: the mangled entry for
//! `PhotoCraftActivity.nativeOnModifierState`, storing the modifier state
//! that winit's Android backend drops from Bluetooth keyboards.
//!
//! The Java side (`dispatchKeyEvent` override) tracks Ctrl/Shift/Alt key
//! presses and forwards the combined bitmask here. The egui input handler
//! reads this state when processing keyboard shortcuts.

use std::sync::atomic::{AtomicI32, Ordering};

/// Modifier bitmask from Java: bit 0 = Ctrl, bit 1 = Shift, bit 2 = Alt.
static MODIFIER_STATE: AtomicI32 = AtomicI32::new(0);

/// Get the current modifier state (for the egui input handler).
pub fn modifier_state() -> i32 {
    MODIFIER_STATE.load(Ordering::SeqCst)
}

/// `PhotoCraftActivity.nativeOnModifierState(modifiers)`. The JVM resolves the
/// mangled name on first call. Runs on Android's UI thread; the atomic is
/// thread-safe.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_photocraft_android_PhotoCraftActivity_nativeOnModifierState(
    raw_env: *mut jni::sys::JNIEnv,
    _this_raw: jni::sys::jobject,
    modifiers: jni::sys::jint,
) {
    let _ = raw_env;
    // Never unwind into the JVM (it would abort the process).
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        MODIFIER_STATE.store(modifiers, Ordering::SeqCst);
    }));
    if r.is_err() {
        log::error!("modifier state handling panicked (caught)");
    }
}
