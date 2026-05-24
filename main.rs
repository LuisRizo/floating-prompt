//! floating-prompt v0.2 - Win32 floating prompt for Claude Code hooks.
//!
//! See REQUIREMENTS.md for the normative spec.
//!
//! Architecture:
//!   - Each invocation registers a request file in
//!     `%LOCALAPPDATA%\floating-prompt\queue\<millis>-<pid>.req.json`.
//!   - A poll timer (~150ms) checks "am I the oldest file in the queue?".
//!     If yes, show window. If no, stay hidden.
//!   - On answer/dismiss: delete own req file and exit. Next-oldest .exe sees
//!     itself as head on its next poll → shows its window.
//!   - Window position is read from / written to
//!     `%LOCALAPPDATA%\floating-prompt\state.json`. Saved on WM_EXITSIZEMOVE.
//!   - Drag is enabled via WM_NCHITTEST returning HTCAPTION over the message
//!     area (above the option buttons & edit box).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::{Cell, RefCell};
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentProcessId, GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};
// STILL_ACTIVE is an NTSTATUS constant (259 / 0x103). It moves between modules
// across windows-crate versions, so we just use the literal.
const STILL_ACTIVE_U32: u32 = 259;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, VK_ESCAPE, VK_RETURN};
use windows::Win32::UI::WindowsAndMessaging::*;

// ===========================================================================
// CLI args
// ===========================================================================
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
struct Args {
    event: String,
    title: String,
    message: String,
    options: Vec<String>,
}

fn parse_args() -> Args {
    let mut a = Args {
        event: "Stop".into(),
        title: "Agent needs you".into(),
        message: String::new(),
        options: Vec::new(),
    };
    let mut it = env::args().skip(1);
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--event" => a.event = it.next().unwrap_or_default(),
            "--title" => a.title = it.next().unwrap_or_default(),
            "--message" => a.message = it.next().unwrap_or_default(),
            "--options" => {
                a.options = it
                    .next()
                    .unwrap_or_default()
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            _ => {}
        }
    }
    a
}

// ===========================================================================
// Hook mode (replaces launch.ps1 - reads stdin JSON, derives args, runs the
// window, emits Claude Code decision JSON to stdout)
// ===========================================================================
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HookEvent {
    Stop,
    Question,
    Gate,
}

enum Mode {
    Cli(Args),
    Hook(HookEvent),
}

fn parse_mode() -> Mode {
    let raw: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == "--hook" {
            let ev = raw.get(i + 1).map(|s| s.as_str()).unwrap_or("Stop");
            let event = match ev {
                "Stop" => HookEvent::Stop,
                "Question" => HookEvent::Question,
                "Gate" => HookEvent::Gate,
                _ => HookEvent::Stop,
            };
            return Mode::Hook(event);
        }
        i += 1;
    }
    Mode::Cli(parse_args())
}

fn read_stdin_payload() -> serde_json::Value {
    let mut buf = String::new();
    let _ = std::io::stdin().read_to_string(&mut buf);
    if buf.trim().is_empty() {
        return serde_json::Value::Object(Default::default());
    }
    serde_json::from_str(&buf).unwrap_or(serde_json::Value::Object(Default::default()))
}

/// Read the transcript file (JSONL, one Claude message per line) and return
/// the most recent assistant text block. Mirrors launch.ps1's tail-last-50
/// behaviour for the Stop event message body.
fn tail_last_text(transcript_path: &str) -> Option<String> {
    let content = fs::read_to_string(transcript_path).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(50);
    let mut last: Option<String> = None;
    for line in &lines[start..] {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(content_arr) = v.pointer("/message/content").and_then(|c| c.as_array()) {
            for block in content_arr {
                if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                    if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                        last = Some(text.to_string());
                    }
                }
            }
        }
    }
    last
}

fn derive_args(event: HookEvent, payload: &serde_json::Value) -> Args {
    let mut a = Args {
        event: match event {
            HookEvent::Stop => "Stop".into(),
            HookEvent::Question => "Question".into(),
            HookEvent::Gate => "Gate".into(),
        },
        title: String::new(),
        message: String::new(),
        options: Vec::new(),
    };
    match event {
        HookEvent::Stop => {
            a.title = "Agent finished - reply or dismiss".into();
            a.message = "Claude finished this turn. Type a reply to keep going, or double-Esc to let it stop.".into();
            // Prefer Claude Code's pre-baked last_assistant_message field
            // (always reflects the message that just finished). Falls back to
            // transcript tailing for older Claude Code versions that don't
            // include the field. Critical: at Stop fire time the assistant's
            // latest text is NOT YET in the transcript JSONL - tailing alone
            // returns the previous text block.
            if let Some(text) = payload
                .get("last_assistant_message")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                a.message = text.to_string();
            } else if let Some(tp) = payload.get("transcript_path").and_then(|v| v.as_str()) {
                if let Some(text) = tail_last_text(tp) {
                    a.message = text;
                }
            }
        }
        HookEvent::Question => {
            a.title = "Agent has a question".into();
            a.message = "Claude needs your input.".into();
            let tool_name = payload.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
            if tool_name == "ExitPlanMode" {
                a.title = "Plan ready".into();
                a.message = "Approve the plan, or type changes.".into();
                a.options = vec!["Approve".into()];
            } else if let Some(questions) = payload
                .pointer("/tool_input/questions")
                .and_then(|v| v.as_array())
            {
                if let Some(q) = questions.first() {
                    if let Some(qtext) = q.get("question").and_then(|v| v.as_str()) {
                        a.message = qtext.to_string();
                    }
                    if let Some(opts) = q.get("options").and_then(|v| v.as_array()) {
                        a.options = opts
                            .iter()
                            .filter_map(|o| {
                                o.get("label").and_then(|v| v.as_str()).map(String::from)
                            })
                            .collect();
                    }
                }
            }
        }
        HookEvent::Gate => {
            a.title = "Permission needed".into();
            if let Some(cmd) = payload.pointer("/tool_input/command").and_then(|v| v.as_str()) {
                a.message = format!("Run: {}", cmd);
            } else if let Some(tn) = payload.get("tool_name").and_then(|v| v.as_str()) {
                a.message = format!("Allow {}?", tn);
            } else {
                a.message = "Claude wants to run a tool.".into();
            }
            a.options = vec!["Allow".into(), "Deny".into()];
        }
    }
    a
}

/// Write the last hook invocation's payload + derived args to a debug file
/// so we can diagnose transcript-parsing or args-derivation issues. Best
/// effort - failures are swallowed.
fn write_debug_log(
    event: HookEvent,
    payload: &serde_json::Value,
    derived: &Args,
) -> std::io::Result<()> {
    let dir = app_data_dir();
    fs::create_dir_all(&dir)?;
    let path = dir.join("last-hook.json");
    let mut transcript_sample = serde_json::Value::Null;
    if let Some(tp) = payload.get("transcript_path").and_then(|v| v.as_str()) {
        if let Ok(c) = fs::read_to_string(tp) {
            let lines: Vec<&str> = c.lines().collect();
            let start = lines.len().saturating_sub(5);
            transcript_sample = serde_json::Value::Array(
                lines[start..]
                    .iter()
                    .map(|l| serde_json::Value::String((*l).to_string()))
                    .collect(),
            );
        }
    }
    let dump = serde_json::json!({
        "event": format!("{:?}", event),
        "received_payload": payload,
        "derived_args": {
            "event": derived.event,
            "title": derived.title,
            "message": derived.message,
            "options": derived.options,
        },
        "transcript_last_5_lines": transcript_sample,
        "timestamp_unix_ms": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0),
    });
    fs::write(&path, serde_json::to_string_pretty(&dump).unwrap_or_default())
}

/// Gate hooks should only prompt the user in "default" permission mode.
/// In any opted-out mode (auto, acceptEdits, dontAsk, bypassPermissions,
/// plan) the user has chosen NOT to be interrupted, so the hook exits
/// silently and lets Claude Code's normal permission flow take over.
fn should_skip_gate(payload: &serde_json::Value) -> bool {
    match payload.get("permission_mode").and_then(|v| v.as_str()) {
        Some("default") | None => false,
        _ => true,
    }
}

/// Build the Claude Code decision JSON for an outcome. Returns None for
/// dismissed/empty (caller should print nothing and exit 0 to let the turn
/// proceed normally).
fn build_decision_json(event: HookEvent, outcome: &Outcome) -> Option<serde_json::Value> {
    let answer = match outcome {
        Outcome::Answered(t) => t.trim().to_string(),
        Outcome::Dismissed => return None,
    };
    if answer.is_empty() {
        return None;
    }
    Some(match event {
        HookEvent::Stop => serde_json::json!({
            "decision": "block",
            "reason": answer
        }),
        HookEvent::Gate => {
            let (decision, reason) = match answer.as_str() {
                "Allow" => ("allow", "Approved by user via floating prompt.".to_string()),
                "Deny" => ("deny", "Denied by user via floating prompt.".to_string()),
                _ => ("deny", answer),
            };
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": decision,
                    "permissionDecisionReason": reason
                }
            })
        }
        HookEvent::Question => serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": answer
            }
        }),
    })
}

// ===========================================================================
// Position persistence (R3)
// ===========================================================================
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Position {
    x: i32,
    y: i32,
}

fn app_data_dir() -> PathBuf {
    let base = env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join("floating-prompt")
}

fn state_path() -> PathBuf {
    app_data_dir().join("state.json")
}

fn load_position_from(path: &Path) -> Option<Position> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str::<Position>(&raw).ok()
}

fn save_position_to(path: &Path, pos: Position) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string(&pos).unwrap();
    fs::write(path, json)
}

/// Clamp (x, y) so the window of size (w, h) stays inside the work rect.
/// work is (left, top, right, bottom).
fn clamp_to_work(x: i32, y: i32, work: (i32, i32, i32, i32), w: i32, h: i32) -> (i32, i32) {
    let (l, t, r, b) = work;
    let max_x = (r - w).max(l);
    let max_y = (b - h).max(t);
    (x.clamp(l, max_x), y.clamp(t, max_y))
}

// ===========================================================================
// Queue management (R2)
// ===========================================================================
fn queue_dir() -> PathBuf {
    app_data_dir().join("queue")
}

fn register_request(args: &Args, pid: u32) -> PathBuf {
    let dir = queue_dir();
    let _ = fs::create_dir_all(&dir);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    // Zero-pad the millis so lexicographic sort = chronological sort for the
    // next few hundred years.
    let path = dir.join(format!("{:020}-{:010}.req.json", millis, pid));
    let json = serde_json::to_string(args).unwrap_or_else(|_| "{}".into());
    let _ = fs::write(&path, json);
    path
}

fn list_queue() -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = match fs::read_dir(queue_dir()) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.ends_with(".req.json"))
                    .unwrap_or(false)
            })
            .collect(),
        Err(_) => return Vec::new(),
    };
    entries.sort();
    entries
}

fn parse_pid(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?;
    let dash = name.find('-')?;
    let dot = name[dash + 1..].find('.')?;
    name[dash + 1..dash + 1 + dot].parse().ok()
}

fn is_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    unsafe {
        let h = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, BOOL(0), pid) {
            Ok(h) => h,
            Err(_) => return false,
        };
        let mut code: u32 = 0;
        let ok = GetExitCodeProcess(h, &mut code).is_ok();
        let _ = CloseHandle(h);
        ok && code == STILL_ACTIVE_U32
    }
}

/// Remove queue files for processes that no longer exist (e.g., crashed).
fn cleanup_stale_queue(my_path: &Path) {
    let my_pid = parse_pid(my_path);
    for path in list_queue() {
        if let Some(pid) = parse_pid(&path) {
            if Some(pid) == my_pid {
                continue;
            }
            if !is_process_alive(pid) {
                let _ = fs::remove_file(&path);
            }
        }
    }
}

/// Returns (my_position_1_indexed, total_in_queue). Position 0 means our
/// file vanished (we should exit).
fn queue_position(my_path: &Path) -> (usize, usize) {
    let q = list_queue();
    let total = q.len();
    let pos = q
        .iter()
        .position(|p| p == my_path)
        .map(|i| i + 1)
        .unwrap_or(0);
    (pos, total)
}

fn format_counter(pos: usize, total: usize) -> String {
    if total <= 1 || pos == 0 {
        String::new()
    } else {
        format!("{} of {}", pos, total)
    }
}

// ===========================================================================
// Outcome + state
// ===========================================================================
enum Outcome {
    Answered(String),
    Dismissed,
}

struct WindowState {
    args: Args,
    req_path: PathBuf,
    outcome: RefCell<Option<Outcome>>,
    is_head_shown: Cell<bool>,
    last_counter: RefCell<String>,
    h_edit: Cell<HWND>,
    h_submit: Cell<HWND>,
    h_message: Cell<HWND>,
    h_options: RefCell<Vec<HWND>>,
    h_font: Cell<HFONT>,
    h_title_font: Cell<HFONT>,
    drag_zone_bottom_y: Cell<i32>,
    window_size: Cell<(i32, i32)>,
    option_heights: RefCell<Vec<i32>>,
}

// ===========================================================================
// Global double-Esc watcher
// ===========================================================================
static DOUBLE_ESC: AtomicBool = AtomicBool::new(false);
thread_local! {
    static LAST_ESC: RefCell<Option<Instant>> = RefCell::new(None);
}
static mut KB_HOOK: HHOOK = HHOOK(0);

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && wparam.0 as u32 == WM_KEYDOWN {
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        if kb.vkCode == VK_ESCAPE.0 as u32 {
            LAST_ESC.with(|cell| {
                let now = Instant::now();
                let mut last = cell.borrow_mut();
                if let Some(prev) = *last {
                    if now.duration_since(prev).as_millis() <= 600 {
                        DOUBLE_ESC.store(true, Ordering::SeqCst);
                    }
                }
                *last = Some(now);
            });
        }
    }
    CallNextHookEx(KB_HOOK, code, wparam, lparam)
}

unsafe fn install_keyboard_hook() {
    let hmod = GetModuleHandleW(None).unwrap_or_default();
    KB_HOOK = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), hmod, 0)
        .unwrap_or(HHOOK(0));
}

unsafe fn remove_keyboard_hook() {
    if KB_HOOK.0 != 0 {
        let _ = UnhookWindowsHookEx(KB_HOOK);
        KB_HOOK = HHOOK(0);
    }
}

fn reset_double_esc() {
    DOUBLE_ESC.store(false, Ordering::SeqCst);
    LAST_ESC.with(|c| *c.borrow_mut() = None);
}

// ===========================================================================
// Layout
// ===========================================================================
const ID_ESC_TIMER: usize = 1;
const ID_POLL_TIMER: usize = 2;
const ID_SUBMIT: i32 = 99;
const ID_EDIT: i32 = 200;
const ID_MESSAGE: i32 = 300;
const ID_OPTION_BASE: i32 = 100;

const PAD: i32 = 16;
const TITLE_H: i32 = 26;
const TITLE_GAP: i32 = 8;
const SECTION_GAP: i32 = 14;
const BTN_MIN_H: i32 = 34;
const BTN_V_PAD: i32 = 14;
const BTN_V_GAP: i32 = 6;
const BTN_GAP: i32 = 8;
const EDIT_H: i32 = 28;
const SUBMIT_W: i32 = 90;
const MIN_WIDTH: i32 = 480;
const MAX_WIDTH: i32 = 640;
const MIN_HEIGHT: i32 = 180;
const MAX_HEIGHT: i32 = 720;
const MSG_BG: u32 = 0x0026_2626;
const BS_MULTILINE_U32: u32 = 0x0000_2000;
const SCROLLBAR_W: i32 = 17;
const EDIT_TEXT_INSET: i32 = 6;

const POLL_HIDDEN_MS: u32 = 150;
const POLL_SHOWN_MS: u32 = 500;
const ESC_TICK_MS: u32 = 80;

struct Layout {
    title_rect: RECT,
    counter_rect: RECT,
    msg_rect: RECT,
    option_rects: Vec<RECT>,
    edit_rect: RECT,
    submit_rect: RECT,
    drag_zone_bottom_y: i32,
}

fn layout(client_w: i32, client_h: i32, option_heights: &[i32]) -> Layout {
    let content_w = client_w - 2 * PAD;
    let footer_top = client_h - PAD - EDIT_H;
    let edit_rect = RECT {
        left: PAD,
        top: footer_top,
        right: client_w - PAD - SUBMIT_W - BTN_GAP,
        bottom: footer_top + EDIT_H,
    };
    let submit_rect = RECT {
        left: client_w - PAD - SUBMIT_W,
        top: footer_top,
        right: client_w - PAD,
        bottom: footer_top + EDIT_H,
    };

    // Vertically stacked option buttons, sitting just above the footer.
    let mut option_rects: Vec<RECT> = Vec::with_capacity(option_heights.len());
    let options_top = if option_heights.is_empty() {
        footer_top
    } else {
        let n = option_heights.len() as i32;
        let total_h: i32 = option_heights.iter().sum::<i32>() + BTN_V_GAP * (n - 1);
        let top = footer_top - SECTION_GAP - total_h;
        let mut y = top;
        for h in option_heights {
            option_rects.push(RECT {
                left: PAD,
                top: y,
                right: PAD + content_w,
                bottom: y + h,
            });
            y += h + BTN_V_GAP;
        }
        top
    };

    let title_rect = RECT {
        left: PAD,
        top: PAD,
        right: client_w - PAD - 80,
        bottom: PAD + TITLE_H,
    };
    let counter_rect = RECT {
        left: client_w - PAD - 80,
        top: PAD,
        right: client_w - PAD,
        bottom: PAD + TITLE_H,
    };

    let msg_top = PAD + TITLE_H + TITLE_GAP;
    let msg_bottom = if option_heights.is_empty() {
        footer_top - SECTION_GAP
    } else {
        options_top - SECTION_GAP
    };
    let msg_rect = RECT {
        left: PAD,
        top: msg_top,
        right: client_w - PAD,
        bottom: msg_bottom,
    };
    let drag_zone_bottom_y = msg_bottom;
    Layout {
        title_rect,
        counter_rect,
        msg_rect,
        option_rects,
        edit_rect,
        submit_rect,
        drag_zone_bottom_y,
    }
}

// ===========================================================================
// Helpers
// ===========================================================================
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn state_ptr(hwnd: HWND) -> Option<*const WindowState> {
    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    if p == 0 {
        None
    } else {
        Some(p as *const WindowState)
    }
}

unsafe fn get_edit_text(h_edit: HWND) -> String {
    let len = GetWindowTextLengthW(h_edit);
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; (len + 1) as usize];
    let got = GetWindowTextW(h_edit, &mut buf);
    if got <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..got as usize]).trim().to_string()
}

unsafe fn get_work_area() -> (i32, i32, i32, i32) {
    let mut work = RECT::default();
    let _ = SystemParametersInfoW(
        SPI_GETWORKAREA,
        0,
        Some(&mut work as *mut _ as *mut _),
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
    );
    (work.left, work.top, work.right, work.bottom)
}

/// Create the body UI font (Segoe UI 10pt regular). Used for the message,
/// option labels, edit box, and Send button.
unsafe fn create_ui_font() -> HFONT {
    let font_name = wide("Segoe UI");
    CreateFontW(
        -13,
        0,
        0,
        0,
        FW_NORMAL.0 as i32,
        0,
        0,
        0,
        DEFAULT_CHARSET.0 as u32,
        OUT_DEFAULT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32,
        CLEARTYPE_QUALITY.0 as u32,
        (DEFAULT_PITCH.0 as u32) | (FF_DONTCARE.0 as u32),
        PCWSTR(font_name.as_ptr()),
    )
}

/// Create the title font (Segoe UI 12pt semibold) for the top bar only —
/// gives the window a clear visual hierarchy without a full theming pass.
unsafe fn create_title_font() -> HFONT {
    let font_name = wide("Segoe UI Semibold");
    CreateFontW(
        -16,
        0,
        0,
        0,
        600,
        0,
        0,
        0,
        DEFAULT_CHARSET.0 as u32,
        OUT_DEFAULT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32,
        CLEARTYPE_QUALITY.0 as u32,
        (DEFAULT_PITCH.0 as u32) | (FF_DONTCARE.0 as u32),
        PCWSTR(font_name.as_ptr()),
    )
}

/// Generic wrapped-text measurement against the real UI font.
unsafe fn measure_wrapped(text: &str, available_w: i32) -> i32 {
    if text.trim().is_empty() {
        return 0;
    }
    let hdc = GetDC(HWND::default());
    let font = create_ui_font();
    let old = SelectObject(hdc, font);
    let mut rc = RECT { left: 0, top: 0, right: available_w, bottom: 0 };
    let mut t = wide(text);
    DrawTextW(hdc, &mut t, &mut rc, DT_LEFT | DT_WORDBREAK | DT_CALCRECT);
    SelectObject(hdc, old);
    let _ = DeleteObject(font);
    ReleaseDC(HWND::default(), hdc);
    (rc.bottom - rc.top).max(0)
}

/// Measure single-line text width — used to decide window width based on
/// the longest option label.
unsafe fn measure_single_line_width(text: &str) -> i32 {
    if text.is_empty() {
        return 0;
    }
    let hdc = GetDC(HWND::default());
    let font = create_ui_font();
    let old = SelectObject(hdc, font);
    let mut rc = RECT { left: 0, top: 0, right: 4096, bottom: 0 };
    let mut t = wide(text);
    DrawTextW(hdc, &mut t, &mut rc, DT_LEFT | DT_SINGLELINE | DT_CALCRECT);
    SelectObject(hdc, old);
    let _ = DeleteObject(font);
    ReleaseDC(HWND::default(), hdc);
    (rc.right - rc.left).max(0)
}

unsafe fn measure_message_height(message: &str, available_w: i32) -> i32 {
    measure_wrapped(message, available_w)
}

/// Measure the height of an option button, given the inner content width
/// available for the label. Wraps long labels; respects BTN_MIN_H so even
/// short labels get a comfortable click target.
unsafe fn measure_option_height(label: &str, inner_w: i32) -> i32 {
    let text_h = measure_wrapped(label, inner_w);
    (text_h + BTN_V_PAD).max(BTN_MIN_H)
}

/// Pure height composer — given pre-measured message height and per-option
/// heights, return the total client-area height (clamped). Easy to unit-test.
fn compose_window_height(msg_h: i32, option_heights: &[i32]) -> i32 {
    let options_block_h = if option_heights.is_empty() {
        0
    } else {
        let n = option_heights.len() as i32;
        option_heights.iter().sum::<i32>() + BTN_V_GAP * (n - 1) + SECTION_GAP
    };
    let raw_h =
        PAD + TITLE_H + TITLE_GAP + msg_h + SECTION_GAP + options_block_h + EDIT_H + PAD;
    raw_h.clamp(MIN_HEIGHT, MAX_HEIGHT)
}

/// Legacy convenience: kept so the existing tests + any path without a DC
/// can size a window assuming single-line option buttons.
#[allow(dead_code)]
fn compute_window_size(args: &Args, msg_h: i32) -> (i32, i32) {
    let heights: Vec<i32> = (0..args.options.len()).map(|_| BTN_MIN_H).collect();
    (MIN_WIDTH, compose_window_height(msg_h, &heights))
}

/// Sizing plan with everything measured: width chosen from option labels,
/// per-option heights, message height at that width. Used by run_window to
/// create a window that fits its content from the first frame.
struct SizingPlan {
    width: i32,
    height: i32,
    option_heights: Vec<i32>,
}

fn plan_window_size(args: &Args) -> SizingPlan {
    // Width: grow with the longest option label so labels don't have to wrap
    // unless they are truly long. ~28px of label padding (button chrome).
    let longest_opt_w = args
        .options
        .iter()
        .map(|s| unsafe { measure_single_line_width(s) })
        .max()
        .unwrap_or(0);
    let preferred_w = (longest_opt_w + 28 + 2 * PAD).max(MIN_WIDTH);
    let width = preferred_w.clamp(MIN_WIDTH, MAX_WIDTH);

    let content_w = width - 2 * PAD;
    let label_inner_w = (content_w - 24).max(80); // 12px L/R inside button

    let option_heights: Vec<i32> = args
        .options
        .iter()
        .map(|s| unsafe { measure_option_height(s, label_inner_w) })
        .collect();

    // Measure message at the actual text width inside the EDIT control: take
    // out the scrollbar gutter + the edit's internal text inset so the
    // measured height matches the rendered wrap exactly.
    let msg_text_w = (content_w - SCROLLBAR_W - 2 * EDIT_TEXT_INSET).max(80);
    let msg_h_text = unsafe { measure_message_height(&args.message, msg_text_w) };
    let msg_h = msg_h_text + 2 * EDIT_TEXT_INSET; // pad inside the control
    let height = compose_window_height(msg_h, &option_heights);

    SizingPlan { width, height, option_heights }
}

// ===========================================================================
// Window procedure
// ===========================================================================
unsafe extern "system" fn wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            let cs = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
            SetTimer(hwnd, ID_ESC_TIMER, ESC_TICK_MS, None);
            SetTimer(hwnd, ID_POLL_TIMER, POLL_HIDDEN_MS, None);
            create_children(hwnd, cs.lpCreateParams as *mut WindowState);
            LRESULT(0)
        }
        WM_TIMER => {
            match wparam.0 {
                ID_ESC_TIMER => {
                    if let Some(state) = state_ptr(hwnd) {
                        if (*state).is_head_shown.get() && DOUBLE_ESC.load(Ordering::SeqCst) {
                            *(*state).outcome.borrow_mut() = Some(Outcome::Dismissed);
                            let _ = DestroyWindow(hwnd);
                        }
                    }
                }
                ID_POLL_TIMER => {
                    poll_queue(hwnd);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => {
            let hdc = HDC(wparam.0 as isize);
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            let bg = CreateSolidBrush(COLORREF(0x001E_1E1E));
            FillRect(hdc, &rc, bg);
            let _ = DeleteObject(bg);
            LRESULT(1)
        }
        WM_NCHITTEST => {
            // Drag the borderless window by clicking anywhere in the upper
            // (non-control) region. Below the drag zone, fall through to the
            // controls so they get clicks normally.
            let screen_x = (lparam.0 & 0xFFFF) as i16 as i32;
            let screen_y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let mut pt = POINT { x: screen_x, y: screen_y };
            let _ = ScreenToClient(hwnd, &mut pt);
            if let Some(state) = state_ptr(hwnd) {
                if pt.y < (*state).drag_zone_bottom_y.get() && pt.y >= 0 {
                    return LRESULT(HTCAPTION as isize);
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_EXITSIZEMOVE => {
            // User finished dragging. Save new position.
            let mut wr = RECT::default();
            let _ = GetWindowRect(hwnd, &mut wr);
            let _ = save_position_to(&state_path(), Position { x: wr.left, y: wr.top });
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => {
            let cur = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let cleared = cur & !(WS_EX_NOACTIVATE.0 as isize);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, cleared);
            LRESULT(MA_ACTIVATE as isize)
        }
        WM_CTLCOLOREDIT => {
            let hdc = HDC(wparam.0 as isize);
            SetTextColor(hdc, COLORREF(0x00FF_FFFF));
            SetBkColor(hdc, COLORREF(0x002D_2D2D));
            let brush = CreateSolidBrush(COLORREF(0x002D_2D2D));
            LRESULT(brush.0 as isize)
        }
        WM_CTLCOLORSTATIC => {
            // Read-only EDITs send CTLCOLORSTATIC. Only one such control in
            // this window (the message view), so a single theme is fine.
            let hdc = HDC(wparam.0 as isize);
            SetTextColor(hdc, COLORREF(0x00E0_E0E0));
            SetBkColor(hdc, COLORREF(MSG_BG));
            let brush = CreateSolidBrush(COLORREF(MSG_BG));
            LRESULT(brush.0 as isize)
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;
            let notify = ((wparam.0 >> 16) & 0xFFFF) as u32;
            if notify == BN_CLICKED {
                if let Some(state) = state_ptr(hwnd) {
                    if id == ID_SUBMIT {
                        let text = get_edit_text((*state).h_edit.get());
                        if !text.is_empty() {
                            *(*state).outcome.borrow_mut() = Some(Outcome::Answered(text));
                            let _ = DestroyWindow(hwnd);
                        }
                    } else if id >= ID_OPTION_BASE {
                        let idx = (id - ID_OPTION_BASE) as usize;
                        if let Some(label) = (&(*state).args.options).get(idx) {
                            *(*state).outcome.borrow_mut() =
                                Some(Outcome::Answered(label.clone()));
                            let _ = DestroyWindow(hwnd);
                        }
                    }
                }
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            if let Some(state) = state_ptr(hwnd) {
                if (*state).outcome.borrow().is_none() {
                    *(*state).outcome.borrow_mut() = Some(Outcome::Dismissed);
                }
            }
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            if let Some(state) = state_ptr(hwnd) {
                let f = (*state).h_font.get();
                if f.0 != 0 {
                    let _ = DeleteObject(f);
                }
                let tf = (*state).h_title_font.get();
                if tf.0 != 0 {
                    let _ = DeleteObject(tf);
                }
            }
            let _ = KillTimer(hwnd, ID_ESC_TIMER);
            let _ = KillTimer(hwnd, ID_POLL_TIMER);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ===========================================================================
// Poll loop: am I head? Update counter. Show if newly head.
// ===========================================================================
unsafe fn poll_queue(hwnd: HWND) {
    let state = match state_ptr(hwnd) {
        Some(s) => s,
        None => return,
    };
    cleanup_stale_queue(&(*state).req_path);
    let (pos, total) = queue_position(&(*state).req_path);
    if pos == 0 {
        // Our own file vanished (manual cleanup, etc.). Bail.
        *(*state).outcome.borrow_mut() = Some(Outcome::Dismissed);
        let _ = DestroyWindow(hwnd);
        return;
    }
    let new_counter = format_counter(pos, total);
    if *(*state).last_counter.borrow() != new_counter {
        *(*state).last_counter.borrow_mut() = new_counter;
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let _ = InvalidateRect(hwnd, Some(&rc), BOOL(0));
    }
    if pos == 1 && !(*state).is_head_shown.get() {
        // Transition: become head + show window.
        reset_double_esc();
        let (w, h) = (*state).window_size.get();
        let work = get_work_area();
        let (x, y) = match load_position_from(&state_path()) {
            Some(p) => clamp_to_work(p.x, p.y, work, w, h),
            None => (work.2 - w - 16, work.3 - h - 16),
        };
        let _ = SetWindowPos(hwnd, HWND_TOPMOST, x, y, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = UpdateWindow(hwnd);
        (*state).is_head_shown.set(true);
        // Slow the poll once we're shown — only counter refresh remains.
        SetTimer(hwnd, ID_POLL_TIMER, POLL_SHOWN_MS, None);
    }
}

// ===========================================================================
// Child controls
// ===========================================================================
unsafe fn create_children(hwnd: HWND, state_ptr: *mut WindowState) {
    let state = &mut *state_ptr;
    let hinstance: HINSTANCE = GetModuleHandleW(None).unwrap_or_default().into();

    let h_font = create_ui_font();
    state.h_font.set(h_font);
    let h_title_font = create_title_font();
    state.h_title_font.set(h_title_font);

    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let heights = state.option_heights.borrow().clone();
    let lay = layout(rc.right, rc.bottom, &heights);
    state.drag_zone_bottom_y.set(lay.drag_zone_bottom_y);

    let edit_class = wide("EDIT");
    let empty = wide("");

    // Read-only multiline message view with vertical scroll. The text comes
    // from args.message and is set via SetWindowTextW below.
    let h_message = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        PCWSTR(edit_class.as_ptr()),
        PCWSTR(empty.as_ptr()),
        WINDOW_STYLE(
            WS_CHILD.0
                | WS_VISIBLE.0
                | WS_VSCROLL.0
                | ES_MULTILINE as u32
                | ES_READONLY as u32
                | ES_AUTOVSCROLL as u32
                | ES_LEFT as u32,
        ),
        lay.msg_rect.left,
        lay.msg_rect.top,
        lay.msg_rect.right - lay.msg_rect.left,
        lay.msg_rect.bottom - lay.msg_rect.top,
        hwnd,
        HMENU(ID_MESSAGE as isize),
        hinstance,
        None,
    );
    state.h_message.set(h_message);
    SendMessageW(h_message, WM_SETFONT, WPARAM(h_font.0 as usize), LPARAM(1));
    // EDIT controls need CRLF for line breaks; LF alone renders as garbage.
    let msg_normalized = state
        .args
        .message
        .replace("\r\n", "\n")
        .replace('\n', "\r\n");
    let msg_text = wide(&msg_normalized);
    let _ = SetWindowTextW(h_message, PCWSTR(msg_text.as_ptr()));

    let h_edit = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        PCWSTR(edit_class.as_ptr()),
        PCWSTR(empty.as_ptr()),
        WINDOW_STYLE(
            WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | ES_AUTOHSCROLL as u32 | ES_LEFT as u32,
        ),
        lay.edit_rect.left,
        lay.edit_rect.top,
        lay.edit_rect.right - lay.edit_rect.left,
        lay.edit_rect.bottom - lay.edit_rect.top,
        hwnd,
        HMENU(ID_EDIT as isize),
        hinstance,
        None,
    );
    state.h_edit.set(h_edit);
    SendMessageW(h_edit, WM_SETFONT, WPARAM(h_font.0 as usize), LPARAM(1));

    let btn_class = wide("BUTTON");
    let send_label = wide("Send");
    let h_submit = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        PCWSTR(btn_class.as_ptr()),
        PCWSTR(send_label.as_ptr()),
        WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_DEFPUSHBUTTON as u32),
        lay.submit_rect.left,
        lay.submit_rect.top,
        lay.submit_rect.right - lay.submit_rect.left,
        lay.submit_rect.bottom - lay.submit_rect.top,
        hwnd,
        HMENU(ID_SUBMIT as isize),
        hinstance,
        None,
    );
    state.h_submit.set(h_submit);
    SendMessageW(h_submit, WM_SETFONT, WPARAM(h_font.0 as usize), LPARAM(1));

    let mut buttons = Vec::with_capacity(state.args.options.len());
    for (i, label) in state.args.options.iter().enumerate() {
        let id = ID_OPTION_BASE + i as i32;
        let rect = &lay.option_rects[i];
        let label_w = wide(label);
        let h_btn = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(btn_class.as_ptr()),
            PCWSTR(label_w.as_ptr()),
            WINDOW_STYLE(
                WS_CHILD.0
                    | WS_VISIBLE.0
                    | WS_TABSTOP.0
                    | BS_PUSHBUTTON as u32
                    | BS_MULTILINE_U32,
            ),
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            hwnd,
            HMENU(id as isize),
            hinstance,
            None,
        );
        SendMessageW(h_btn, WM_SETFONT, WPARAM(h_font.0 as usize), LPARAM(1));
        buttons.push(h_btn);
    }
    *state.h_options.borrow_mut() = buttons;
}

// ===========================================================================
// Paint
// ===========================================================================
unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);

    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);

    SetBkMode(hdc, TRANSPARENT);

    if let Some(state) = state_ptr(hwnd) {
        let body_font = (*state).h_font.get();
        let title_font = (*state).h_title_font.get();

        let heights = (*state).option_heights.borrow().clone();
        let lay = layout(rc.right, rc.bottom, &heights);

        // Title — semibold, full white. Message body is painted by the
        // read-only EDIT child (see WM_CTLCOLORSTATIC for its colors).
        let old_font = if title_font.0 != 0 {
            SelectObject(hdc, title_font)
        } else {
            HGDIOBJ(0)
        };
        SetTextColor(hdc, COLORREF(0x00FF_FFFF));
        let mut title = wide(&(*state).args.title);
        let mut tr = lay.title_rect;
        DrawTextW(hdc, &mut title, &mut tr, DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS);

        // Switch to body font for the queue counter.
        if body_font.0 != 0 {
            SelectObject(hdc, body_font);
        }

        let counter = (*state).last_counter.borrow().clone();
        if !counter.is_empty() {
            SetTextColor(hdc, COLORREF(0x0099_99FF));
            let mut c = wide(&counter);
            let mut cr = lay.counter_rect;
            DrawTextW(hdc, &mut c, &mut cr, DT_RIGHT | DT_SINGLELINE);
        }

        if old_font.0 != 0 {
            SelectObject(hdc, old_font);
        }
    }

    let _ = EndPaint(hwnd, &ps);
}

// ===========================================================================
// Create the top-level window (hidden initially — shown when we become head)
// ===========================================================================
unsafe fn create_window(state: *mut WindowState) -> Result<HWND> {
    let hinstance: HINSTANCE = GetModuleHandleW(None)?.into();
    let class_name = wide("FloatingPromptShell");

    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wndproc),
        hInstance: hinstance,
        hCursor: LoadCursorW(HINSTANCE::default(), IDC_ARROW)?,
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };
    RegisterClassW(&wc);

    let (w, h) = (*state).window_size.get();
    // Initial off-screen placement; poll_queue will reposition before showing.
    let x = -10000;
    let y = -10000;

    let title = wide(&(*state).args.title);

    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
        PCWSTR(class_name.as_ptr()),
        PCWSTR(title.as_ptr()),
        WS_POPUP | WS_BORDER,
        x,
        y,
        w,
        h,
        HWND::default(),
        HMENU::default(),
        hinstance,
        Some(state as *const _ as *const core::ffi::c_void),
    );
    if hwnd.0 == 0 {
        return Err(Error::from_win32());
    }
    // Stays hidden until poll_queue detects head status and calls ShowWindow.
    Ok(hwnd)
}

// ===========================================================================
// run_window: queue + show + collect outcome. Used by both CLI and Hook mode.
// ===========================================================================
fn run_window(args: Args) -> Outcome {
    let pid = unsafe { GetCurrentProcessId() };
    let req_path = register_request(&args, pid);

    // Plan window dimensions so it fits the message + the (possibly
    // multi-line, vertically stacked) option buttons from the first frame.
    let plan = plan_window_size(&args);
    let size = (plan.width, plan.height);

    let state = Box::into_raw(Box::new(WindowState {
        args,
        req_path: req_path.clone(),
        outcome: RefCell::new(None),
        is_head_shown: Cell::new(false),
        last_counter: RefCell::new(String::new()),
        h_edit: Cell::new(HWND::default()),
        h_submit: Cell::new(HWND::default()),
        h_message: Cell::new(HWND::default()),
        h_options: RefCell::new(Vec::new()),
        h_font: Cell::new(HFONT(0)),
        h_title_font: Cell::new(HFONT(0)),
        drag_zone_bottom_y: Cell::new(0),
        window_size: Cell::new(size),
        option_heights: RefCell::new(plan.option_heights),
    }));

    let outcome = unsafe {
        install_keyboard_hook();
        let hwnd = match create_window(state) {
            Ok(h) => h,
            Err(_) => {
                let _ = Box::from_raw(state);
                remove_keyboard_hook();
                let _ = fs::remove_file(&req_path);
                return Outcome::Dismissed;
            }
        };

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, HWND::default(), 0, 0).into() {
            // Intercept Enter directly: IsDialogMessageW's default-button
            // translation is unreliable on WS_EX_NOACTIVATE popups. If focus
            // is inside our window, treat Enter as a Submit click.
            if msg.message == WM_KEYDOWN && msg.wParam.0 as u32 == VK_RETURN.0 as u32 {
                let focused = GetFocus();
                if focused == hwnd || IsChild(hwnd, focused).as_bool() {
                    let wp = ((BN_CLICKED as usize) << 16) | (ID_SUBMIT as usize);
                    SendMessageW(hwnd, WM_COMMAND, WPARAM(wp), LPARAM(0));
                    continue;
                }
            }
            if !IsDialogMessageW(hwnd, &msg).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        remove_keyboard_hook();
        let state = Box::from_raw(state);
        state.outcome.into_inner().unwrap_or(Outcome::Dismissed)
    };

    let _ = fs::remove_file(&req_path);
    outcome
}

// ===========================================================================
// main: dispatch between CLI mode (test/manual) and Hook mode (live)
// ===========================================================================
fn main() {
    match parse_mode() {
        Mode::Cli(args) => match run_window(args) {
            Outcome::Answered(text) => {
                println!("{text}");
                std::process::exit(0);
            }
            Outcome::Dismissed => std::process::exit(10),
        },
        Mode::Hook(event) => {
            let payload = read_stdin_payload();
            // Respect user's permission mode: Gate hooks pop up only in
            // "default" mode. In auto/acceptEdits/etc., exit silently so
            // the normal Claude Code flow handles it.
            if event == HookEvent::Gate && should_skip_gate(&payload) {
                std::process::exit(0);
            }
            let args = derive_args(event, &payload);
            // Debug log: write the last hook invocation so we can diagnose
            // when Claude's transcript isn't being parsed as expected.
            // Overwrites each call; small file; safe to leave on.
            let _ = write_debug_log(event, &payload, &args);
            let outcome = run_window(args);
            if let Some(json) = build_decision_json(event, &outcome) {
                println!("{}", json);
            }
            std::process::exit(0);
        }
    }
}

// ===========================================================================
// Tests (cargo test)
// ===========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    // ---- format_counter ----
    #[test]
    fn counter_hidden_when_alone() {
        assert_eq!(format_counter(1, 1), "");
        assert_eq!(format_counter(1, 0), "");
    }

    #[test]
    fn counter_shown_when_queued() {
        assert_eq!(format_counter(1, 3), "1 of 3");
        assert_eq!(format_counter(2, 5), "2 of 5");
    }

    #[test]
    fn counter_hidden_when_zero_pos() {
        // Position 0 means our file vanished — don't render a misleading counter.
        assert_eq!(format_counter(0, 3), "");
    }

    // ---- clamp_to_work ----
    #[test]
    fn clamp_keeps_in_bounds_position() {
        let work = (0, 0, 1920, 1080);
        assert_eq!(clamp_to_work(100, 200, work, 480, 280), (100, 200));
    }

    #[test]
    fn clamp_pulls_offscreen_right() {
        let work = (0, 0, 1920, 1080);
        let (x, _) = clamp_to_work(5000, 100, work, 480, 280);
        assert_eq!(x, 1920 - 480);
    }

    #[test]
    fn clamp_pulls_offscreen_below() {
        let work = (0, 0, 1920, 1080);
        let (_, y) = clamp_to_work(100, 5000, work, 480, 280);
        assert_eq!(y, 1080 - 280);
    }

    #[test]
    fn clamp_pulls_offscreen_negative() {
        let work = (0, 0, 1920, 1080);
        let (x, y) = clamp_to_work(-500, -500, work, 480, 280);
        assert_eq!((x, y), (0, 0));
    }

    #[test]
    fn clamp_with_offset_work_area() {
        // Multi-monitor: secondary monitor work area starts at x=1920.
        let work = (1920, 0, 3840, 1080);
        let (x, y) = clamp_to_work(100, 100, work, 480, 280);
        assert_eq!((x, y), (1920, 100));
    }

    // ---- Position round-trip ----
    #[test]
    fn position_roundtrip_through_file() {
        let dir = std::env::temp_dir().join(format!("fp-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("state-rt.json");
        let _ = std::fs::remove_file(&path);
        save_position_to(&path, Position { x: 123, y: 456 }).unwrap();
        let loaded = load_position_from(&path).unwrap();
        assert_eq!(loaded, Position { x: 123, y: 456 });
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn position_load_returns_none_when_missing() {
        let path = std::env::temp_dir().join(format!("fp-missing-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        assert!(load_position_from(&path).is_none());
    }

    #[test]
    fn position_load_returns_none_when_malformed() {
        let path =
            std::env::temp_dir().join(format!("fp-malformed-{}.json", std::process::id()));
        std::fs::write(&path, "not json").unwrap();
        assert!(load_position_from(&path).is_none());
        let _ = std::fs::remove_file(&path);
    }

    // ---- parse_pid ----
    #[test]
    fn parse_pid_basic() {
        let p = PathBuf::from(r"C:\foo\00000000001700000000000-0000012345.req.json");
        assert_eq!(parse_pid(&p), Some(12345));
    }

    #[test]
    fn parse_pid_returns_none_for_garbage() {
        let p = PathBuf::from(r"C:\foo\not-a-real-name.json");
        assert_eq!(parse_pid(&p), None);
    }

    // ---- derive_args ----
    #[test]
    fn derive_stop_uses_default_message_without_transcript() {
        let p = serde_json::json!({});
        let a = derive_args(HookEvent::Stop, &p);
        assert_eq!(a.event, "Stop");
        assert!(a.title.contains("Agent finished"));
        assert!(a.message.contains("Claude finished"));
        assert!(a.options.is_empty());
    }

    #[test]
    fn derive_stop_prefers_last_assistant_message_over_transcript() {
        // Even when transcript_path is provided, last_assistant_message wins.
        let dir = std::env::temp_dir().join(format!("fp-prefer-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let tp = dir.join("transcript.jsonl");
        let stale = serde_json::json!({"message":{"content":[{"type":"text","text":"OLD TRANSCRIPT TEXT"}]}});
        std::fs::write(&tp, format!("{}\n", stale)).unwrap();
        let p = serde_json::json!({
            "transcript_path": tp.to_string_lossy(),
            "last_assistant_message": "FRESH FROM PAYLOAD"
        });
        let a = derive_args(HookEvent::Stop, &p);
        assert_eq!(a.message, "FRESH FROM PAYLOAD");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn derive_stop_falls_back_to_transcript_when_last_assistant_missing() {
        let dir = std::env::temp_dir().join(format!("fp-fallback-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let tp = dir.join("transcript.jsonl");
        let msg = serde_json::json!({"message":{"content":[{"type":"text","text":"only via transcript"}]}});
        std::fs::write(&tp, format!("{}\n", msg)).unwrap();
        let p = serde_json::json!({"transcript_path": tp.to_string_lossy()});
        let a = derive_args(HookEvent::Stop, &p);
        assert_eq!(a.message, "only via transcript");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn derive_stop_reads_transcript_when_present() {
        let dir = std::env::temp_dir().join(format!("fp-tr-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let tp = dir.join("transcript.jsonl");
        let line1 = serde_json::json!({"message":{"content":[{"type":"text","text":"first"}]}});
        let line2 = serde_json::json!({"message":{"content":[{"type":"text","text":"latest"}]}});
        std::fs::write(&tp, format!("{}\n{}\n", line1, line2)).unwrap();
        let p = serde_json::json!({"transcript_path": tp.to_string_lossy()});
        let a = derive_args(HookEvent::Stop, &p);
        assert_eq!(a.message, "latest");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn derive_question_picks_first_question_and_options() {
        let p = serde_json::json!({
            "tool_input": {
                "questions": [{
                    "question": "Which library should we use?",
                    "options": [{"label":"react"}, {"label":"vue"}, {"label":"solid"}]
                }]
            }
        });
        let a = derive_args(HookEvent::Question, &p);
        assert_eq!(a.message, "Which library should we use?");
        assert_eq!(a.options, vec!["react".to_string(), "vue".to_string(), "solid".to_string()]);
    }

    #[test]
    fn derive_question_handles_exit_plan_mode() {
        let p = serde_json::json!({"tool_name": "ExitPlanMode"});
        let a = derive_args(HookEvent::Question, &p);
        assert!(a.title.contains("Plan"));
        assert_eq!(a.options, vec!["Approve".to_string()]);
    }

    #[test]
    fn derive_gate_shows_bash_command() {
        let p = serde_json::json!({
            "tool_name": "Bash",
            "tool_input": {"command": "rm -rf node_modules"}
        });
        let a = derive_args(HookEvent::Gate, &p);
        assert_eq!(a.message, "Run: rm -rf node_modules");
        assert_eq!(a.options, vec!["Allow".to_string(), "Deny".to_string()]);
    }

    #[test]
    fn derive_gate_falls_back_to_tool_name() {
        let p = serde_json::json!({"tool_name": "Write"});
        let a = derive_args(HookEvent::Gate, &p);
        assert_eq!(a.message, "Allow Write?");
    }

    // ---- derive_args preserves full message length ----
    #[test]
    fn derive_does_not_truncate_long_messages() {
        let long = "x".repeat(2000);
        let p = serde_json::json!({"last_assistant_message": long});
        let a = derive_args(HookEvent::Stop, &p);
        assert_eq!(a.message.chars().count(), 2000);
    }

    // ---- build_decision_json ----
    #[test]
    fn decision_none_for_dismiss() {
        let d = build_decision_json(HookEvent::Stop, &Outcome::Dismissed);
        assert!(d.is_none());
    }

    #[test]
    fn decision_none_for_empty_answer() {
        let d = build_decision_json(HookEvent::Stop, &Outcome::Answered("   ".into()));
        assert!(d.is_none());
    }

    #[test]
    fn decision_stop_emits_block_with_reason() {
        let d = build_decision_json(HookEvent::Stop, &Outcome::Answered("keep going".into()))
            .unwrap();
        assert_eq!(d.get("decision").and_then(|v| v.as_str()), Some("block"));
        assert_eq!(d.get("reason").and_then(|v| v.as_str()), Some("keep going"));
    }

    #[test]
    fn decision_gate_allow_emits_allow_decision() {
        let d = build_decision_json(HookEvent::Gate, &Outcome::Answered("Allow".into())).unwrap();
        let pd = d.pointer("/hookSpecificOutput/permissionDecision");
        assert_eq!(pd.and_then(|v| v.as_str()), Some("allow"));
    }

    #[test]
    fn decision_gate_deny_emits_deny_decision() {
        let d = build_decision_json(HookEvent::Gate, &Outcome::Answered("Deny".into())).unwrap();
        let pd = d.pointer("/hookSpecificOutput/permissionDecision");
        assert_eq!(pd.and_then(|v| v.as_str()), Some("deny"));
    }

    #[test]
    fn decision_gate_free_text_is_deny_with_text_as_reason() {
        let d = build_decision_json(HookEvent::Gate, &Outcome::Answered("explain first".into()))
            .unwrap();
        let pd = d.pointer("/hookSpecificOutput/permissionDecision");
        let pr = d.pointer("/hookSpecificOutput/permissionDecisionReason");
        assert_eq!(pd.and_then(|v| v.as_str()), Some("deny"));
        assert_eq!(pr.and_then(|v| v.as_str()), Some("explain first"));
    }

    #[test]
    fn decision_question_always_deny_with_reason() {
        let d = build_decision_json(HookEvent::Question, &Outcome::Answered("use vue".into()))
            .unwrap();
        let pd = d.pointer("/hookSpecificOutput/permissionDecision");
        let pr = d.pointer("/hookSpecificOutput/permissionDecisionReason");
        assert_eq!(pd.and_then(|v| v.as_str()), Some("deny"));
        assert_eq!(pr.and_then(|v| v.as_str()), Some("use vue"));
    }

    // ---- tail_last_text ----
    #[test]
    fn tail_returns_none_for_missing_file() {
        assert!(tail_last_text("Z:\\does\\not\\exist.jsonl").is_none());
    }

    #[test]
    fn tail_returns_latest_text_block() {
        let dir = std::env::temp_dir().join(format!("fp-tail-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let tp = dir.join("t.jsonl");
        let lines = vec![
            serde_json::json!({"message":{"content":[{"type":"text","text":"old"}]}}).to_string(),
            serde_json::json!({"message":{"content":[{"type":"tool_use"}]}}).to_string(),
            serde_json::json!({"message":{"content":[{"type":"text","text":"newer"}]}})
                .to_string(),
            "garbage line that should be skipped".to_string(),
            serde_json::json!({"message":{"content":[{"type":"text","text":"newest"}]}})
                .to_string(),
        ];
        std::fs::write(&tp, lines.join("\n")).unwrap();
        let got = tail_last_text(tp.to_str().unwrap());
        assert_eq!(got.as_deref(), Some("newest"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- compute_window_size / compose_window_height ----
    #[test]
    fn window_size_minimum_for_short_message() {
        let a = Args { event: "Stop".into(), title: "t".into(), message: "short".into(), options: vec![] };
        let (w, h) = compute_window_size(&a, 18);
        assert_eq!(w, MIN_WIDTH);
        assert_eq!(h, MIN_HEIGHT);
    }

    #[test]
    fn window_size_grows_with_message_height() {
        let a = Args { event: "Stop".into(), title: "t".into(), message: "long".into(), options: vec![] };
        let (_w, h_small) = compute_window_size(&a, 18);
        let (_, h_big) = compute_window_size(&a, 400);
        assert!(h_big > h_small);
    }

    #[test]
    fn window_size_caps_at_max_height() {
        let a = Args { event: "Stop".into(), title: "t".into(), message: "long".into(), options: vec![] };
        let (_w, h) = compute_window_size(&a, 99999);
        assert_eq!(h, MAX_HEIGHT);
    }

    #[test]
    fn window_size_adds_button_row_when_options_present() {
        let a_no = Args { event: "Stop".into(), title: "t".into(), message: "x".into(), options: vec![] };
        let a_opts = Args { event: "Question".into(), title: "t".into(), message: "x".into(), options: vec!["Allow".into(), "Deny".into()] };
        let (_, h_no) = compute_window_size(&a_no, 200);
        let (_, h_opts) = compute_window_size(&a_opts, 200);
        assert!(h_opts > h_no, "options block should add height (no_opts={}, with_opts={})", h_no, h_opts);
    }

    #[test]
    fn compose_height_grows_with_more_stacked_options() {
        let h_one = compose_window_height(120, &[BTN_MIN_H]);
        let h_three = compose_window_height(120, &[BTN_MIN_H, BTN_MIN_H, BTN_MIN_H]);
        assert!(h_three > h_one, "stacking more options should grow height (1={}, 3={})", h_one, h_three);
    }

    #[test]
    fn compose_height_grows_with_taller_option_buttons() {
        let small = compose_window_height(120, &[BTN_MIN_H, BTN_MIN_H]);
        let tall = compose_window_height(120, &[BTN_MIN_H * 2, BTN_MIN_H * 2]);
        assert!(tall > small, "taller buttons should grow height (small={}, tall={})", small, tall);
    }

    #[test]
    fn compose_height_clamps_to_min_for_tiny_inputs() {
        assert_eq!(compose_window_height(0, &[]), MIN_HEIGHT);
    }

    // ---- layout stacking ----
    #[test]
    fn layout_stacks_options_vertically() {
        let heights = [BTN_MIN_H, BTN_MIN_H, BTN_MIN_H];
        let lay = layout(MIN_WIDTH, 600, &heights);
        assert_eq!(lay.option_rects.len(), 3);
        // All buttons share the same x (left + right).
        let left = lay.option_rects[0].left;
        let right = lay.option_rects[0].right;
        for r in &lay.option_rects {
            assert_eq!(r.left, left);
            assert_eq!(r.right, right);
        }
        // Each next button sits strictly below the previous.
        assert!(lay.option_rects[1].top > lay.option_rects[0].bottom);
        assert!(lay.option_rects[2].top > lay.option_rects[1].bottom);
    }

    #[test]
    fn layout_options_span_full_content_width() {
        let heights = [BTN_MIN_H];
        let lay = layout(MIN_WIDTH, 600, &heights);
        let r = &lay.option_rects[0];
        assert_eq!(r.left, PAD);
        assert_eq!(r.right, MIN_WIDTH - PAD);
    }

    #[test]
    fn layout_message_rect_shrinks_when_options_present() {
        let no_opts = layout(MIN_WIDTH, 600, &[]);
        let with_opts = layout(MIN_WIDTH, 600, &[BTN_MIN_H, BTN_MIN_H]);
        assert!(
            with_opts.msg_rect.bottom < no_opts.msg_rect.bottom,
            "message area should shrink to make room for stacked options"
        );
    }

    // ---- queue path generation ----
    #[test]
    fn register_creates_file_with_pid_in_name() {
        // Use a temp queue dir via env override to avoid touching real LOCALAPPDATA.
        let testdir = std::env::temp_dir().join(format!("fp-q-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&testdir);
        std::env::set_var("LOCALAPPDATA", &testdir);

        static FAKE_PID: AtomicU32 = AtomicU32::new(0);
        let pid = FAKE_PID.fetch_add(1, Ordering::SeqCst) + 99001;

        let args = Args {
            event: "Stop".into(),
            title: "t".into(),
            message: "m".into(),
            options: vec!["A".into(), "B".into()],
        };
        let path = register_request(&args, pid);
        assert!(path.exists(), "request file should exist at {:?}", path);
        assert_eq!(parse_pid(&path), Some(pid));

        // Round-trip the args back out of the file.
        let raw = std::fs::read_to_string(&path).unwrap();
        let parsed: Args = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed.event, "Stop");
        assert_eq!(parsed.options, vec!["A".to_string(), "B".to_string()]);

        // Don't leave the file around to confuse other tests.
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&testdir);
    }
}
