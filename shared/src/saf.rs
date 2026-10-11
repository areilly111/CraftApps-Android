//! Storage Access Framework: "open from elsewhere" / "save a copy elsewhere"
//! pickers, the SD-card folder choice, and SAF-tree media I/O.
//!
//! Anything outside `/CraftApps` on shared storage goes through the system
//! pickers here. The app's `<App>Activity extends NativeActivity` forwards
//! `onActivityResult` to [`native_on_activity_result`] below (mangled JNI
//! name, resolved by the JVM on first call — no explicit registration, so no
//! class-loader pitfalls on the attach thread), and exposes
//! `launchForResult(Intent, int)` so pickers launch on the Activity itself
//! (the bridge's activity ref *is* the `<App>Activity` object; `find_class`
//! from an attached thread cannot see app classes).
//!
//! Intent action/category/extra strings are Android API-contract literals
//! (stable forever); only genuinely dynamic values go through reflection.
//! Every fallible step returns `Err(String)` for the UI; nothing here panics
//! (workspace lints deny it) and every `unsafe` block carries a SAFETY comment.

use crate::jni_bridge::{BridgeError, JniResult, content_resolver, jstring, with_env};
use crate::storage::StorageManager;
use jni::objects::{JObject, JString, JValue};
use jni::{Env, jni_sig, jni_str};
/// The answer to a file dialog, mirroring each app's ui-egui `FileDialogAnswer`.
/// Each port adapts its own reply type to [`DialogReply`].
#[derive(Clone, Debug)]
pub enum DialogAnswer {
    /// The chosen files on disk; the app reads them.
    Paths(Vec<String>),
    /// A chosen file's name and contents.
    Contents(String, Vec<u8>),
    /// The URI to save to.
    SaveTo(String),
}

/// Answers a pending file dialog. Each port implements this for its ui-egui
/// `FileDialogReply` (a one-line adapter).
pub trait DialogReply: Send + 'static {
    fn send(self: Box<Self>, answer: Option<DialogAnswer>);
}
use std::path::Path;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Request codes (must be < 65536) and result codes.


const REQ_OPEN_DOCUMENT: i32 = 101;
const REQ_CREATE_DOCUMENT: i32 = 102;
const REQ_OPEN_DOCUMENT_TREE: i32 = 103;
/// `Activity.RESULT_OK`.
const RESULT_OK: i32 = -1;

// Intent contract literals (android.content.Intent / android.provider.DocumentsContract).
const ACTION_OPEN_DOCUMENT: &str = "android.intent.action.OPEN_DOCUMENT";
const ACTION_CREATE_DOCUMENT: &str = "android.intent.action.CREATE_DOCUMENT";
const ACTION_OPEN_DOCUMENT_TREE: &str = "android.intent.action.OPEN_DOCUMENT_TREE";
const CATEGORY_OPENABLE: &str = "android.intent.category.OPENABLE";
const EXTRA_MIME_TYPES: &str = "android.intent.extra.MIME_TYPES";
const EXTRA_ALLOW_MULTIPLE: &str = "android.intent.extra.ALLOW_MULTIPLE";
const EXTRA_TITLE: &str = "android.intent.extra.TITLE";
// Intent flags (android.content.Intent; stable since API 1/19).
const FLAG_GRANT_READ: i32 = 0x1;
const FLAG_GRANT_WRITE: i32 = 0x2;
const FLAG_GRANT_PERSISTABLE: i32 = 0x40;

// ---------------------------------------------------------------------------
// Pending request slot: the dialog registers its reply before launching the
// picker; the native result handler answers it exactly once.

enum PendingKind {
    /// "Browse other locations…" from an Open dialog.
    Open { reply: Box<dyn DialogReply>, multiple: bool },
    /// "Save elsewhere…" from a Save dialog.
    Create { reply: Box<dyn DialogReply>, name: String },
    /// Preferences › Storage › SD card…: remember the picked tree.
    PickTree,
}

struct Pending {
    code: i32,
    kind: PendingKind,
    storage: Arc<StorageManager>,
}

static PENDING: Mutex<Option<Pending>> = Mutex::new(None);

fn set_pending(code: i32, kind: PendingKind, storage: Arc<StorageManager>) {
    if let Ok(mut slot) = PENDING.lock() {
        *slot = Some(Pending { code, kind, storage });
    }
}

/// Drop a pending request unanswered (the picker failed to launch): the reply
/// is answered as cancelled so the dialog doesn't hang.
pub fn cancel_pending() {
    let pending = PENDING.lock().unwrap_or_else(|poison| poison.into_inner()).take();
    if let Some(p) = pending {
        match p.kind {
            PendingKind::Open { reply, .. } | PendingKind::Create { reply, .. } => reply.send(None),
            PendingKind::PickTree => {}
        }
    }
}

pub fn set_pending_open(reply: Box<dyn DialogReply>, multiple: bool, storage: Arc<StorageManager>) {
    set_pending(REQ_OPEN_DOCUMENT, PendingKind::Open { reply, multiple }, storage);
}

pub fn set_pending_create(reply: Box<dyn DialogReply>, name: String, storage: Arc<StorageManager>) {
    set_pending(REQ_CREATE_DOCUMENT, PendingKind::Create { reply, name }, storage);
}

pub fn set_pending_tree(storage: Arc<StorageManager>) {
    set_pending(REQ_OPEN_DOCUMENT_TREE, PendingKind::PickTree, storage);
}

// ---------------------------------------------------------------------------
// Picker launchers (called on the UI thread).

fn new_intent<'a>(env: &mut Env<'a>, action: &str) -> JniResult<JObject<'a>> {
    let cls = env.find_class(jni_str!("android/content/Intent"))?;
    let action_j = jstring(env, action)?;
    Ok(env.new_object(&cls, jni_sig!("(Ljava/lang/String;)V"), &[JValue::from(&action_j)])?)
}

fn add_category(env: &mut Env, intent: &JObject, category: &str) -> JniResult<()> {
    let cat_j = jstring(env, category)?;
    env.call_method(
        intent,
        jni_str!("addCategory"),
        jni_sig!("(Ljava/lang/String;)Landroid/content/Intent;"),
        &[JValue::from(&cat_j)],
    )?;
    Ok(())
}

fn put_extra_bool(env: &mut Env, intent: &JObject, name: &str, value: bool) -> JniResult<()> {
    let name_j = jstring(env, name)?;
    env.call_method(
        intent,
        jni_str!("putExtra"),
        jni_sig!("(Ljava/lang/String;Z)Landroid/content/Intent;"),
        &[JValue::from(&name_j), JValue::from(value)],
    )?;
    Ok(())
}

fn put_extra_string(env: &mut Env, intent: &JObject, name: &str, value: &str) -> JniResult<()> {
    let name_j = jstring(env, name)?;
    let value_j = jstring(env, value)?;
    env.call_method(
        intent,
        jni_str!("putExtra"),
        jni_sig!("(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;"),
        &[JValue::from(&name_j), JValue::from(&value_j)],
    )?;
    Ok(())
}

fn put_extra_mime_types(env: &mut Env, intent: &JObject, types: &[String]) -> JniResult<()> {
    let name_j = jstring(env, EXTRA_MIME_TYPES)?;
    let arr = env.new_object_array(types.len() as i32, jni_str!("java/lang/String"), JObject::null())?;
    for (i, t) in types.iter().enumerate() {
        let t_j = jstring(env, t)?;
        // `set_object_array_element` is deprecated in favour of
        // `JObjectArray::set_element`, which has no `JString` bound — the old
        // method is sound, so keep it until the JNI bump.
        #[allow(deprecated)]
        env.set_object_array_element(&arr, i, &t_j)?;
    }
    env.call_method(
        intent,
        jni_str!("putExtra"),
        jni_sig!("(Ljava/lang/String;[Ljava/lang/String;)Landroid/content/Intent;"),
        &[JValue::from(&name_j), JValue::from(&arr)],
    )?;
    Ok(())
}

fn add_flags(env: &mut Env, intent: &JObject, flags: i32) -> JniResult<()> {
    env.call_method(
        intent,
        jni_str!("addFlags"),
        jni_sig!("(I)Landroid/content/Intent;"),
        &[JValue::from(flags)],
    )?;
    Ok(())
}

fn start_activity_for_result(env: &mut Env, activity: &JObject, intent: &JObject, code: i32) -> JniResult<()> {
    // `startActivityForResult` exists only on `Activity`; `activity` is the
    // bridge's ref to the app's `<App>Activity` (the very object ndk-context
    // hands us), so call its `launchForResult` directly. Never resolve the
    // activity with `find_class` here: from a thread attached with
    // `AttachCurrentThread`, Android's FindClass cannot see app classes and
    // fails with "class not found or linkage error" (found on hardware).
    env.call_method(
        activity,
        jni_str!("launchForResult"),
        jni_sig!("(Landroid/content/Intent;I)V"),
        &[JValue::from(intent), JValue::from(code)],
    )?;
    Ok(())
}

/// System file picker (`ACTION_OPEN_DOCUMENT`). `mime_types` may be empty
/// (no filter).
pub fn launch_open_document(multiple: bool, mime_types: &[String]) -> Result<(), String> {
    with_env(|env, activity| {
        let intent = new_intent(env, ACTION_OPEN_DOCUMENT)?;
        add_category(env, &intent, CATEGORY_OPENABLE)?;
        let type_j = jstring(env, "*/*")?;
        env.call_method(
            &intent,
            jni_str!("setType"),
            jni_sig!("(Ljava/lang/String;)Landroid/content/Intent;"),
            &[JValue::from(&type_j)],
        )?;
        if !mime_types.is_empty() {
            put_extra_mime_types(env, &intent, mime_types)?;
        }
        if multiple {
            put_extra_bool(env, &intent, EXTRA_ALLOW_MULTIPLE, true)?;
        }
        add_flags(env, &intent, FLAG_GRANT_READ | FLAG_GRANT_PERSISTABLE)?;
        start_activity_for_result(env, activity, &intent, REQ_OPEN_DOCUMENT)?;
        Ok(())
    })
}

/// System save picker (`ACTION_CREATE_DOCUMENT`) with a suggested file name.
pub fn launch_create_document(name: &str, mime: &str) -> Result<(), String> {
    with_env(|env, activity| {
        let intent = new_intent(env, ACTION_CREATE_DOCUMENT)?;
        add_category(env, &intent, CATEGORY_OPENABLE)?;
        let mime_j = jstring(env, mime)?;
        env.call_method(
            &intent,
            jni_str!("setType"),
            jni_sig!("(Ljava/lang/String;)Landroid/content/Intent;"),
            &[JValue::from(&mime_j)],
        )?;
        put_extra_string(env, &intent, EXTRA_TITLE, name)?;
        add_flags(env, &intent, FLAG_GRANT_READ | FLAG_GRANT_WRITE | FLAG_GRANT_PERSISTABLE)?;
        start_activity_for_result(env, activity, &intent, REQ_CREATE_DOCUMENT)?;
        Ok(())
    })
}

/// System folder picker for the SD-card media location.
pub fn launch_open_document_tree() -> Result<(), String> {
    with_env(|env, activity| {
        let intent = new_intent(env, ACTION_OPEN_DOCUMENT_TREE)?;
        add_flags(env, &intent, FLAG_GRANT_READ | FLAG_GRANT_WRITE | FLAG_GRANT_PERSISTABLE)?;
        start_activity_for_result(env, activity, &intent, REQ_OPEN_DOCUMENT_TREE)?;
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Native result handler (called on the Android main thread).

/// Body of each port's `<Activity>.nativeOnActivityResult` JNI entry. Each
/// port defines its own `#[unsafe(no_mangle)] pub unsafe extern "system" fn
/// Java_app_<name>_android_<Activity>_nativeOnActivityResult(...)` with the
/// same parameters and forwards to this function. The JVM resolves the
/// mangled name on first call; `this` is the activity (used for the
/// ContentResolver).
///
/// # Safety
/// The raw pointers must be the valid JNI args the JVM passed to the entry.
pub unsafe fn activity_result_ffi(
    raw_env: *mut jni::sys::JNIEnv,
    this_raw: jni::sys::jobject,
    request_code: jni::sys::jint,
    result_code: jni::sys::jint,
    data_raw: jni::sys::jobject,
) {
    // `raw_env` is unused on purpose: the bridge's JavaVM attaches the calling
    // thread instead, which is the same JNIEnv the JVM would hand us (this
    // runs on the Android main thread, already attached).
    let _ = raw_env;
    // Never unwind into the JVM (it would abort the process).
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_env(|env, _activity| {
            // SAFETY: the JVM passes valid local refs for the calling thread;
            // the wrappers are confined to this call and never outlive the
            // JNI frame. `data` may be null (cancelled picker); JObject
            // handles null.
            let this = unsafe { JObject::from_raw(&*env, this_raw) };
            let data = unsafe { JObject::from_raw(&*env, data_raw) };
            on_activity_result(env, &this, request_code, result_code, &data)
        })
    }));
    match r {
        Ok(Ok(())) => {}
        Ok(Err(e)) => log::error!("SAF result handling failed: {e}"),
        Err(_) => log::error!("SAF result handling panicked (caught)"),
    }
}

fn on_activity_result(
    env: &mut Env,
    activity: &JObject,
    request_code: i32,
    result_code: i32,
    data: &JObject,
) -> JniResult<()> {
    let pending = PENDING.lock().unwrap_or_else(|poison| poison.into_inner()).take();
    let Some(p) = pending else {
        log::warn!("SAF result (code {request_code}) with no pending request; ignoring");
        return Ok(());
    };
    if p.code != request_code {
        log::warn!("SAF result code {request_code} != pending {}; ignoring", p.code);
        // Answer the orphaned reply as cancelled rather than hanging the dialog.
        match p.kind {
            PendingKind::Open { reply, .. } | PendingKind::Create { reply, .. } => reply.send(None),
            PendingKind::PickTree => {}
        }
        return Ok(());
    }
    if result_code != RESULT_OK || data.is_null() {
        // Cancelled: the dialog treats it as the user backing out.
        match p.kind {
            PendingKind::Open { reply, .. } | PendingKind::Create { reply, .. } => reply.send(None),
            PendingKind::PickTree => log::info!("SD-card folder pick cancelled"),
        }
        return Ok(());
    }
    match p.kind {
        PendingKind::Open { reply, multiple } => {
            let uris = pick_uris(env, data)?;
            if uris.is_empty() {
                reply.send(None);
                return Ok(());
            }
            let mut files = Vec::with_capacity(uris.len());
            for uri in &uris {
                take_persistable(env, activity, uri, false)?;
                let uri_s = uri_to_string(env, uri)?;
                let (name, bytes) = read_uri(env, activity, uri)?;
                p.storage.record_uri_name(&uri_s, &name);
                files.push((name, bytes));
            }
            if files.len() == 1 && !multiple {
                let (name, bytes) =
                    files.into_iter().next().unwrap_or_else(|| ("untitled".to_string(), Vec::new()));
                reply.send(Some(DialogAnswer::Contents(name, bytes)));
            } else {
                // Several files: stage them into the cache and answer paths,
                // like a multi-select in the media browser.
                match stage_files(&p.storage, &files) {
                    Ok(paths) => reply.send(Some(DialogAnswer::Paths(paths))),
                    Err(e) => {
                        log::error!("SAF multi-open staging failed: {e}");
                        reply.send(None);
                    }
                }
            }
        }
        PendingKind::Create { reply, name } => {
            let uri = data_uri(env, data)?;
            take_persistable(env, activity, &uri, true)?;
            let uri_s = uri_to_string(env, &uri)?;
            p.storage.record_uri_name(&uri_s, &name);
            reply.send(Some(DialogAnswer::SaveTo(uri_s)));
        }
        PendingKind::PickTree => {
            let uri = data_uri(env, data)?;
            take_persistable(env, activity, &uri, true)?;
            let uri_s = uri_to_string(env, &uri)?;
            if let Err(e) = p.storage.set_sd_tree_uri(&uri_s) {
                log::error!("couldn't remember the SD-card folder: {e}");
            } else {
                log::info!("SD-card media folder set: {uri_s}");
            }
        }
    }
    Ok(())
}

/// The single `Intent.getData()` URI.
fn data_uri<'a>(env: &mut Env<'a>, data: &JObject<'_>) -> JniResult<JObject<'a>> {
    let uri = env.call_method(data, jni_str!("getData"), jni_sig!("()Landroid/net/Uri;"), &[])?.l()?;
    if uri.is_null() {
        return Err(BridgeError::Msg("the picker returned no URI".to_string()));
    }
    Ok(uri)
}

/// All picked URIs: `ClipData` items for a multi-pick, else `getData()`.
fn pick_uris<'a>(env: &mut Env<'a>, data: &JObject<'_>) -> JniResult<Vec<JObject<'a>>> {
    let clip = env.call_method(data, jni_str!("getClipData"), jni_sig!("()Landroid/content/ClipData;"), &[])?.l()?;
    if clip.is_null() {
        return Ok(vec![data_uri(env, data)?]);
    }
    let n = env.call_method(&clip, jni_str!("getItemCount"), jni_sig!("()I"), &[])?.i()?;
    let mut uris = Vec::new();
    let mut i = 0;
    while i < n {
        let item = env
            .call_method(&clip, jni_str!("getItemAt"), jni_sig!("(I)Landroid/content/ClipData$Item;"), &[JValue::from(i)])?
            .l()?;
        let uri = env.call_method(&item, jni_str!("getUri"), jni_sig!("()Landroid/net/Uri;"), &[])?.l()?;
        if !uri.is_null() {
            uris.push(uri);
        }
        i += 1;
    }
    Ok(uris)
}

fn uri_to_string(env: &mut Env, uri: &JObject) -> JniResult<String> {
    let s = env.call_method(uri, jni_str!("toString"), jni_sig!("()Ljava/lang/String;"), &[])?.l()?;
    let s: JString = env.cast_local::<JString>(s)?;
    Ok(s.mutf8_chars(env)?.to_str().into_owned())
}

/// Persist read (and, for trees/created files, write) access across restarts.
/// A provider that refuses persistable permission only logs: the URI still
/// works for this process lifetime.
fn take_persistable(env: &mut Env, activity: &JObject, uri: &JObject, write: bool) -> JniResult<()> {
    let resolver = content_resolver(env, activity)?;
    let mut flags = FLAG_GRANT_READ;
    if write {
        flags |= FLAG_GRANT_WRITE;
    }
    match env.call_method(
        &resolver,
        jni_str!("takePersistableUriPermission"),
        jni_sig!("(Landroid/net/Uri;I)V"),
        &[JValue::from(uri), JValue::from(flags)],
    ) {
        Ok(_) => Ok(()),
        Err(e) => {
            log::warn!("takePersistableUriPermission failed ({e}); URI works for this session only");
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// URI I/O.

/// Display name via `OpenableColumns.DISPLAY_NAME`, falling back to the URI's
/// last path segment (good enough: import sniffs magic bytes, so the name is
/// mostly cosmetic).
fn display_name(env: &mut Env, resolver: &JObject, uri: &JObject) -> JniResult<String> {
    let col = jstring(env, "_display_name")?;
    let cols = env.new_object_array(1, jni_str!("java/lang/String"), JObject::null())?;
    // See `put_extra_mime_types`: the replacement API has no `JString` bound.
    #[allow(deprecated)]
    env.set_object_array_element(&cols, 0, &col)?;
    let null = JObject::null();
    let cursor = env
        .call_method(
            resolver,
            jni_str!("query"),
            jni_sig!("(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;"),
            &[
                JValue::from(uri),
                JValue::from(&cols),
                JValue::from(&null),
                JValue::from(&null),
                JValue::from(&null),
            ],
        )?
        .l()?;
    if cursor.is_null() {
        return fallback_name(env, uri);
    }
    let name: JniResult<Option<String>> = (|| {
        let moved = env.call_method(&cursor, jni_str!("moveToFirst"), jni_sig!("()Z"), &[])?.z()?;
        if !moved {
            return Ok(None);
        }
        let s = env.call_method(&cursor, jni_str!("getString"), jni_sig!("(I)Ljava/lang/String;"), &[JValue::from(0)])?.l()?;
        if s.is_null() {
            return Ok(None);
        }
        let s: JString = env.cast_local::<JString>(s)?;
        Ok(Some(s.mutf8_chars(env)?.to_str().into_owned()))
    })();
    let _ = env.call_method(&cursor, jni_str!("close"), jni_sig!("()V"), &[]);
    match name {
        Ok(Some(n)) if !n.trim().is_empty() => Ok(n),
        _ => fallback_name(env, uri),
    }
}

/// Display-name fallback: the URI's last path segment.
fn fallback_name(env: &mut Env, uri: &JObject) -> JniResult<String> {
    let seg = env
        .call_method(uri, jni_str!("getLastPathSegment"), jni_sig!("()Ljava/lang/String;"), &[])?
        .l()?;
    if seg.is_null() {
        return Ok("untitled".to_string());
    }
    let seg: JString = env.cast_local::<JString>(seg)?;
    let seg = seg.mutf8_chars(env)?.to_str().into_owned();
    let name = seg.rsplit(['/', ':']).next().unwrap_or("untitled");
    Ok(if name.is_empty() { "untitled".to_string() } else { name.to_string() })
}

/// Read a document URI fully: display name + bytes.
fn read_uri(env: &mut Env, activity: &JObject, uri: &JObject) -> JniResult<(String, Vec<u8>)> {
    let resolver = content_resolver(env, activity)?;
    let name = display_name(env, &resolver, uri)?;
    let stream =
        env.call_method(&resolver, jni_str!("openInputStream"), jni_sig!("(Landroid/net/Uri;)Ljava/io/InputStream;"), &[JValue::from(uri)])?
            .l()?;
    if stream.is_null() {
        return Err(BridgeError::Msg(format!("couldn't open {name} for reading")));
    }
    let bytes = read_stream(env, &stream)?;
    let _ = env.call_method(&stream, jni_str!("close"), jni_sig!("()V"), &[]);
    Ok((name, bytes))
}

/// Slurp an `InputStream` into memory.
fn read_stream(env: &mut Env, stream: &JObject) -> JniResult<Vec<u8>> {
    let out = env.new_object(jni_str!("java/io/ByteArrayOutputStream"), jni_sig!("()V"), &[])?;
    let buf = env.new_byte_array(65536)?;
    loop {
        let n = env.call_method(stream, jni_str!("read"), jni_sig!("([B)I"), &[JValue::from(&buf)])?.i()?;
        if n <= 0 {
            break;
        }
        env.call_method(
            &out,
            jni_str!("write"),
            jni_sig!("([BII)V"),
            &[JValue::from(&buf), JValue::from(0i32), JValue::from(n)],
        )?;
    }
    let arr = env.call_method(&out, jni_str!("toByteArray"), jni_sig!("()[B"), &[])?.l()?;
    let arr = env.cast_local::<jni::objects::JByteArray>(arr)?;
    let len = arr.len(env)?;
    let mut raw = vec![0i8; len];
    arr.get_region(env, 0, &mut raw)?;
    Ok(raw.into_iter().map(|b| b as u8).collect())
}

/// Stage picked files into `<cache>/saf-inbox/` and return their paths (for
/// multi-open answers).
pub(crate) fn stage_files(storage: &StorageManager, files: &[(String, Vec<u8>)]) -> Result<Vec<String>, String> {
    let dir: &Path = storage.saf_inbox_dir();
    std::fs::create_dir_all(dir).map_err(|e| format!("couldn't create {}: {e}", dir.display()))?;
    let mut paths = Vec::with_capacity(files.len());
    for (i, (name, bytes)) in files.iter().enumerate() {
        let safe: String = name.chars().map(|c| if c == '/' || c == '\\' || c == ':' { '_' } else { c }).collect();
        let safe = if safe.is_empty() { format!("file-{i}") } else { format!("{i}-{safe}") };
        let dest = dir.join(safe);
        std::fs::write(&dest, bytes).map_err(|e| format!("couldn't stage {}: {e}", dest.display()))?;
        paths.push(dest.to_string_lossy().into_owned());
    }
    Ok(paths)
}

// ---------------------------------------------------------------------------
// High-level URI/tree operations (each attaches its thread as needed).

/// Read any content URI: display name + bytes.
pub fn saf_read_uri(uri_str: &str) -> Result<(String, Vec<u8>), String> {
    with_env(|env, activity| {
        let uri_j = jstring(env, uri_str)?;
        let uri = env
            .call_static_method(
                jni_str!("android/net/Uri"),
                jni_str!("parse"),
                jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
                &[JValue::from(&uri_j)],
            )?
            .l()?;
        let (name, bytes) = read_uri(env, activity, &uri)?;
        Ok((name, bytes))
    })
}

/// Overwrite a content URI (`"wt"` = write + truncate).
pub fn saf_write_uri(uri_str: &str, bytes: &[u8]) -> Result<(), String> {
    with_env(|env, activity| {
        let resolver = content_resolver(env, activity)?;
        let uri_j = jstring(env, uri_str)?;
        let uri = env
            .call_static_method(
                jni_str!("android/net/Uri"),
                jni_str!("parse"),
                jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
                &[JValue::from(&uri_j)],
            )?
            .l()?;
        let mode_j = jstring(env, "wt")?;
        let out = env
            .call_method(
                &resolver,
                jni_str!("openOutputStream"),
                jni_sig!("(Landroid/net/Uri;Ljava/lang/String;)Ljava/io/OutputStream;"),
                &[JValue::from(&uri), JValue::from(&mode_j)],
            )?
            .l()?;
        if out.is_null() {
            return Err(BridgeError::Msg("the system refused to open the file for writing".to_string()));
        }
        let arr = env.new_byte_array(bytes.len())?;
        let signed: Vec<i8> = bytes.iter().map(|b| *b as i8).collect();
        arr.set_region(env, 0, &signed)?;
        let r = env.call_method(&out, jni_str!("write"), jni_sig!("([B)V"), &[JValue::from(&arr)]);
        let c = env.call_method(&out, jni_str!("close"), jni_sig!("()V"), &[]);
        r.map_err(BridgeError::from)?;
        c.map_err(BridgeError::from)?;
        Ok(())
    })
}

/// One entry of an SAF tree.
pub struct TreeEntry {
    pub name: String,
    pub is_dir: bool,
    /// Document URI string.
    pub uri: String,
}

/// `DocumentsContract.getTreeDocumentId` / `getDocumentId`.
fn dc_string(env: &mut Env, uri: &JObject, method: &str) -> JniResult<String> {
    let method_j = jni::strings::JNIString::from(method);
    let s = env
        .call_static_method(
            jni_str!("android/provider/DocumentsContract"),
            &method_j,
            jni_sig!("(Landroid/net/Uri;)Ljava/lang/String;"),
            &[JValue::from(uri)],
        )?
        .l()?;
    let s: JString = env.cast_local::<JString>(s)?;
    Ok(s.mutf8_chars(env)?.to_str().into_owned())
}

/// List a tree's children. `parent_uri`: None = the tree root.
pub fn saf_tree_children(tree_uri: &str, parent_uri: Option<&str>) -> Result<Vec<TreeEntry>, String> {
    with_env(|env, activity| {
        let resolver = content_resolver(env, activity)?;
        let tree_j = jstring(env, tree_uri)?;
        let tree = env
            .call_static_method(
                jni_str!("android/net/Uri"),
                jni_str!("parse"),
                jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
                &[JValue::from(&tree_j)],
            )?
            .l()?;
        let parent_doc_id = match parent_uri {
            Some(p) => {
                let p_j = jstring(env, p)?;
                let p_uri = env
                    .call_static_method(
                        jni_str!("android/net/Uri"),
                        jni_str!("parse"),
                        jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
                        &[JValue::from(&p_j)],
                    )?
                    .l()?;
                dc_string(env, &p_uri, "getDocumentId")?
            }
            None => dc_string(env, &tree, "getTreeDocumentId")?,
        };
        let parent_id_j = jstring(env, &parent_doc_id)?;
        let children = env
            .call_static_method(
                jni_str!("android/provider/DocumentsContract"),
                jni_str!("buildChildUriUsingTree"),
                jni_sig!("(Landroid/net/Uri;Ljava/lang/String;)Landroid/net/Uri;"),
                &[JValue::from(&tree), JValue::from(&parent_id_j)],
            )?
            .l()?;
        // Columns: document_id, _display_name, mime_type (contract literals).
        let cols = ["document_id", "_display_name", "mime_type"];
        let arr = env.new_object_array(cols.len() as i32, jni_str!("java/lang/String"), JObject::null())?;
        for (i, c) in cols.iter().enumerate() {
            let c_j = jstring(env, c)?;
            // See `put_extra_mime_types`: the replacement API has no `JString`
            // bound, so the deprecated (sound) method stays until the JNI bump.
            #[allow(deprecated)]
            env.set_object_array_element(&arr, i, &c_j)?;
        }
        let null = JObject::null();
        let cursor = env
            .call_method(
                &resolver,
                jni_str!("query"),
                jni_sig!("(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;"),
                &[
                    JValue::from(&children),
                    JValue::from(&arr),
                    JValue::from(&null),
                    JValue::from(&null),
                    JValue::from(&null),
                ],
            )?
            .l()?;
        if cursor.is_null() {
            return Err(BridgeError::Msg("the folder couldn't be listed".to_string()));
        }
        let mut entries = Vec::new();
        let r: JniResult<()> = (|| {
            loop {
                let more = env.call_method(&cursor, jni_str!("moveToNext"), jni_sig!("()Z"), &[])?.z()?;
                if !more {
                    break;
                }
                let doc_id = {
                    let s = env
                        .call_method(&cursor, jni_str!("getString"), jni_sig!("(I)Ljava/lang/String;"), &[JValue::from(0)])?
                        .l()?;
                    let s: JString = env.cast_local::<JString>(s)?;
                    s.mutf8_chars(env)?.to_str().into_owned()
                };
                let name = {
                    let s = env
                        .call_method(&cursor, jni_str!("getString"), jni_sig!("(I)Ljava/lang/String;"), &[JValue::from(1)])?
                        .l()?;
                    if s.is_null() {
                        doc_id.clone()
                    } else {
                        let s: JString = env.cast_local::<JString>(s)?;
                        s.mutf8_chars(env)?.to_str().into_owned()
                    }
                };
                let mime = {
                    let s = env
                        .call_method(&cursor, jni_str!("getString"), jni_sig!("(I)Ljava/lang/String;"), &[JValue::from(2)])?
                        .l()?;
                    if s.is_null() {
                        String::new()
                    } else {
                        let s: JString = env.cast_local::<JString>(s)?;
                        s.mutf8_chars(env)?.to_str().into_owned()
                    }
                };
                let doc_id_j = jstring(env, &doc_id)?;
                let doc_uri = env
                    .call_static_method(
                        jni_str!("android/provider/DocumentsContract"),
                        jni_str!("buildDocumentUriUsingTree"),
                        jni_sig!("(Landroid/net/Uri;Ljava/lang/String;)Landroid/net/Uri;"),
                        &[JValue::from(&tree), JValue::from(&doc_id_j)],
                    )?
                    .l()?;
                entries.push(TreeEntry {
                    name,
                    is_dir: mime == "vnd.android.document/directory",
                    uri: uri_to_string(env, &doc_uri)?,
                });
            }
            Ok(())
        })();
        let _ = env.call_method(&cursor, jni_str!("close"), jni_sig!("()V"), &[]);
        r?;
        // Directories first, then files, case-insensitive (matches the tablet browser).
        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(entries)
    })
}

/// Create a document inside a tree; returns its URI string. `parent_uri` is a
/// document URI within the tree (the tree root for top-level creates).
pub fn saf_tree_create(parent_uri: &str, name: &str, mime: &str) -> Result<String, String> {
    with_env(|env, activity| {
        let resolver = content_resolver(env, activity)?;
        let p_j = jstring(env, parent_uri)?;
        let parent = env
            .call_static_method(
                jni_str!("android/net/Uri"),
                jni_str!("parse"),
                jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
                &[JValue::from(&p_j)],
            )?
            .l()?;
        let mime_j = jstring(env, mime)?;
        let name_j = jstring(env, name)?;
        let doc = env
            .call_static_method(
                jni_str!("android/provider/DocumentsContract"),
                jni_str!("createDocument"),
                jni_sig!("(Landroid/content/ContentResolver;Landroid/net/Uri;Ljava/lang/String;Ljava/lang/String;)Landroid/net/Uri;"),
                &[JValue::from(&resolver), JValue::from(&parent), JValue::from(&mime_j), JValue::from(&name_j)],
            )?
            .l()?;
        if doc.is_null() {
            return Err(BridgeError::Msg(format!("couldn't create {name} there")));
        }
        take_persistable(env, activity, &doc, true)?;
        uri_to_string(env, &doc)
    })
}

// ---------------------------------------------------------------------------
// MIME helpers.

/// File extension → MIME type for the pickers.
pub fn mime_for_extension(ext: &str) -> Option<&'static str> {
    Some(match ext.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "tif" | "tiff" => "image/tiff",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "psd" | "psb" => "image/vnd.adobe.photoshop",
        "pdf" => "application/pdf",
        _ => return None,
    })
}
