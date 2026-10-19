//! Host bridge (cargo feature `host-bridge`): the page that embeds the app in an `<iframe>` drives
//! it with `postMessage`, protocol `emu/1`. The rules (origins, message kinds, the exam lock,
//! autosave timing) are in `bridge_protocol.rs`; this file is the browser glue.
//!
//! Off unless the page URL names the host's origin (`?host=https://school.example`). Messages from
//! any other origin, or from any window but the parent, are ignored; replies go to that origin
//! only. `?exam=1` starts in exam mode before the host says anything; `?lang=<code>` picks the UI
//! language (English when the app has no catalog for it).
//!
//! Every message is `{bridge: "emu/1", type, …}`.
//!
//! Host → app:
//! - `hello` (or `ping`) → `ready`
//! - `open {name, bytes: ArrayBuffer | Uint8Array, requestId?}` → `opened {requestId?, name, ok, warnings?, error?}`
//! - `export {format: "psd" | "png", requestId}` → `exported {requestId, format, ok, bytes?, warnings?, error?}`
//!   (`png` is the flattened composite; `bytes` is a transferred ArrayBuffer)
//! - `exam {on}` → `exam {on}`
//! - `autosave {intervalMs}` (0 = off; clamped to 5 s – 10 min) → `autosave {intervalMs}`, then
//!   `snapshot {seq, format: "psd", ok, bytes?, error?, commandCount}` each interval in which
//!   commands ran
//! - `compact {on}` (default on) → `compact {on}`: the tablet/touch workspace (`photocraft_ui_egui::compact`);
//!   `?compact=1` starts in it
//!
//! App → host, unasked: `ready {protocol, app, version, upstream, exam, compact, language, capabilities}`
//! once at start, and `commands {ids, params}` with every engine command run since the last frame
//! (the session journal) plus `bridge.open {name, size}` for each document the host opened.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use js_sys::{Array, Object, Reflect, Uint8Array};
use photocraft_ui_egui::PhotocraftApp;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast as _, JsValue};

use crate::bridge_protocol::{self as proto, Format, Kind};

static EXAM: AtomicBool = AtomicBool::new(false);

/// Is exam mode on? (Also read by the web shell's file dialog and download services.)
pub fn exam_mode() -> bool {
    EXAM.load(Ordering::Relaxed)
}

/// [`photocraft_ui_egui::Services::locked_command`]: what the menus grey and refuse in exam mode.
pub fn locked_command(id: &str) -> bool {
    exam_mode() && proto::exam_blocks(id)
}

/// The session gate ([`photocraft_engine::Session::authorize`]) while exam mode is on: also
/// covers commands run on another command's behalf (`actions.play` steps).
fn exam_gate(id: &str, _params: &serde_json::Value) -> photocraft_engine::Result<()> {
    if locked_command(id) {
        return Err(photocraft_engine::EngineError::Other(format!("`{id}` is turned off in exam mode")));
    }
    Ok(())
}

/// Exam mode: drop dropped files and text pasted from outside before the app sees them. The
/// app's own clipboard (⌘C / ⌘V between layers) does not use these events and keeps working.
pub fn filter_input(ctx: &egui::Context) {
    if !exam_mode() {
        return;
    }
    ctx.input_mut(|i| {
        i.raw.dropped_files.clear();
        i.raw.hovered_files.clear();
        i.raw.events.retain(|e| !matches!(e, egui::Event::Paste(_)));
        i.events.retain(|e| !matches!(e, egui::Event::Paste(_)));
    });
}

enum HostMsg {
    Hello,
    Open { name: String, bytes: Vec<u8>, request_id: JsValue },
    Export { format: Option<Format>, asked: String, request_id: JsValue },
    Exam(bool),
    Autosave(Option<u32>),
    Compact(bool),
}

/// The `?host=`, `?exam=` and `?lang=` parameters.
fn query_param(name: &str) -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    web_sys::UrlSearchParams::new_with_str(&search).ok()?.get(name)
}

fn get(obj: &JsValue, key: &str) -> JsValue {
    Reflect::get(obj, &JsValue::from_str(key)).unwrap_or(JsValue::UNDEFINED)
}

fn set(obj: &Object, key: &str, value: impl Into<JsValue>) {
    let _ = Reflect::set(obj, &JsValue::from_str(key), &value.into());
}

fn strings(items: &[String]) -> Array {
    items.iter().map(|w| JsValue::from_str(w)).collect()
}

fn parse(data: &JsValue) -> Option<HostMsg> {
    let kind = proto::kind(get(data, "bridge").as_string().as_deref(), get(data, "type").as_string().as_deref())?;
    Some(match kind {
        Kind::Hello => HostMsg::Hello,
        Kind::Open => {
            let bytes = get(data, "bytes");
            let bytes = if bytes.is_instance_of::<js_sys::ArrayBuffer>() {
                Uint8Array::new(&bytes).to_vec()
            } else {
                bytes.dyn_into::<Uint8Array>().ok()?.to_vec()
            };
            let name = get(data, "name").as_string().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "starter.psd".into());
            HostMsg::Open { name, bytes, request_id: get(data, "requestId") }
        }
        Kind::Export => {
            let asked = get(data, "format").as_string();
            HostMsg::Export { format: proto::export_format(asked.as_deref()), asked: asked.unwrap_or_else(|| "psd".into()), request_id: get(data, "requestId") }
        }
        Kind::Exam => HostMsg::Exam(get(data, "on").as_bool().unwrap_or(true)),
        Kind::Autosave => HostMsg::Autosave(proto::autosave_interval(get(data, "intervalMs").as_f64())),
        Kind::Compact => HostMsg::Compact(proto::compact_request(get(data, "on").as_bool())),
    })
}

fn now_ms() -> f64 {
    js_sys::Date::now()
}

pub struct HostBridge {
    host: String,
    queue: Rc<RefCell<Vec<HostMsg>>>,
    /// Journal entries already forwarded.
    journal_seen: usize,
    /// Commands forwarded so far (the journal may be trimmed; this never goes back).
    commands: u64,
    /// Opens done by the host since the last `commands` message: (name, size).
    opens: Vec<(String, usize)>,
    gate_installed: bool,
    autosave: Option<u32>,
    autosave_due: f64,
    snapshot_seq: u64,
    /// [`Self::commands`] at the last snapshot.
    saved_at: u64,
    language: Option<&'static str>,
}

impl HostBridge {
    /// Listens for the host's messages when the page URL names a host origin; `None` otherwise.
    pub fn install(ctx: egui::Context, app: &mut PhotocraftApp) -> Option<Self> {
        let host = proto::host_origin(query_param("host").as_deref())?;
        if query_param("compact").as_deref() == Some("1") {
            photocraft_ui_egui::compact::set_on(true);
        }
        if query_param("exam").as_deref() == Some("1") {
            EXAM.store(true, Ordering::Relaxed);
        }
        app.services.locked_command = Some(locked_command);
        let language = query_param("lang").map(|code| proto::ui_language(&code));
        if let Some(code) = language {
            app.session.edit_prefs(|p| p.interface.language = code.to_string());
        }
        let window = web_sys::window()?;
        let queue: Rc<RefCell<Vec<HostMsg>>> = Rc::default();
        let (inbox, expected) = (queue.clone(), host.clone());
        let listener = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
            let parent = web_sys::window().and_then(|w| w.parent().ok().flatten());
            let from_parent = matches!((e.source(), parent), (Some(source), Some(parent)) if Object::is(&source, &parent));
            if !proto::accepts(&expected, &e.origin(), from_parent) {
                return;
            }
            if let Some(msg) = parse(&e.data()) {
                inbox.borrow_mut().push(msg);
                ctx.request_repaint();
            }
        });
        window.add_event_listener_with_callback("message", listener.as_ref().unchecked_ref()).ok()?;
        listener.forget();
        let bridge = Self {
            host,
            queue,
            journal_seen: app.session.journal.len(),
            commands: 0,
            opens: Vec::new(),
            gate_installed: false,
            autosave: None,
            autosave_due: 0.0,
            snapshot_seq: 0,
            saved_at: 0,
            language,
        };
        bridge.send_ready();
        Some(bridge)
    }

    fn post(&self, kind: &str, msg: &Object, transfer: Option<&JsValue>) {
        set(msg, "bridge", proto::PROTOCOL);
        set(msg, "type", kind);
        let Some(parent) = web_sys::window().and_then(|w| w.parent().ok().flatten()) else { return };
        let sent = match transfer {
            Some(t) => parent.post_message_with_transfer(msg, &self.host, &Array::of1(t)),
            None => parent.post_message(msg, &self.host),
        };
        if let Err(e) = sent {
            log::error!("host bridge: postMessage failed: {e:?}");
        }
    }

    fn send_ready(&self) {
        let msg = Object::new();
        set(&msg, "protocol", proto::PROTOCOL);
        set(&msg, "app", proto::APP);
        set(&msg, "version", proto::VERSION);
        set(&msg, "upstream", proto::UPSTREAM);
        set(&msg, "exam", exam_mode());
        set(&msg, "compact", photocraft_ui_egui::compact::on());
        set(&msg, "language", self.language.unwrap_or("auto"));
        set(&msg, "capabilities", proto::CAPABILITIES.iter().map(|c| JsValue::from_str(c)).collect::<Array>());
        self.post("ready", &msg, None);
    }

    /// The active document as a file, or why not.
    fn export(app: &PhotocraftApp, format: Format) -> Result<(Vec<u8>, Vec<String>), String> {
        let state = app.session.active().ok_or_else(|| "no open document".to_string())?;
        photocraft_io::export(&state.doc, format.file(), &photocraft_io::ExportOptions::default()).map(|out| (out.bytes, out.warnings)).map_err(|e| e.to_string())
    }

    /// Once per frame, after the app's own logic: answer the host's messages, keep the exam gate
    /// installed, send the commands run since the last frame, and an autosave snapshot when due.
    pub fn pump(&mut self, app: &mut PhotocraftApp, ctx: &egui::Context) {
        let messages = std::mem::take(&mut *self.queue.borrow_mut());
        for msg in messages {
            match msg {
                HostMsg::Hello => self.send_ready(),
                HostMsg::Exam(on) => {
                    EXAM.store(on, Ordering::Relaxed);
                    let reply = Object::new();
                    set(&reply, "on", on);
                    self.post("exam", &reply, None);
                    ctx.request_repaint();
                }
                HostMsg::Compact(on) => {
                    photocraft_ui_egui::compact::set_on(on);
                    let reply = Object::new();
                    set(&reply, "on", on);
                    self.post("compact", &reply, None);
                    ctx.request_repaint();
                }
                HostMsg::Autosave(interval) => {
                    self.autosave = interval;
                    self.autosave_due = now_ms() + f64::from(interval.unwrap_or(0));
                    let reply = Object::new();
                    set(&reply, "intervalMs", interval.unwrap_or(0));
                    self.post("autosave", &reply, None);
                }
                HostMsg::Open { name, bytes, request_id } => {
                    let reply = Object::new();
                    if !request_id.is_undefined() {
                        set(&reply, "requestId", request_id);
                    }
                    set(&reply, "name", name.as_str());
                    let size = bytes.len();
                    match app.open_bytes(&name, &bytes) {
                        Ok(warnings) => {
                            set(&reply, "ok", true);
                            set(&reply, "warnings", strings(&warnings));
                            self.opens.push((name.clone(), size));
                        }
                        Err(e) => {
                            set(&reply, "ok", false);
                            set(&reply, "error", e.as_str());
                        }
                    }
                    self.post("opened", &reply, None);
                    ctx.request_repaint();
                }
                HostMsg::Export { format, asked, request_id } => {
                    let reply = Object::new();
                    set(&reply, "requestId", request_id);
                    set(&reply, "format", match format { Some(f) => f.name(), None => asked.as_str() });
                    let result = format.ok_or_else(|| format!("unknown export format `{asked}` (psd or png)")).and_then(|f| Self::export(app, f));
                    match result {
                        Ok((bytes, warnings)) => {
                            let buffer = Uint8Array::from(bytes.as_slice()).buffer();
                            set(&reply, "ok", true);
                            set(&reply, "bytes", buffer.clone());
                            set(&reply, "warnings", strings(&warnings));
                            self.post("exported", &reply, Some(&buffer));
                        }
                        Err(e) => {
                            set(&reply, "ok", false);
                            set(&reply, "error", e.as_str());
                            self.post("exported", &reply, None);
                        }
                    }
                }
            }
        }
        // The gate stays installed while exam mode is on (control requests restore it after use).
        if exam_mode() && app.session.authorize.is_none() {
            app.session.authorize = Some(exam_gate);
            self.gate_installed = true;
        } else if !exam_mode() && self.gate_installed {
            app.session.authorize = None;
            self.gate_installed = false;
        }
        self.send_commands(app);
        self.autosave_tick(app, ctx);
    }

    /// The edit log: the journal entries added since the last frame, after the host's opens.
    fn send_commands(&mut self, app: &PhotocraftApp) {
        let journal = &app.session.journal;
        if journal.len() < self.journal_seen {
            self.journal_seen = 0;
        }
        if journal.len() == self.journal_seen && self.opens.is_empty() {
            return;
        }
        let (ids, params) = (Array::new(), Array::new());
        for (name, size) in self.opens.drain(..) {
            ids.push(&JsValue::from_str("bridge.open"));
            params.push(&JsValue::from_str(&serde_json::json!({"name": name, "size": size}).to_string()));
        }
        for (id, p) in &journal[self.journal_seen..] {
            ids.push(&JsValue::from_str(id));
            params.push(&JsValue::from_str(&proto::truncate_params(&p.to_string())));
            self.commands += 1;
        }
        self.journal_seen = journal.len();
        let msg = Object::new();
        set(&msg, "ids", ids);
        set(&msg, "params", params);
        self.post("commands", &msg, None);
    }

    fn autosave_tick(&mut self, app: &PhotocraftApp, ctx: &egui::Context) {
        let Some(interval) = self.autosave else { return };
        let now = now_ms();
        if !proto::snapshot_due(now, self.autosave_due, self.commands, self.saved_at) {
            if now < self.autosave_due {
                ctx.request_repaint_after(std::time::Duration::from_millis((self.autosave_due - now).max(0.0) as u64));
            } else {
                // Due, but nothing changed: look again next interval.
                self.autosave_due = now + f64::from(interval);
                ctx.request_repaint_after(std::time::Duration::from_millis(u64::from(interval)));
            }
            return;
        }
        self.autosave_due = now + f64::from(interval);
        self.saved_at = self.commands;
        self.snapshot_seq += 1;
        let msg = Object::new();
        set(&msg, "seq", self.snapshot_seq as f64);
        set(&msg, "format", "psd");
        set(&msg, "commandCount", self.commands as f64);
        match Self::export(app, Format::Psd) {
            Ok((bytes, _)) => {
                let buffer = Uint8Array::from(bytes.as_slice()).buffer();
                set(&msg, "ok", true);
                set(&msg, "bytes", buffer.clone());
                self.post("snapshot", &msg, Some(&buffer));
            }
            Err(e) => {
                set(&msg, "ok", false);
                set(&msg, "error", e.as_str());
                self.post("snapshot", &msg, None);
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(u64::from(interval)));
    }
}
