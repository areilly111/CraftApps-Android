//! Pen/stylus JNI bridge: the mangled entry for
//! `PhotoCraftActivity.nativeOnPenEvent`, forwarding to the shared
//! [`craftapps_shared::pen`] runtime.
//!
//! The Java side (`dispatchTouchEvent` override) reports pen-relevant state
//! changes that winit's Android backend drops (tool type, barrel-button
//! state). See `craftapps-shared/src/pen.rs` for the full protocol.

/// `PhotoCraftActivity.nativeOnPenEvent(action, toolType, buttonState,
/// actionButton)`. The JVM resolves the mangled name on first call. Runs on
/// Android's UI thread; the shared pen runtime is thread-safe.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_app_photocraft_android_PhotoCraftActivity_nativeOnPenEvent(
    raw_env: *mut jni::sys::JNIEnv,
    _this_raw: jni::sys::jobject,
    action: jni::sys::jint,
    tool_type: jni::sys::jint,
    button_state: jni::sys::jint,
    action_button: jni::sys::jint,
) {
    let _ = raw_env;
    // Never unwind into the JVM (it would abort the process).
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        craftapps_shared::pen::handle_pen_event(action, tool_type, button_state, action_button);
    }));
    if r.is_err() {
        log::error!("pen event handling panicked (caught)");
    }
}
