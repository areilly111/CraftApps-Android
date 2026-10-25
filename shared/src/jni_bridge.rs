//! JNI bridge to the Android APIs the shell needs: browser intents, the
//! MediaStore (gallery export), HTTP(S) downloads (updater), SHA-256 and the
//! APK install intent.
//!
//! Every call attaches its thread on demand through [`JavaVM`], so callers can
//! be the UI thread, a worker thread or `android_main`'s thread alike. Java
//! exceptions surface as `Err(String)`; nothing here panics.
//!
//! `unsafe` appears only where raw pointers cross the FFI boundary into
//! `jni`'s safe wrappers; each site has a SAFETY comment (the
//! `photocraft-tablet` pattern required by this workspace).

use jni::objects::{Global, JByteArray, JObject, JString, JValue};
use jni::{Env, JavaVM, jni_sig, jni_str};
use std::sync::OnceLock;
use std::sync::mpsc::Sender;

/// Build a generic product User-Agent (no personal details): `<AppId>-Android/<version>`.
pub fn user_agent(app_id: &str, version: &str) -> String {
    format!("{app_id}-Android/{version}")
}

/// (JavaVM, global ref to the NativeActivity), installed once by [`init`].
static BRIDGE: OnceLock<(JavaVM, Global<JObject<'static>>)> = OnceLock::new();

/// Errors inside a JNI call: either a `jni` error or a plain message (the
/// `jni` error type has no string-message variant).
pub(crate) enum BridgeError {
    Jni(jni::errors::Error),
    Msg(String),
}

impl From<jni::errors::Error> for BridgeError {
    fn from(e: jni::errors::Error) -> Self {
        BridgeError::Jni(e)
    }
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeError::Jni(e) => write!(f, "{e}"),
            BridgeError::Msg(m) => write!(f, "{m}"),
        }
    }
}

pub(crate) type JniResult<T> = Result<T, BridgeError>;

pub(crate) fn with_env<T>(f: impl FnOnce(&mut Env, &JObject) -> JniResult<T>) -> Result<T, String> {
    let (vm, activity) = BRIDGE
        .get()
        .ok_or_else(|| "JNI bridge not initialised".to_string())?;
    vm.attach_current_thread(|env| f(env, activity.as_obj()))
        .map_err(|e: BridgeError| format!("JNI call failed: {e}"))
}

/// Install the bridge. Must run once, from `android_main`'s thread, which
/// android-activity has already attached to the JVM and associated with
/// `ndk-context`.
#[cfg(target_os = "android")]
pub fn init() -> Result<(), String> {
    let ctx = ndk_context::android_context();
    // SAFETY: android-activity guarantees `ndk-context` holds a valid JavaVM
    // pointer before `android_main` runs. `from_raw` only wraps the pointer;
    // nothing is dereferenced here.
    let vm = unsafe { JavaVM::from_raw(ctx.vm() as *mut jni::sys::JavaVM) };
    let activity = vm
        .attach_current_thread(|env| {
            // SAFETY: the activity jobject is valid while `android_main`'s
            // caller lives; it is immediately promoted to a global ref, which
            // keeps it valid for the process lifetime.
            let act = unsafe { JObject::from_raw(&*env, ctx.context() as *mut jni::sys::_jobject) };
            env.new_global_ref(act).map_err(BridgeError::from)
        })
        .map_err(|e: BridgeError| format!("JNI init: could not take a global activity ref: {e}"))?;
    BRIDGE
        .set((vm, activity))
        .map_err(|_| "JNI bridge initialised twice".to_string())?;
    Ok(())
}

/// The bridge is Android-only: the host build exists so the shared crate's
/// pure logic (pen settings, gestures, modifiers) can be unit-tested.
#[cfg(not(target_os = "android"))]
pub fn init() -> Result<(), String> {
    Err("the JNI bridge is Android-only".to_string())
}

pub(crate) fn jstring<'a>(env: &mut Env<'a>, s: &str) -> JniResult<JString<'a>> {
    Ok(env.new_string(s)?)
}

/// `ContentResolver` for the activity. The activity's lifetime is independent
/// of the env's: it is only a method receiver, while the returned local ref
/// is created by (and tied to) `env`.
pub(crate) fn content_resolver<'a>(
    env: &mut Env<'a>,
    activity: &JObject<'_>,
) -> JniResult<JObject<'a>> {
    Ok(env
        .call_method(
            activity,
            jni_str!("getContentResolver"),
            jni_sig!("()Landroid/content/ContentResolver;"),
            &[],
        )?
        .l()?)
}

/// `android.os.Build.VERSION.SDK_INT`.
pub fn android_sdk_int() -> Result<i32, String> {
    with_env(|env, _| {
        let version = env.find_class(jni_str!("android/os/Build$VERSION"))?;
        Ok(env
            .get_static_field(&version, jni_str!("SDK_INT"), jni_sig!("I"))?
            .i()?)
    })
}

/// Whether the app holds all-files access (`MANAGE_EXTERNAL_STORAGE`).
/// API 29 and below are covered by `requestLegacyExternalStorage`, so only
/// API 30+ is actually gated.
pub fn is_external_storage_manager() -> Result<bool, String> {
    if android_sdk_int()? < 30 {
        return Ok(true);
    }
    with_env(|env, _| {
        let env_class = env.find_class(jni_str!("android/os/Environment"))?;
        Ok(env
            .call_static_method(
                &env_class,
                jni_str!("isExternalStorageManager"),
                jni_sig!("()Z"),
                &[],
            )?
            .z()?)
    })
}

/// Shared-storage root, e.g. `/storage/emulated/0`
/// (`Environment.getExternalStorageDirectory()`).
pub fn external_storage_root() -> Result<String, String> {
    with_env(|env, _| {
        let env_class = env.find_class(jni_str!("android/os/Environment"))?;
        let dir = env
            .call_static_method(
                &env_class,
                jni_str!("getExternalStorageDirectory"),
                jni_sig!("()Ljava/io/File;"),
                &[],
            )?
            .l()?;
        let path = env
            .call_method(
                &dir,
                jni_str!("getAbsolutePath"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        let s: JString = env.cast_local::<JString>(path)?;
        Ok(s.mutf8_chars(env)?.to_str().into_owned())
    })
}

/// Open the system "All files access" page for this app
/// (`Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION`).
pub fn open_all_files_settings() -> Result<(), String> {
    with_env(|env, activity| {
        let intent_class = env.find_class(jni_str!("android/content/Intent"))?;
        let settings_class = env.find_class(jni_str!("android/provider/Settings"))?;
        let action = env
            .get_static_field(
                &settings_class,
                jni_str!("ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION"),
                jni_sig!("Ljava/lang/String;"),
            )?
            .l()?;
        let intent = env.new_object(
            &intent_class,
            jni_sig!("(Ljava/lang/String;)V"),
            &[JValue::from(&action)],
        )?;
        // package:<pkg> deep-links straight to this app's page.
        let pkg = env
            .call_method(
                activity,
                jni_str!("getPackageName"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        let pkg: JString = env.cast_local::<JString>(pkg)?;
        let pkg = pkg.mutf8_chars(env)?.to_str().into_owned();
        let uri_j = jstring(env, &format!("package:{pkg}"))?;
        let uri = env
            .call_static_method(
                jni_str!("android/net/Uri"),
                jni_str!("parse"),
                jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
                &[JValue::from(&uri_j)],
            )?
            .l()?;
        env.call_method(
            &intent,
            jni_str!("setData"),
            jni_sig!("(Landroid/net/Uri;)Landroid/content/Intent;"),
            &[JValue::from(&uri)],
        )?;
        // The JNI bridge holds the application context, not the Activity:
        // starting an activity from it requires FLAG_ACTIVITY_NEW_TASK.
        let flag = env
            .get_static_field(
                &intent_class,
                jni_str!("FLAG_ACTIVITY_NEW_TASK"),
                jni_sig!("I"),
            )?
            .i()?;
        env.call_method(
            &intent,
            jni_str!("addFlags"),
            jni_sig!("(I)Landroid/content/Intent;"),
            &[JValue::from(flag)],
        )?;
        env.call_method(
            activity,
            jni_str!("startActivity"),
            jni_sig!("(Landroid/content/Intent;)V"),
            &[JValue::from(&intent)],
        )?;
        Ok(())
    })
}

/// Kill this process (`android.os.Process.killProcess`). Used once: after the
/// storage-permission gate is granted, the app relaunches into shared-storage
/// mode (re-pointing every service mid-run is not sound). The system restarts
/// the activity automatically.
pub fn restart_process() -> Result<(), String> {
    with_env(|env, _| {
        let process = env.find_class(jni_str!("android/os/Process"))?;
        let pid = env
            .call_static_method(&process, jni_str!("myPid"), jni_sig!("()I"), &[])?
            .i()?;
        // Never returns; the Err case below is only for a failed lookup.
        env.call_static_method(
            &process,
            jni_str!("killProcess"),
            jni_sig!("(I)V"),
            &[JValue::from(pid)],
        )?;
        Ok(())
    })
}

/// Open `url` in the system browser / matching app (`Intent.ACTION_VIEW`).
pub fn open_url(url: &str) -> Result<(), String> {
    with_env(|env, activity| {
        let intent_class = env.find_class(jni_str!("android/content/Intent"))?;
        let action_view = env
            .get_static_field(
                &intent_class,
                jni_str!("ACTION_VIEW"),
                jni_sig!("Ljava/lang/String;"),
            )?
            .l()?;
        let url_j = jstring(env, url)?;
        let uri = env
            .call_static_method(
                jni_str!("android/net/Uri"),
                jni_str!("parse"),
                jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
                &[JValue::from(&url_j)],
            )?
            .l()?;
        let intent = env.new_object(
            &intent_class,
            jni_sig!("(Ljava/lang/String;Landroid/net/Uri;)V"),
            &[JValue::from(&action_view), JValue::from(&uri)],
        )?;
        let flag = env
            .get_static_field(
                &intent_class,
                jni_str!("FLAG_ACTIVITY_NEW_TASK"),
                jni_sig!("I"),
            )?
            .i()?;
        env.call_method(
            &intent,
            jni_str!("addFlags"),
            jni_sig!("(I)Landroid/content/Intent;"),
            &[JValue::from(flag)],
        )?;
        env.call_method(
            activity,
            jni_str!("startActivity"),
            jni_sig!("(Landroid/content/Intent;)V"),
            &[JValue::from(&intent)],
        )?;
        Ok(())
    })
}

/// The activity's `ClipboardManager` system service.
fn clipboard_manager<'a>(env: &mut Env<'a>, activity: &JObject<'_>) -> JniResult<JObject<'a>> {
    let name = jstring(env, "clipboard")?;
    Ok(env
        .call_method(
            activity,
            jni_str!("getSystemService"),
            jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
            &[JValue::from(&name)],
        )?
        .l()?)
}

/// Put plain text on the Android system clipboard (`ClipboardManager`).
/// Used by ports whose upstream uses `arboard` for text; image clipboard
/// data stays in the app's own clipboard (arboard has no Android backend).
pub fn clipboard_set_text(text: &str) -> Result<(), String> {
    with_env(|env, activity| {
        let cm = clipboard_manager(env, activity)?;
        let label = jstring(env, "PdfCraft")?;
        let text_j = jstring(env, text)?;
        let clip = env
            .call_static_method(
                jni_str!("android/content/ClipData"),
                jni_str!("newPlainText"),
                jni_sig!(
                    "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;"
                ),
                &[JValue::from(&label), JValue::from(&text_j)],
            )?
            .l()?;
        env.call_method(
            &cm,
            jni_str!("setPrimaryClip"),
            jni_sig!("(Landroid/content/ClipData;)V"),
            &[JValue::from(&clip)],
        )?;
        Ok(())
    })
}

/// Read plain text from the Android system clipboard. Returns `Ok(None)`
/// when the clipboard is empty or holds no text.
pub fn clipboard_get_text() -> Result<Option<String>, String> {
    with_env(|env, activity| {
        let cm = clipboard_manager(env, activity)?;
        let has = env
            .call_method(&cm, jni_str!("hasPrimaryClip"), jni_sig!("()Z"), &[])?
            .z()?;
        if !has {
            return Ok(None);
        }
        let clip = env
            .call_method(
                &cm,
                jni_str!("getPrimaryClip"),
                jni_sig!("()Landroid/content/ClipData;"),
                &[],
            )?
            .l()?;
        if clip.is_null() {
            return Ok(None);
        }
        let count = env
            .call_method(&clip, jni_str!("getItemCount"), jni_sig!("()I"), &[])?
            .i()?;
        if count <= 0 {
            return Ok(None);
        }
        let item = env
            .call_method(
                &clip,
                jni_str!("getItemAt"),
                jni_sig!("(I)Landroid/content/ClipData$Item;"),
                &[JValue::from(0)],
            )?
            .l()?;
        let text = env
            .call_method(
                &item,
                jni_str!("getText"),
                jni_sig!("()Ljava/lang/CharSequence;"),
                &[],
            )?
            .l()?;
        if text.is_null() {
            return Ok(None);
        }
        let s = env
            .call_method(
                &text,
                jni_str!("toString"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        let s: JString = env.cast_local::<JString>(s)?;
        let out = s
            .mutf8_chars(env)
            .map_err(BridgeError::from)?
            .to_str()
            .into_owned();
        if out.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(out))
    })
}

/// `(version_code, version_name)` for an installed package, or `None` when
/// the package isn't installed. The Craft Launcher uses this to show each
/// suite app's installed version.
pub fn installed_package_info(package: &str) -> Result<Option<(i64, String)>, String> {
    with_env(|env, activity| {
        let pm = env
            .call_method(
                activity,
                jni_str!("getPackageManager"),
                jni_sig!("()Landroid/content/pm/PackageManager;"),
                &[],
            )?
            .l()?;
        let pkg_j = jstring(env, package)?;
        let pi = match env.call_method(
            &pm,
            jni_str!("getPackageInfo"),
            jni_sig!("(Ljava/lang/String;I)Landroid/content/pm/PackageInfo;"),
            &[JValue::from(&pkg_j), JValue::from(0i32)],
        ) {
            Ok(v) => v.l()?,
            Err(e) => {
                // NameNotFoundException means "not installed"; anything else
                // is a real failure.
                if env.exception_check() {
                    env.exception_clear();
                    return Ok(None);
                }
                return Err(e.into());
            }
        };
        // minSdk 29, so getLongVersionCode() (API 28+) is always available.
        let code = env
            .call_method(&pi, jni_str!("getLongVersionCode"), jni_sig!("()J"), &[])?
            .j()?;
        let name_obj = env
            .get_field(&pi, jni_str!("versionName"), jni_sig!("Ljava/lang/String;"))?
            .l()?;
        let name_j: JString = env.cast_local::<JString>(name_obj)?;
        Ok(Some((code, name_j.mutf8_chars(env)?.to_str().into_owned())))
    })
}

/// Launch another suite app by package name (the Craft Launcher's "Open"
/// button), via its launch intent.
pub fn launch_package(package: &str) -> Result<(), String> {
    with_env(|env, activity| {
        let pm = env
            .call_method(
                activity,
                jni_str!("getPackageManager"),
                jni_sig!("()Landroid/content/pm/PackageManager;"),
                &[],
            )?
            .l()?;
        let pkg_j = jstring(env, package)?;
        let intent = env
            .call_method(
                &pm,
                jni_str!("getLaunchIntentForPackage"),
                jni_sig!("(Ljava/lang/String;)Landroid/content/Intent;"),
                &[JValue::from(&pkg_j)],
            )?
            .l()?;
        if intent.is_null() {
            return Err(BridgeError::Msg(format!(
                "{package} has no launchable activity"
            )));
        }
        let intent_class = env.find_class(jni_str!("android/content/Intent"))?;
        let flag = env
            .get_static_field(
                &intent_class,
                jni_str!("FLAG_ACTIVITY_NEW_TASK"),
                jni_sig!("I"),
            )?
            .i()?;
        env.call_method(
            &intent,
            jni_str!("addFlags"),
            jni_sig!("(I)Landroid/content/Intent;"),
            &[JValue::from(flag)],
        )?;
        env.call_method(
            activity,
            jni_str!("startActivity"),
            jni_sig!("(Landroid/content/Intent;)V"),
            &[JValue::from(&intent)],
        )?;
        Ok(())
    })
}

/// Persistent app-private directory, independent of broad storage permission.
pub fn files_dir() -> Result<String, String> {
    with_env(|env, activity| {
        let dir = env
            .call_method(
                activity,
                jni_str!("getFilesDir"),
                jni_sig!("()Ljava/io/File;"),
                &[],
            )?
            .l()?;
        let path = env
            .call_method(
                &dir,
                jni_str!("getAbsolutePath"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        let s: JString = env.cast_local::<JString>(path)?;
        Ok(s.mutf8_chars(env)?.to_str().into_owned())
    })
}

/// The app's cache directory (downloads land here).
pub fn cache_dir() -> Result<String, String> {
    with_env(|env, activity| {
        let dir = env
            .call_method(
                activity,
                jni_str!("getCacheDir"),
                jni_sig!("()Ljava/io/File;"),
                &[],
            )?
            .l()?;
        let path = env
            .call_method(
                &dir,
                jni_str!("getAbsolutePath"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        let s: JString = env.cast_local::<JString>(path)?;
        Ok(s.mutf8_chars(env)?.to_str().into_owned())
    })
}

/// GET `url` over HTTPS with Android's stack (TLS, proxy, redirects handled by
/// the platform). Sends the caller-supplied User-Agent plus the GitHub API accept header the
/// updater needs. When `dest` is `Some`, streams the body to that file and
/// reports `(received_bytes, total_bytes_or_-1)` on `progress`; otherwise the
/// body is returned in memory.
pub fn http_get(
    url: &str,
    user_agent: &str,
    dest: Option<&std::path::Path>,
    progress: Option<&Sender<(u64, i64)>>,
) -> Result<Vec<u8>, String> {
    with_env(|env, _activity| {
        let url_j = jstring(env, url)?;
        let url_obj = env.new_object(
            jni_str!("java/net/URL"),
            jni_sig!("(Ljava/lang/String;)V"),
            &[JValue::from(&url_j)],
        )?;
        let conn = env
            .call_method(
                &url_obj,
                jni_str!("openConnection"),
                jni_sig!("()Ljava/net/URLConnection;"),
                &[],
            )?
            .l()?;
        let method_j = jstring(env, "GET")?;
        env.call_method(
            &conn,
            jni_str!("setRequestMethod"),
            jni_sig!("(Ljava/lang/String;)V"),
            &[JValue::from(&method_j)],
        )?;
        for (key, value) in [
            ("User-Agent", user_agent),
            ("Accept", "application/vnd.github+json"),
        ] {
            let k_j = jstring(env, key)?;
            let v_j = jstring(env, value)?;
            env.call_method(
                &conn,
                jni_str!("setRequestProperty"),
                jni_sig!("(Ljava/lang/String;Ljava/lang/String;)V"),
                &[JValue::from(&k_j), JValue::from(&v_j)],
            )?;
        }
        env.call_method(
            &conn,
            jni_str!("setConnectTimeout"),
            jni_sig!("(I)V"),
            &[JValue::from(15_000i32)],
        )?;
        env.call_method(
            &conn,
            jni_str!("setReadTimeout"),
            jni_sig!("(I)V"),
            &[JValue::from(60_000i32)],
        )?;
        env.call_method(&conn, jni_str!("connect"), jni_sig!("()V"), &[])?;
        let code = env
            .call_method(&conn, jni_str!("getResponseCode"), jni_sig!("()I"), &[])?
            .i()?;
        let stream = if (200..300).contains(&code) {
            env.call_method(
                &conn,
                jni_str!("getInputStream"),
                jni_sig!("()Ljava/io/InputStream;"),
                &[],
            )?
            .l()?
        } else {
            let err = env
                .call_method(
                    &conn,
                    jni_str!("getErrorStream"),
                    jni_sig!("()Ljava/io/InputStream;"),
                    &[],
                )?
                .l()?;
            let body = read_all(env, &err)?;
            env.call_method(&conn, jni_str!("disconnect"), jni_sig!("()V"), &[])?;
            let text = String::from_utf8_lossy(&body);
            return Err(BridgeError::Msg(format!("HTTP {code}: {text}")));
        };
        let total = env
            .call_method(
                &conn,
                jni_str!("getContentLengthLong"),
                jni_sig!("()J"),
                &[],
            )?
            .j()?;
        let body = if let Some(dest) = dest {
            let dest_s = dest.to_string_lossy();
            let dest_j = jstring(env, &dest_s)?;
            let fos = env.new_object(
                jni_str!("java/io/FileOutputStream"),
                jni_sig!("(Ljava/lang/String;)V"),
                &[JValue::from(&dest_j)],
            )?;
            stream_to(env, &stream, &fos, total, progress)?;
            env.call_method(&fos, jni_str!("close"), jni_sig!("()V"), &[])?;
            Vec::new()
        } else {
            read_all(env, &stream)?
        };
        env.call_method(&stream, jni_str!("close"), jni_sig!("()V"), &[])?;
        env.call_method(&conn, jni_str!("disconnect"), jni_sig!("()V"), &[])?;
        Ok(body)
    })
}

/// Copy `stream` into `out` (an `OutputStream`), reporting progress.
fn stream_to(
    env: &mut Env,
    stream: &JObject<'_>,
    out: &JObject<'_>,
    total: i64,
    progress: Option<&Sender<(u64, i64)>>,
) -> JniResult<()> {
    let buf = env.new_byte_array(65536)?;
    let mut received: u64 = 0;
    let mut last_report: u64 = 0;
    loop {
        let n = env
            .call_method(
                stream,
                jni_str!("read"),
                jni_sig!("([B)I"),
                &[JValue::from(&buf)],
            )?
            .i()?;
        if n <= 0 {
            break;
        }
        env.call_method(
            out,
            jni_str!("write"),
            jni_sig!("([BII)V"),
            &[JValue::from(&buf), JValue::from(0i32), JValue::from(n)],
        )?;
        received += n as u64;
        if let Some(tx) = progress {
            // Report every 256 KiB; a 90 MB APK would otherwise post ~1400 messages.
            if received - last_report >= 262_144 || (total > 0 && received >= total as u64) {
                last_report = received;
                let _ = tx.send((received, total));
            }
        }
    }
    Ok(())
}

/// Slurp an `InputStream` into memory via `ByteArrayOutputStream`.
fn read_all(env: &mut Env, stream: &JObject<'_>) -> JniResult<Vec<u8>> {
    let out = env.new_object(
        jni_str!("java/io/ByteArrayOutputStream"),
        jni_sig!("()V"),
        &[],
    )?;
    stream_to(env, stream, &out, -1, None)?;
    let arr = env
        .call_method(&out, jni_str!("toByteArray"), jni_sig!("()[B"), &[])?
        .l()?;
    let arr: JByteArray = env.cast_local::<JByteArray>(arr)?;
    let len = arr.len(env)?;
    let mut raw = vec![0i8; len];
    arr.get_region(env, 0, &mut raw)?;
    Ok(raw.into_iter().map(|b| b as u8).collect())
}

/// SHA-256 of `bytes`, lowercase hex, via `java.security.MessageDigest`
/// (no extra Rust dependency).
pub fn sha256_hex(bytes: &[u8]) -> Result<String, String> {
    with_env(|env, _| {
        let alg_j = jstring(env, "SHA-256")?;
        let md = env
            .call_static_method(
                jni_str!("java/security/MessageDigest"),
                jni_str!("getInstance"),
                jni_sig!("(Ljava/lang/String;)Ljava/security/MessageDigest;"),
                &[JValue::from(&alg_j)],
            )?
            .l()?;
        let arr = env.new_byte_array(bytes.len())?;
        let signed: Vec<i8> = bytes.iter().map(|b| *b as i8).collect();
        arr.set_region(env, 0, &signed)?;
        let digest = env
            .call_method(
                &md,
                jni_str!("digest"),
                jni_sig!("([B)[B"),
                &[JValue::from(&arr)],
            )?
            .l()?;
        let digest: JByteArray = env.cast_local::<JByteArray>(digest)?;
        let len = digest.len(env)?;
        let mut raw = vec![0i8; len];
        digest.get_region(env, 0, &mut raw)?;
        Ok(raw.iter().map(|b| format!("{:02x}", *b as u8)).collect())
    })
}

/// Insert a PNG/JPEG/WebP into the shared photo gallery (API 29+ scoped
/// storage: `MediaStore` with `RELATIVE_PATH`). Returns the content URI.
pub fn gallery_insert(
    app_name: &str,
    display_name: &str,
    mime: &str,
    bytes: &[u8],
) -> Result<String, String> {
    with_env(|env, activity| {
        let values = env.new_object(
            jni_str!("android/content/ContentValues"),
            jni_sig!("()V"),
            &[],
        )?;
        let relative_path = format!("Pictures/{app_name}");
        for (k, v) in [
            ("display_name", display_name),
            ("mime_type", mime),
            ("relative_path", relative_path.as_str()),
        ] {
            let k_j = jstring(env, k)?;
            let v_j = jstring(env, v)?;
            env.call_method(
                &values,
                jni_str!("put"),
                jni_sig!("(Ljava/lang/String;Ljava/lang/String;)V"),
                &[JValue::from(&k_j), JValue::from(&v_j)],
            )?;
        }
        let resolver = env
            .call_method(
                activity,
                jni_str!("getContentResolver"),
                jni_sig!("()Landroid/content/ContentResolver;"),
                &[],
            )?
            .l()?;
        let media_class = env.find_class(jni_str!("android/provider/MediaStore$Images$Media"))?;
        let collection = env
            .get_static_field(
                &media_class,
                jni_str!("EXTERNAL_CONTENT_URI"),
                jni_sig!("Landroid/net/Uri;"),
            )?
            .l()?;
        let uri = env
            .call_method(
                &resolver,
                jni_str!("insert"),
                jni_sig!("(Landroid/net/Uri;Landroid/content/ContentValues;)Landroid/net/Uri;"),
                &[JValue::from(&collection), JValue::from(&values)],
            )?
            .l()?;
        let out = env
            .call_method(
                &resolver,
                jni_str!("openOutputStream"),
                jni_sig!("(Landroid/net/Uri;)Ljava/io/OutputStream;"),
                &[JValue::from(&uri)],
            )?
            .l()?;
        let arr = env.new_byte_array(bytes.len())?;
        let signed: Vec<i8> = bytes.iter().map(|b| *b as i8).collect();
        arr.set_region(env, 0, &signed)?;
        env.call_method(
            &out,
            jni_str!("write"),
            jni_sig!("([B)V"),
            &[JValue::from(&arr)],
        )?;
        env.call_method(&out, jni_str!("close"), jni_sig!("()V"), &[])?;
        let s = env
            .call_method(
                &uri,
                jni_str!("toString"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        let s: JString = env.cast_local::<JString>(s)?;
        Ok(s.mutf8_chars(env)?.to_str().into_owned())
    })
}

/// Hand `apk_path` (in the app cache dir) to the system package installer via
/// `ACTION_VIEW`; the `ApkProvider` (see `android/app/src/main/java/…`) serves
/// the file. Needs `REQUEST_INSTALL_PACKAGES` (in the manifest); the system
/// shows its own confirmation dialog.
/// Fire the system package installer for `apk_path` (must be world-readable,
/// e.g. under the app's cache dir served by the manifest's FileProvider).
/// `provider_authority` is the app's FileProvider authority, e.g.
/// `"app.designcraft.android.apkprovider"`.
pub fn install_apk(apk_path: &std::path::Path, provider_authority: &str) -> Result<(), String> {
    with_env(|env, activity| {
        let name = apk_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| BridgeError::Msg("APK path has no file name".to_string()))?;
        let uri_str = format!("content://{provider_authority}/{name}");
        let intent_class = env.find_class(jni_str!("android/content/Intent"))?;
        let action_view = env
            .get_static_field(
                &intent_class,
                jni_str!("ACTION_VIEW"),
                jni_sig!("Ljava/lang/String;"),
            )?
            .l()?;
        let uri_j = jstring(env, &uri_str)?;
        let uri = env
            .call_static_method(
                jni_str!("android/net/Uri"),
                jni_str!("parse"),
                jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
                &[JValue::from(&uri_j)],
            )?
            .l()?;
        let mime_j = jstring(env, "application/vnd.android.package-archive")?;
        let intent = env.new_object(
            &intent_class,
            jni_sig!("(Ljava/lang/String;)V"),
            &[JValue::from(&action_view)],
        )?;
        env.call_method(
            &intent,
            jni_str!("setDataAndType"),
            jni_sig!("(Landroid/net/Uri;Ljava/lang/String;)Landroid/content/Intent;"),
            &[JValue::from(&uri), JValue::from(&mime_j)],
        )?;
        let new_task = env
            .get_static_field(
                &intent_class,
                jni_str!("FLAG_ACTIVITY_NEW_TASK"),
                jni_sig!("I"),
            )?
            .i()?;
        let grant = env
            .get_static_field(
                &intent_class,
                jni_str!("FLAG_GRANT_READ_URI_PERMISSION"),
                jni_sig!("I"),
            )?
            .i()?;
        env.call_method(
            &intent,
            jni_str!("addFlags"),
            jni_sig!("(I)Landroid/content/Intent;"),
            &[JValue::from(new_task | grant)],
        )?;
        env.call_method(
            activity,
            jni_str!("startActivity"),
            jni_sig!("(Landroid/content/Intent;)V"),
            &[JValue::from(&intent)],
        )?;
        Ok(())
    })
}
