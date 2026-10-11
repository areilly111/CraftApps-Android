//! ScriptUI hosts: who owns a script window's JavaScript objects, and how user actions reach
//! its handlers.
//!
//! * **Script files** (native builds) run on a *script-host thread*. The caller's session moves
//!   to that thread while JavaScript runs and comes back whenever the script stops: at the end,
//!   or while a modal dialog waits in `show()`. Each [`ScriptUiEvent`] moves the session there
//!   again, runs the handler, and brings it back with the windows' new state. The host ends
//!   when its last window closes. Only one side holds the session at a time, so nothing runs
//!   concurrently and every edit is still an ordinary undoable engine command.
//! * **The Script Console** (and every run on the web) runs inline: its windows' handlers run in
//!   the console's (or a kept) context on the caller's thread, and dialogs don't block.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicU32, Ordering};

use boa_engine::{Context, JsResult, JsValue, js_string};
use effectcraft_engine::Session;
use effectcraft_engine::scriptui::{ScriptUiEvent, ScriptWindow, layout};
use serde_json::{Value as J, json};

use crate::runtime::{ACTIVE, Active, CONSOLE, Restore, ScriptError, arg_string, js_str, outcome, script_error, settle, throw, with_active};

pub(crate) const SCRIPTUI: &str = include_str!("scriptui.js");

/// The Script Console's host id.
pub(crate) const CONSOLE_HOST: u32 = u32::MAX;

static NEXT_WINDOW: AtomicU32 = AtomicU32::new(1);
static NEXT_HOST: AtomicU32 = AtomicU32::new(1);

pub(crate) fn new_host_id() -> u32 {
    NEXT_HOST.fetch_add(1, Ordering::Relaxed)
}

thread_local! {
    /// Contexts of inline runs whose palettes / panels are still open (dropped when they close).
    static INLINE: RefCell<BTreeMap<u32, ManuallyDrop<Context>>> = const { RefCell::new(BTreeMap::new()) };
}

pub(crate) fn keep_inline(host: u32, ctx: Context) {
    INLINE.with(|m| m.borrow_mut().insert(host, ManuallyDrop::new(ctx)));
}

fn drop_inline(host: u32) {
    if let Some(ctx) = INLINE.with(|m| m.borrow_mut().remove(&host)) {
        drop(ManuallyDrop::into_inner(ctx));
    }
}

// ---------------------------------------------------------------- JavaScript side

fn call_global(ctx: &mut Context, name: &str, args: &[JsValue]) -> JsResult<JsValue> {
    let f = ctx.global_object().get(js_string!(name), ctx)?;
    let f = f.as_object().ok_or_else(|| throw(format!("{name} is not defined")))?;
    f.call(&JsValue::undefined(), args, ctx)
}

/// The open windows of a context (closed ones are reported once, invisible, then forgotten).
pub(crate) fn snapshot(ctx: &mut Context) -> Vec<ScriptWindow> {
    let Ok(v) = call_global(ctx, "__uiSnapshot", &[]) else { return vec![] };
    let s = v.to_string(ctx).map(|s| s.to_std_string_escaped()).unwrap_or_default();
    serde_json::from_str(&s).unwrap_or_default()
}

/// Run a window event's handlers in `ctx`.
fn dispatch_js(ctx: &mut Context, ev: &ScriptUiEvent) -> Result<J, ScriptError> {
    let args = [JsValue::from(ev.window), JsValue::from(ev.widget), js_str(&ev.kind), js_str(&ev.value.to_string())];
    match call_global(ctx, "__uiDispatch", &args) {
        Ok(_) => Ok(J::Null),
        Err(e) => {
            let mut err = script_error(&e, ctx, "ScriptUI handler", "");
            err.message = format!("{} handler: {}", ev.kind, err.message);
            Err(err)
        }
    }
}

/// `__uiNewId()`: a window id unique in the process.
pub(crate) fn native_new_id(_: &JsValue, _: &[JsValue], _: &mut Context) -> JsResult<JsValue> {
    Ok(JsValue::from(NEXT_WINDOW.fetch_add(1, Ordering::Relaxed)))
}

/// `__uiLayoutNative(windowJson)` → `{controlId: [x, y, width, height]}`: ScriptUI's automatic
/// layout ([`effectcraft_engine::scriptui::layout`]) for `layout.layout()`.
pub(crate) fn native_layout(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let s = arg_string(args, 0, ctx)?;
    let mut w: ScriptWindow = serde_json::from_str(&s).map_err(|e| throw(format!("layout: {e}")))?;
    layout(&mut w);
    let mut all = vec![];
    w.root.walk(&mut all);
    let map: serde_json::Map<String, J> = all.iter().map(|c| (c.id.to_string(), json!(c.bounds))).collect();
    Ok(js_str(&J::Object(map).to_string()))
}

/// `__uiModal(windowId)`: a dialog's `show()`. On a script-host thread it hands the session back
/// to the caller and waits for the dialog's events until it closes, returning its result. Inline
/// (Script Console, web) it returns `null` at once: the dialog stays open like a palette.
pub(crate) fn native_modal(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = args.first().cloned().unwrap_or_default().to_number(ctx)? as u32;
    let linked = with_active(|a| Ok(a.link.is_some())).unwrap_or(false);
    if !linked {
        return Ok(JsValue::null());
    }
    loop {
        let r = call_global(ctx, "__uiModalResult", &[JsValue::from(id)])?;
        if !r.is_null() && !r.is_undefined() {
            return Ok(r);
        }
        let windows = snapshot(ctx);
        let out = crate::Outcome { output: settle(), waiting: true, ..Default::default() };
        let Some(ev) = yield_wait(out, windows, false) else {
            // The caller went away: cancel the dialog.
            return Ok(JsValue::from(2));
        };
        if let Err(e) = dispatch_js(ctx, &ev) {
            let line = e.line.map(|l| format!(" (line {l})")).unwrap_or_default();
            let _ = with_active(|a| {
                a.output.push(format!("Error{line}: {}", e.message));
                Ok(())
            });
        }
    }
}

// ---------------------------------------------------------------- script-host threads

pub(crate) enum ToHost {
    /// Run the script (script-host threads exist only off the web: the browser runs scripts
    /// inline).
    #[cfg(not(target_arch = "wasm32"))]
    Start(Session),
    Event(Session, ScriptUiEvent),
}

impl ToHost {
    /// The session a message carries (given back when the host has gone).
    fn into_session(self) -> Session {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            ToHost::Start(s) => s,
            ToHost::Event(s, _) => s,
        }
    }
}

pub(crate) struct FromHost {
    session: Session,
    outcome: crate::Outcome,
    windows: Vec<ScriptWindow>,
    name: String,
    /// The host thread has ended (no windows left).
    done: bool,
}

/// The host thread's end of the hand-off.
pub(crate) struct Link {
    rx: std::sync::mpsc::Receiver<ToHost>,
    tx: std::sync::mpsc::Sender<FromHost>,
}

/// The caller's end.
struct HostEnd {
    tx: std::sync::mpsc::Sender<ToHost>,
    rx: std::sync::mpsc::Receiver<FromHost>,
}

fn hosts() -> &'static std::sync::Mutex<BTreeMap<u32, HostEnd>> {
    static H: std::sync::OnceLock<std::sync::Mutex<BTreeMap<u32, HostEnd>>> = std::sync::OnceLock::new();
    H.get_or_init(Default::default)
}

// The session crosses to script-host threads and back.
const _: fn() = || {
    fn send<T: Send>() {}
    send::<Session>();
};

/// On the host thread: hand the session (with the windows' state and what the script printed)
/// back to the caller, and wait for the next event. `None` when the caller has gone.
fn yield_wait(outcome: crate::Outcome, windows: Vec<ScriptWindow>, done: bool) -> Option<ScriptUiEvent> {
    let (session, link, name) = ACTIVE.with(|a| {
        let mut a = a.borrow_mut();
        let a = a.as_mut()?;
        Some((std::mem::take(&mut a.session), a.link.take()?, a.name.clone()))
    })?;
    if link.tx.send(FromHost { session, outcome, windows, name, done }).is_err() {
        return None;
    }
    if done {
        return None;
    }
    match link.rx.recv() {
        Ok(ToHost::Event(s, ev)) => {
            ACTIVE.with(|a| {
                if let Some(a) = a.borrow_mut().as_mut() {
                    a.session = s;
                    a.link = Some(link);
                }
            });
            Some(ev)
        }
        _ => None,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn host_main(name: String, code: String, link: Link) {
    let Ok(ToHost::Start(session)) = link.rx.recv() else { return };
    let mut a = Active::new(session, &name);
    a.link = Some(link);
    ACTIVE.with(|c| *c.borrow_mut() = Some(a));
    let file = if name.is_empty() { "script".to_string() } else { name.clone() };
    let body = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut ctx = match crate::runtime::new_context() {
            Ok(c) => c,
            Err(e) => {
                yield_wait(outcome(settle(), Err(ScriptError { message: e, file: file.clone(), ..Default::default() })), vec![], true);
                return;
            }
        };
        let r = crate::runtime::eval_with_tasks(&mut ctx, &code, &file);
        let mut out = outcome(settle(), r);
        loop {
            let windows = snapshot(&mut ctx);
            let open = windows.iter().any(|w| w.visible);
            match yield_wait(out, windows, !open) {
                Some(ev) => {
                    let r = dispatch_js(&mut ctx, &ev);
                    out = outcome(settle(), r);
                }
                None => return,
            }
        }
    }));
    // A panic: give the session back all the same.
    let left = ACTIVE.with(|c| c.borrow_mut().take());
    if let Some(mut a) = left
        && let Some(link) = a.link.take()
    {
        let message = if body.is_err() { "the script engine stopped unexpectedly" } else { "the script ended" };
        let out = crate::Outcome { error: Some(ScriptError { message: message.into(), file, ..Default::default() }), ..Default::default() };
        let _ = link.tx.send(FromHost { session: std::mem::take(&mut a.session), outcome: out, windows: vec![], name, done: true });
    }
}

/// Wait for the host to hand the session back; keep its link while it has windows open.
fn receive(session: &mut Session, host: u32, end: HostEnd) -> crate::Outcome {
    match end.rx.recv() {
        Ok(FromHost { session: s, outcome, windows, name, done }) => {
            *session = s;
            publish(session, host, &name, windows);
            if done {
                session.script_ui.windows.retain(|w| w.host != host);
                session.script_ui.revision += 1;
            } else if let Ok(mut h) = hosts().lock() {
                h.insert(host, end);
            }
            outcome
        }
        Err(_) => crate::Outcome { error: Some(ScriptError { message: "the script host stopped".into(), ..Default::default() }), ..Default::default() },
    }
}

/// Run a script file on a new script-host thread (see the module docs). Not on the web, where
/// scripts run inline.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn run_threaded(session: &mut Session, req: &effectcraft_engine::ScriptRequest) -> crate::Outcome {
    let host = new_host_id();
    let (to_tx, to_rx) = std::sync::mpsc::channel();
    let (from_tx, from_rx) = std::sync::mpsc::channel();
    let (name, code) = (req.name.to_string(), req.code.to_string());
    let spawned =
        std::thread::Builder::new().name(format!("script-{host}")).stack_size(64 << 20).spawn(move || host_main(name, code, Link { rx: to_rx, tx: from_tx }));
    if spawned.is_err() {
        return crate::runtime::run_inline(session, req);
    }
    if let Err(std::sync::mpsc::SendError(msg)) = to_tx.send(ToHost::Start(std::mem::take(session))) {
        *session = msg.into_session();
        return crate::Outcome { error: Some(ScriptError { message: "the script host did not start".into(), ..Default::default() }), ..Default::default() };
    }
    receive(session, host, HostEnd { tx: to_tx, rx: from_rx })
}

/// Record the windows a host published (named after the script that made them).
pub(crate) fn publish(session: &mut Session, host: u32, name: &str, mut windows: Vec<ScriptWindow>) {
    for w in &mut windows {
        if w.script.is_empty() {
            w.script = name.to_string();
        }
    }
    let had = session.script_ui.windows.iter().any(|w| w.host == host);
    if windows.is_empty() && !had {
        return;
    }
    session.script_ui.publish(host, windows);
    if host != CONSOLE_HOST && !session.script_ui.windows.iter().any(|w| w.host == host) {
        drop_inline(host);
    }
}

/// The [`effectcraft_engine::scriptui::ScriptUiDispatch`] this crate provides: run a window
/// event's handlers in the script that made the window, then publish its windows' new state.
/// Returns the handler run's `{ok, output, error, waiting?}`.
pub fn dispatch_ui(session: &mut Session, ev: &ScriptUiEvent) -> Result<J, String> {
    if ACTIVE.with(|a| a.try_borrow().map(|g| g.is_some()).unwrap_or(true)) {
        return Err("scripts can't drive script windows".into());
    }
    let w = session.script_ui.window(ev.window).ok_or_else(|| format!("no script window {}", ev.window))?;
    let (host, name) = (w.host, w.script.clone());
    let inline = host == CONSOLE_HOST || INLINE.with(|m| m.borrow().contains_key(&host));
    if inline {
        return Ok(dispatch_inline(session, host, &name, ev).to_json());
    }
    let end = hosts().lock().ok().and_then(|mut h| h.remove(&host));
    let Some(end) = end else {
        session.script_ui.windows.retain(|w| w.host != host);
        session.script_ui.revision += 1;
        return Err("the script that made this window has ended".into());
    };
    if let Err(std::sync::mpsc::SendError(msg)) = end.tx.send(ToHost::Event(std::mem::take(session), ev.clone())) {
        *session = msg.into_session();
        session.script_ui.windows.retain(|w| w.host != host);
        session.script_ui.revision += 1;
        return Err("the script that made this window has ended".into());
    }
    Ok(receive(session, host, end).to_json())
}

fn dispatch_inline(session: &mut Session, host: u32, name: &str, ev: &ScriptUiEvent) -> crate::Outcome {
    let s = std::mem::take(session);
    ACTIVE.with(|a| *a.borrow_mut() = Some(Active::new(s, name)));
    let guard = Restore { target: session };
    let (r, windows) = if host == CONSOLE_HOST {
        CONSOLE.with(|c| match c.borrow_mut().as_mut() {
            Some(ctx) => {
                let r = dispatch_js(ctx, ev);
                (r, snapshot(ctx))
            }
            None => (Ok(J::Null), vec![]),
        })
    } else {
        INLINE.with(|m| match m.borrow_mut().get_mut(&host) {
            Some(ctx) => {
                let r = dispatch_js(ctx, ev);
                (r, snapshot(ctx))
            }
            None => (Ok(J::Null), vec![]),
        })
    };
    let output = settle();
    drop(guard);
    publish(session, host, name, windows);
    outcome(output, r)
}
