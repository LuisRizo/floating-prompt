//! floating-prompt v0.3 — Win32 floating prompt for Claude Code hooks.
//!
//! See REQUIREMENTS.md (R1–R7) for the normative spec and `design/` for the
//! visual reference (popup.jsx + palettes.js + artboards.jsx).
//!
//! Architecture (unchanged from v0.2):
//!   - Each invocation registers a request file in
//!     `%LOCALAPPDATA%\floating-prompt\queue\<millis>-<pid>.req.json`.
//!   - A poll timer (~150ms) checks "am I the oldest file in the queue?".
//!     If yes, show window. If no, stay hidden.
//!   - On answer/dismiss: delete own req file and exit. Next-oldest .exe sees
//!     itself as head on its next poll → shows its window.
//!   - Window position is read from / written to
//!     `%LOCALAPPDATA%\floating-prompt\state.json`. Saved on WM_EXITSIZEMOVE.
//!
//! Rendering (v0.3): the popup is fully owner-drawn via Direct2D + DirectWrite.
//! The only real child control is the single-line EDIT used for free-text
//! input. Everything else (session chip, drag grip, queue badge, message
//! panel, option cards, dismiss cluster) is painted directly in WM_PAINT.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
#[cfg(test)]
use std::time::Duration;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::System::Threading::{
    GetCurrentProcessId, GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};
const STILL_ACTIVE_U32: u32 = 259;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, VK_A, VK_BACK, VK_C, VK_CONTROL, VK_DELETE, VK_DOWN,
    VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_UP, VK_V, VK_X,
};
use windows::Win32::UI::WindowsAndMessaging::*;

// ===========================================================================
// Palettes — embedded from design/palettes.js
// ===========================================================================
#[derive(Clone, Copy, Debug)]
struct Palette {
    name: &'static str,
    bg: u32,
    panel: u32,
    chip: u32,
    chip_border: u32,
    accent: u32,
    accent_soft: u32, // already-mixed soft variant (alpha applied)
    option_bg: u32,
    option_hover: u32,
    option_border: u32,
    option_number: u32,
    input_bg: u32,
    input_border: u32,
    body: u32,
    title: u32,
    dim: u32,
    scroll_thumb: u32,
    /// Subtle inset tint behind inline `code` and fenced code blocks. Sits a
    /// step darker than `panel` so code reads as recessed inside the message.
    code_bg: u32,
}

// Colors are 0xAARRGGBB. accent_soft uses alpha; everything else opaque.
const SOFT_ALPHA: u32 = 0x2E_00_00_00; // ~0.18 (matches design's accent_soft)

// Border contrasts are pushed harder than the design tokens because D2D's 1px
// stroke at integer coordinates is half-pixel-aliased to ~50% intensity. The
// original web design relied on subpixel browser AA to read crisp at low
// contrast; on Win32 the same hex values disappear.
const PALETTES: &[Palette] = &[
    Palette {
        name: "slate",
        bg: 0xFF_1A_1D_23,
        panel: 0xFF_20_24_2B,
        chip: 0xFF_27_2C_34,
        chip_border: 0xFF_52_5C_6C,
        accent: 0xFF_86_B0_D8,
        accent_soft: SOFT_ALPHA | 0x00_86_B0_D8,
        option_bg: 0xFF_25_2A_32,
        option_hover: 0xFF_2C_32_3C,
        option_border: 0xFF_4D_56_64,
        option_number: 0xFF_6A_72_80,
        input_bg: 0xFF_1D_21_28,
        input_border: 0xFF_47_50_5E,
        body: 0xFF_D6_DA_E0,
        title: 0xFF_F2_F4_F7,
        dim: 0xFF_7A_81_8C,
        scroll_thumb: 0xFF_3A_41_4C,
        code_bg: 0xFF_16_1A_20,
    },
    Palette {
        name: "ocean",
        bg: 0xFF_0F_1A_24,
        panel: 0xFF_14_22_31,
        chip: 0xFF_1A_2C_40,
        chip_border: 0xFF_3F_64_85,
        accent: 0xFF_5F_D0_C4,
        accent_soft: SOFT_ALPHA | 0x00_5F_D0_C4,
        option_bg: 0xFF_18_28_38,
        option_hover: 0xFF_1E_32_4A,
        option_border: 0xFF_3F_64_85,
        option_number: 0xFF_5E_7A_92,
        input_bg: 0xFF_11_20_2F,
        input_border: 0xFF_34_56_75,
        body: 0xFF_CA_D8_E4,
        title: 0xFF_EB_F2_F7,
        dim: 0xFF_6A_80_94,
        scroll_thumb: 0xFF_2C_42_58,
        code_bg: 0xFF_0C_18_25,
    },
    Palette {
        name: "amber",
        bg: 0xFF_1A_16_12,
        panel: 0xFF_22_1D_17,
        chip: 0xFF_2C_24_1B,
        chip_border: 0xFF_5C_4B_36,
        accent: 0xFF_E8_A0_4A,
        accent_soft: SOFT_ALPHA | 0x00_E8_A0_4A,
        option_bg: 0xFF_25_20_1A,
        option_hover: 0xFF_2E_28_20,
        option_border: 0xFF_56_47_36,
        option_number: 0xFF_7A_6D_5A,
        input_bg: 0xFF_1D_18_13,
        input_border: 0xFF_4A_3D_2E,
        body: 0xFF_D6_CD_C0,
        title: 0xFF_F5_ED_E0,
        dim: 0xFF_85_7B_6B,
        scroll_thumb: 0xFF_3D_33_28,
        code_bg: 0xFF_1A_14_10,
    },
    Palette {
        name: "forest",
        bg: 0xFF_13_18_15,
        panel: 0xFF_18_1F_1B,
        chip: 0xFF_1D_28_20,
        chip_border: 0xFF_45_5A_4A,
        accent: 0xFF_7E_C5_95,
        accent_soft: SOFT_ALPHA | 0x00_7E_C5_95,
        option_bg: 0xFF_1C_24_20,
        option_hover: 0xFF_22_2C_26,
        option_border: 0xFF_42_55_47,
        option_number: 0xFF_6A_7A_70,
        input_bg: 0xFF_16_1C_18,
        input_border: 0xFF_38_4A_3E,
        body: 0xFF_C8_D0_C8,
        title: 0xFF_EB_F2_EB,
        dim: 0xFF_74_83_78,
        scroll_thumb: 0xFF_2F_3A_33,
        code_bg: 0xFF_10_16_12,
    },
    Palette {
        name: "plum",
        bg: 0xFF_1A_16_20,
        panel: 0xFF_22_1C_2A,
        chip: 0xFF_2A_22_36,
        chip_border: 0xFF_5A_48_72,
        accent: 0xFF_C8_A3_E6,
        accent_soft: SOFT_ALPHA | 0x00_C8_A3_E6,
        option_bg: 0xFF_25_1F_30,
        option_hover: 0xFF_2C_24_38,
        option_border: 0xFF_55_45_6A,
        option_number: 0xFF_7A_6E_88,
        input_bg: 0xFF_1D_18_25,
        input_border: 0xFF_46_3A_58,
        body: 0xFF_D2_C8_D8,
        title: 0xFF_F0_E8_F5,
        dim: 0xFF_85_78_90,
        scroll_thumb: 0xFF_3A_2F_48,
        code_bg: 0xFF_18_12_22,
    },
    Palette {
        name: "default",
        bg: 0xFF_17_17_19,
        panel: 0xFF_1E_1E_21,
        chip: 0xFF_27_27_2B,
        chip_border: 0xFF_55_55_5C,
        accent: 0xFF_E8_E8_EA,
        accent_soft: 0x24_E8_E8_EA,
        option_bg: 0xFF_23_23_26,
        option_hover: 0xFF_2A_2A_2E,
        option_border: 0xFF_50_50_57,
        option_number: 0xFF_75_75_7A,
        input_bg: 0xFF_1A_1A_1D,
        input_border: 0xFF_43_43_4A,
        body: 0xFF_D5_D5_D8,
        title: 0xFF_F5_F5_F7,
        dim: 0xFF_82_82_86,
        scroll_thumb: 0xFF_3A_3A_40,
        code_bg: 0xFF_13_13_15,
    },
];

fn palette_by_name(name: &str) -> &'static Palette {
    PALETTES
        .iter()
        .find(|p| p.eq_name(name))
        .unwrap_or_else(|| PALETTES.last().unwrap())
}

impl Palette {
    fn eq_name(&self, other: &str) -> bool {
        self.name.eq_ignore_ascii_case(other)
    }
}

fn argb_to_color(argb: u32) -> D2D1_COLOR_F {
    let a = ((argb >> 24) & 0xFF) as f32 / 255.0;
    let r = ((argb >> 16) & 0xFF) as f32 / 255.0;
    let g = ((argb >> 8) & 0xFF) as f32 / 255.0;
    let b = (argb & 0xFF) as f32 / 255.0;
    D2D1_COLOR_F { r, g, b, a }
}

// ===========================================================================
// Markdown adapter — pulldown-cmark events -> styled text + spans.
//
// Subset rendered as styled: **bold**, *italic*/_italic_, `inline code`,
// fenced ```code blocks```, headings (rendered as bold lines), and `---`
// rules. List items get a `• ` prefix. Everything else (links, images,
// blockquotes, tables, etc.) passes through with its inner text plain.
//
// All offsets are UTF-16 code units because that's the index space
// IDWriteTextLayout uses for SetFontWeight / SetFontStyle / SetFontFamilyName
// and HitTestTextRange.
// ===========================================================================
mod markdown {
    use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Style {
        Bold,
        Italic,
        InlineCode,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Span {
        pub start: u32,
        pub len: u32,
        pub style: Style,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CodeBlock {
        pub start: u32,
        pub len: u32,
    }

    #[derive(Debug, Default, Clone)]
    pub struct StyledText {
        pub text: String,
        pub spans: Vec<Span>,
        pub code_blocks: Vec<CodeBlock>,
    }

    fn utf16_len(s: &str) -> u32 {
        s.encode_utf16().count() as u32
    }

    fn append(out: &mut StyledText, pos: &mut u32, s: &str) {
        out.text.push_str(s);
        *pos += utf16_len(s);
    }

    fn ensure_newline(out: &mut StyledText, pos: &mut u32) {
        if !out.text.is_empty() && !out.text.ends_with('\n') {
            append(out, pos, "\n");
        }
    }

    fn ensure_blank_line(out: &mut StyledText, pos: &mut u32) {
        if out.text.is_empty() {
            return;
        }
        if out.text.ends_with("\n\n") {
            return;
        }
        if out.text.ends_with('\n') {
            append(out, pos, "\n");
        } else {
            append(out, pos, "\n\n");
        }
    }

    pub fn parse(input: &str) -> StyledText {
        let mut out = StyledText::default();
        let mut pos: u32 = 0;
        let mut bold_starts: Vec<u32> = Vec::new();
        let mut italic_starts: Vec<u32> = Vec::new();
        let mut code_block_start: Option<u32> = None;
        let mut in_code_block = false;
        let mut list_depth: u32 = 0;
        let mut just_started_item = false;

        for event in Parser::new_ext(input, Options::empty()) {
            match event {
                Event::Start(Tag::Strong) => bold_starts.push(pos),
                Event::End(TagEnd::Strong) => {
                    if let Some(start) = bold_starts.pop() {
                        if pos > start {
                            out.spans.push(Span {
                                start,
                                len: pos - start,
                                style: Style::Bold,
                            });
                        }
                    }
                }
                Event::Start(Tag::Emphasis) => italic_starts.push(pos),
                Event::End(TagEnd::Emphasis) => {
                    if let Some(start) = italic_starts.pop() {
                        if pos > start {
                            out.spans.push(Span {
                                start,
                                len: pos - start,
                                style: Style::Italic,
                            });
                        }
                    }
                }
                Event::Code(s) => {
                    let start = pos;
                    append(&mut out, &mut pos, &s);
                    if pos > start {
                        out.spans.push(Span {
                            start,
                            len: pos - start,
                            style: Style::InlineCode,
                        });
                    }
                }
                Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_)))
                | Event::Start(Tag::CodeBlock(CodeBlockKind::Indented)) => {
                    ensure_blank_line(&mut out, &mut pos);
                    code_block_start = Some(pos);
                    in_code_block = true;
                }
                Event::End(TagEnd::CodeBlock) => {
                    if let Some(start) = code_block_start.take() {
                        // pulldown-cmark always appends a trailing '\n' to the
                        // last text event in a code block. Trim that from the
                        // span so the bg rect doesn't extend below the text.
                        let mut end = pos;
                        if out.text.ends_with('\n') {
                            end = end.saturating_sub(1);
                        }
                        if end > start {
                            out.code_blocks.push(CodeBlock {
                                start,
                                len: end - start,
                            });
                        }
                    }
                    in_code_block = false;
                }
                Event::Text(s) => {
                    append(&mut out, &mut pos, &s);
                    if just_started_item {
                        just_started_item = false;
                    }
                }
                Event::SoftBreak => {
                    if in_code_block {
                        append(&mut out, &mut pos, "\n");
                    } else {
                        append(&mut out, &mut pos, " ");
                    }
                }
                Event::HardBreak => append(&mut out, &mut pos, "\n"),
                Event::Rule => {
                    ensure_blank_line(&mut out, &mut pos);
                    append(&mut out, &mut pos, "────────");
                    ensure_blank_line(&mut out, &mut pos);
                }
                Event::Start(Tag::Paragraph) => {
                    if just_started_item || list_depth > 0 {
                        // Inside a list item: no paragraph spacing — the `• `
                        // prefix already positioned us. Multi-paragraph items
                        // get a single newline at most.
                        if list_depth > 0 && !out.text.is_empty() && !out.text.ends_with('\n') {
                            append(&mut out, &mut pos, "\n");
                        }
                    } else {
                        ensure_blank_line(&mut out, &mut pos);
                    }
                }
                Event::End(TagEnd::Paragraph) => {}
                Event::Start(Tag::Heading { .. }) => {
                    ensure_blank_line(&mut out, &mut pos);
                    bold_starts.push(pos);
                }
                Event::End(TagEnd::Heading(_)) => {
                    if let Some(start) = bold_starts.pop() {
                        if pos > start {
                            out.spans.push(Span {
                                start,
                                len: pos - start,
                                style: Style::Bold,
                            });
                        }
                    }
                }
                Event::Start(Tag::List(_)) => {
                    list_depth = list_depth.saturating_add(1);
                    ensure_newline(&mut out, &mut pos);
                }
                Event::End(TagEnd::List(_)) => {
                    list_depth = list_depth.saturating_sub(1);
                }
                Event::Start(Tag::Item) => {
                    ensure_newline(&mut out, &mut pos);
                    append(&mut out, &mut pos, "• ");
                    just_started_item = true;
                }
                Event::End(TagEnd::Item) => {}
                // Drop raw HTML — Claude rarely emits it and rendering it as
                // plain text would expose tags to the user.
                Event::Html(_) | Event::InlineHtml(_) => {}
                // Everything else (Link, Image, BlockQuote, Table, etc.):
                // no-op on the tag itself; the inner Text events still flow
                // through and get rendered plain.
                _ => {}
            }
        }

        out
    }
}

// ===========================================================================
// CLI args
// ===========================================================================
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OptionMode {
    Single,
    Multi,
    Preview,
    Approve,
}

impl Default for OptionMode {
    fn default() -> Self {
        OptionMode::Single
    }
}

fn parse_option_mode(s: &str) -> OptionMode {
    match s.to_ascii_lowercase().as_str() {
        "multi" => OptionMode::Multi,
        "preview" => OptionMode::Preview,
        "approve" => OptionMode::Approve,
        _ => OptionMode::Single,
    }
}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
struct Args {
    event: String,
    title: String,
    message: String,
    options: Vec<String>,
    #[serde(default)]
    previews: Vec<String>,
    #[serde(default)]
    mode: String, // "single"|"multi"|"preview"|"approve"
    #[serde(default)]
    project: String,
    #[serde(default)]
    session: String, // short hash
    #[serde(default)]
    placeholder: String,
    #[serde(default)]
    palette: String, // optional override
}

fn split_pipe(s: &str) -> Vec<String> {
    s.split('|')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

fn split_comma(s: &str) -> Vec<String> {
    s.split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

fn parse_args() -> Args {
    let mut a = Args::default();
    a.event = "Stop".into();
    a.title = "Agent needs you".into();
    let mut it = env::args().skip(1);
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--event" => a.event = it.next().unwrap_or_default(),
            "--title" => a.title = it.next().unwrap_or_default(),
            "--message" => a.message = it.next().unwrap_or_default(),
            "--options" => a.options = split_comma(&it.next().unwrap_or_default()),
            "--previews" => a.previews = split_pipe(&it.next().unwrap_or_default()),
            "--mode" => a.mode = it.next().unwrap_or_default(),
            "--project" => a.project = it.next().unwrap_or_default(),
            "--session" => a.session = it.next().unwrap_or_default(),
            "--placeholder" => a.placeholder = it.next().unwrap_or_default(),
            "--palette" => a.palette = it.next().unwrap_or_default(),
            _ => {}
        }
    }
    a
}

// ===========================================================================
// Hook mode
// ===========================================================================
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HookEvent {
    Stop,
    Question,
    Gate,
    Notification,
    /// Claude Code's `PermissionRequest` event — fires only when auto-mode
    /// can't decide and the user would otherwise see the built-in permission
    /// prompt. Same UX as Gate (Allow / Deny / free text), but the output
    /// schema differs (`hookSpecificOutput.decision.behavior` instead of
    /// `permissionDecision`, no reason field) — see `build_decision_json`.
    Permission,
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
                "Notification" => HookEvent::Notification,
                "Permission" | "PermissionRequest" => HookEvent::Permission,
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

/// Read the transcript file (JSONL) and return the most recent assistant
/// text block. Used as a fallback when `last_assistant_message` is missing.
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

fn project_from_cwd(cwd: &str) -> String {
    let p = Path::new(cwd);
    p.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(cwd)
        .to_string()
}

fn derive_args(event: HookEvent, payload: &serde_json::Value) -> Args {
    let mut a = Args::default();
    a.event = match event {
        HookEvent::Stop => "Stop".into(),
        HookEvent::Question => "Question".into(),
        HookEvent::Gate => "Gate".into(),
        HookEvent::Notification => "Notification".into(),
        HookEvent::Permission => "Permission".into(),
    };

    if let Some(cwd) = payload.get("cwd").and_then(|v| v.as_str()) {
        a.project = project_from_cwd(cwd);
    }
    if let Some(sid) = payload.get("session_id").and_then(|v| v.as_str()) {
        // Short hash for the chip — first 7 hex-ish chars.
        a.session = sid.chars().take(7).collect();
    }

    match event {
        HookEvent::Stop => {
            a.title = "Agent finished".into();
            a.message =
                "Claude finished this turn. Type a reply to keep going, or double-Esc to let it stop."
                    .into();
            a.placeholder = "Reply to continue, or double-Esc to let Claude stop.".into();
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
            a.placeholder = "Type a custom answer…".into();
            let tool_name = payload.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
            if tool_name == "ExitPlanMode" {
                a.title = "Plan ready".into();
                a.message = "Approve the plan, or type changes.".into();
                if let Some(plan) = payload
                    .pointer("/tool_input/plan")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                {
                    a.message = plan.to_string();
                }
                a.options = vec!["Approve".into()];
                a.mode = "approve".into();
                a.placeholder = "Or describe changes to the plan…".into();
            } else if let Some(questions) = payload
                .pointer("/tool_input/questions")
                .and_then(|v| v.as_array())
            {
                if let Some(q) = questions.first() {
                    if let Some(qtext) = q.get("question").and_then(|v| v.as_str()) {
                        a.message = qtext.to_string();
                    }
                    let multi_select = q
                        .get("multiSelect")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let mut any_preview = false;
                    if let Some(opts) = q.get("options").and_then(|v| v.as_array()) {
                        for o in opts {
                            let label = o
                                .get("label")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let preview = o
                                .get("preview")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            if !preview.is_empty() {
                                any_preview = true;
                            }
                            a.options.push(label);
                            a.previews.push(preview);
                        }
                    }
                    a.mode = if any_preview {
                        "preview".into()
                    } else if multi_select {
                        "multi".into()
                    } else {
                        "single".into()
                    };
                }
            }
        }
        HookEvent::Gate | HookEvent::Permission => {
            a.title = "Permission needed".into();
            // For Gate (PreToolUse), free-text becomes the deny reason that
            // Claude reads back. For Permission (PermissionRequest), the
            // output schema has no reason field — typed text is dropped, only
            // the Allow/Deny decision survives. Same popup either way; the
            // difference is in build_decision_json.
            a.placeholder = "Type a reason…".into();
            if let Some(cmd) = payload.pointer("/tool_input/command").and_then(|v| v.as_str()) {
                a.message = format!("Run: {}", cmd);
            } else if let Some(tn) = payload.get("tool_name").and_then(|v| v.as_str()) {
                a.message = format!("Allow {}?", tn);
            } else {
                a.message = "Claude wants to run a tool.".into();
            }
            a.options = vec!["Allow".into(), "Deny".into()];
        }
        HookEvent::Notification => {
            // Claude Code fires Notification when it needs the user's attention
            // — most commonly a permission prompt that auto-mode can't decide,
            // or an idle-waiting alert (~60s). The payload carries a single
            // `message` string. Our popup surfaces it; we don't influence the
            // notification itself (always exit 0 with no decision JSON).
            a.title = "Claude needs attention".into();
            a.placeholder = "Press Esc Esc or Dismiss to acknowledge.".into();
            a.message = payload
                .get("message")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("Claude is waiting for you.")
                .to_string();
        }
    }
    a
}

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
            "previews": derived.previews,
            "mode": derived.mode,
            "project": derived.project,
            "session": derived.session,
        },
        "transcript_last_5_lines": transcript_sample,
        "timestamp_unix_ms": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0),
    });
    fs::write(&path, serde_json::to_string_pretty(&dump).unwrap_or_default())
}

fn should_skip_gate(payload: &serde_json::Value) -> bool {
    match payload.get("permission_mode").and_then(|v| v.as_str()) {
        Some("default") | None => false,
        _ => true,
    }
}

fn build_decision_json(event: HookEvent, outcome: &Outcome) -> Option<serde_json::Value> {
    // Notification is informational — the popup is just a richer surface for
    // Claude Code's "needs attention" signal. We never alter the underlying
    // notification (no JSON output, exit 0), regardless of whether the user
    // dismissed or typed something.
    if matches!(event, HookEvent::Notification) {
        return None;
    }
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
        HookEvent::Permission => {
            // PermissionRequest output schema (distinct from PreToolUse):
            //   hookSpecificOutput.decision is an OBJECT with `behavior`,
            //   not a string. There is no reason field — free-text answers
            //   collapse to `deny` and the text is dropped (Claude Code
            //   doesn't carry it through).
            let behavior = if answer == "Allow" { "allow" } else { "deny" };
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": { "behavior": behavior }
                }
            })
        }
        HookEvent::Notification => unreachable!("handled by early return above"),
    })
}

// ===========================================================================
// Persistent state (position + per-project palette mapping)
// ===========================================================================
fn true_default() -> bool {
    true
}
fn is_true(b: &bool) -> bool {
    *b
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistentState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    x: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    y: Option<i32>,
    #[serde(default)]
    palettes: HashMap<String, String>,
    /// D1 / R9 — when false, hook-mode invocations exit 0 silently with no
    /// UI. CLI-mode invocations ignore this flag (you're explicitly asking
    /// for a window).
    #[serde(default = "true_default", skip_serializing_if = "is_true")]
    enabled: bool,
}

impl Default for PersistentState {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            palettes: HashMap::new(),
            enabled: true,
        }
    }
}

fn app_data_dir() -> PathBuf {
    let base = env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join("floating-prompt")
}

fn state_path() -> PathBuf {
    app_data_dir().join("state.json")
}

fn load_state_from(path: &Path) -> PersistentState {
    let raw = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return PersistentState::default(),
    };
    serde_json::from_str::<PersistentState>(&raw).unwrap_or_default()
}

fn save_state_to(path: &Path, state: &PersistentState) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(state).unwrap_or_else(|_| "{}".into());
    fs::write(path, json)
}

/// Back-compat: position-only round trip on top of the richer schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Position {
    x: i32,
    y: i32,
}

fn load_position_from(path: &Path) -> Option<Position> {
    let st = load_state_from(path);
    match (st.x, st.y) {
        (Some(x), Some(y)) => Some(Position { x, y }),
        _ => None,
    }
}

fn save_position_to(path: &Path, pos: Position) -> std::io::Result<()> {
    let mut st = load_state_from(path);
    st.x = Some(pos.x);
    st.y = Some(pos.y);
    save_state_to(path, &st)
}

fn resolve_palette(args: &Args, state: &PersistentState) -> &'static Palette {
    if !args.palette.is_empty() {
        return palette_by_name(&args.palette);
    }
    if !args.project.is_empty() {
        if let Some(name) = state.palettes.get(&args.project) {
            return palette_by_name(name);
        }
    }
    palette_by_name("default")
}

fn clamp_to_work(x: i32, y: i32, work: (i32, i32, i32, i32), w: i32, h: i32) -> (i32, i32) {
    let (l, t, r, b) = work;
    let max_x = (r - w).max(l);
    let max_y = (b - h).max(t);
    (x.clamp(l, max_x), y.clamp(t, max_y))
}

// ===========================================================================
// Queue management
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

/// Returns the queue badge string per the new design — bare number when
/// total ≥ 2 (e.g. "3"), empty otherwise. R7 supersedes the old "X of Y".
fn format_counter(_pos: usize, total: usize) -> String {
    if total >= 2 {
        format!("{}", total)
    } else {
        String::new()
    }
}

// ===========================================================================
// Outcome + dismiss state machine
// ===========================================================================
enum Outcome {
    Answered(String),
    Dismissed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DismissPhase {
    Rest,
    Armed,
    Done,
    Timeout,
}

#[derive(Debug, Clone, Copy)]
struct DismissState {
    phase: DismissPhase,
    /// When the phase started (used for Armed → Timeout and Timeout → Rest
    /// time-based transitions, and for the drain animation).
    since: Instant,
}

impl Default for DismissState {
    fn default() -> Self {
        Self { phase: DismissPhase::Rest, since: Instant::now() }
    }
}

const DISMISS_ARM_MS: u128 = 600;
const DISMISS_TIMEOUT_HOLD_MS: u128 = 250;

/// Advance the dismiss state machine given pending Esc events + the wall
/// clock. Returns (new_state, should_dismiss).
///
/// Pure: easy to unit-test.
fn advance_dismiss(
    cur: DismissState,
    single_esc: bool,
    double_esc: bool,
    now: Instant,
) -> (DismissState, bool) {
    // Time-based transitions first (so a single Esc arriving the moment we
    // would have timed out is treated as a fresh arm).
    let mut s = cur;
    match s.phase {
        DismissPhase::Armed => {
            if now.duration_since(s.since).as_millis() >= DISMISS_ARM_MS {
                s = DismissState { phase: DismissPhase::Timeout, since: now };
            }
        }
        DismissPhase::Timeout => {
            if now.duration_since(s.since).as_millis() >= DISMISS_TIMEOUT_HOLD_MS {
                s = DismissState { phase: DismissPhase::Rest, since: now };
            }
        }
        DismissPhase::Rest | DismissPhase::Done => {}
    }

    // Event-based transitions.
    if double_esc && s.phase == DismissPhase::Armed {
        return (DismissState { phase: DismissPhase::Done, since: now }, true);
    }
    if single_esc {
        match s.phase {
            DismissPhase::Rest | DismissPhase::Timeout => {
                s = DismissState { phase: DismissPhase::Armed, since: now };
            }
            DismissPhase::Armed => {
                // Treat a second single-Esc within the window as a "Done"
                // too — defends against missing the DOUBLE_ESC flag race.
                if now.duration_since(s.since).as_millis() < DISMISS_ARM_MS {
                    return (DismissState { phase: DismissPhase::Done, since: now }, true);
                } else {
                    s = DismissState { phase: DismissPhase::Armed, since: now };
                }
            }
            DismissPhase::Done => {}
        }
    }

    (s, false)
}

/// Progress 0..1 for the draining bar under the Esc pips while armed.
fn dismiss_progress(s: DismissState, now: Instant) -> f32 {
    match s.phase {
        DismissPhase::Armed => {
            let elapsed = now.duration_since(s.since).as_millis() as f32;
            (1.0 - (elapsed / DISMISS_ARM_MS as f32)).clamp(0.0, 1.0)
        }
        DismissPhase::Timeout => 0.04,
        _ => 0.0,
    }
}

// ===========================================================================
// Global double-Esc watcher
// ===========================================================================
static DOUBLE_ESC: AtomicBool = AtomicBool::new(false);
static SINGLE_ESC: AtomicBool = AtomicBool::new(false);
thread_local! {
    static LAST_ESC: RefCell<Option<Instant>> = RefCell::new(None);
}
static mut KB_HOOK: HHOOK = HHOOK(0);

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && wparam.0 as u32 == WM_KEYDOWN {
        let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        if kb.vkCode == VK_ESCAPE.0 as u32 {
            SINGLE_ESC.store(true, Ordering::SeqCst);
            LAST_ESC.with(|cell| {
                let now = Instant::now();
                let mut last = cell.borrow_mut();
                if let Some(prev) = *last {
                    if now.duration_since(prev).as_millis() <= DISMISS_ARM_MS {
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

fn reset_esc_flags() {
    DOUBLE_ESC.store(false, Ordering::SeqCst);
    SINGLE_ESC.store(false, Ordering::SeqCst);
    LAST_ESC.with(|c| *c.borrow_mut() = None);
}

// ===========================================================================
// Layout
// ===========================================================================
const ID_ESC_TIMER: usize = 1;
const ID_POLL_TIMER: usize = 2;
const ID_ANIM_TIMER: usize = 3;
const ID_CARET_TIMER: usize = 4;
const CARET_BLINK_MS: u32 = 530;
const CARET_SOLID_AFTER_MOVE_MS: u128 = 250;
const INPUT_TEXT_PAD_X: f32 = 10.0;
const INPUT_TEXT_PAD_Y: f32 = 8.0;

const POPUP_MIN_W: i32 = 480;
const POPUP_MAX_W: i32 = 640;
const POPUP_DEFAULT_W: i32 = 520;
const POPUP_MIN_H: i32 = 220;
const POPUP_MAX_H: i32 = 760;
const POPUP_RADIUS: f32 = 14.0;

const PAD: i32 = 14;
const TOP_ROW_H: i32 = 24;
const GAP: i32 = 12;

const PANEL_RADIUS: f32 = 10.0;
const PANEL_PAD_X: i32 = 16;
const PANEL_PAD_Y: i32 = 14;
const PANEL_MAX_H: i32 = 360;
const PANEL_SCROLL_W: i32 = 8;
const PANEL_SCROLL_INSET: i32 = 4;

const OPT_RADIUS: f32 = 8.0;
const OPT_PAD_X: i32 = 14;
const OPT_PAD_Y: i32 = 10;
const OPT_GAP: i32 = 6;
const OPT_NUM_W: i32 = 18;
const OPT_CHECK_SIZE: i32 = 16;
const OPT_INNER_GAP: i32 = 12;
const OPT_MIN_H: i32 = 38;

// Multi-line input: tall enough for ~3 lines of body text + the EDIT's own
// internal text inset. Shift+Enter inserts a newline; Enter alone submits.
const INPUT_H: i32 = 84;
const INPUT_RADIUS: f32 = 8.0;
const FOOTER_H: i32 = 28;

const CHIP_H: i32 = 24;
const CHIP_PAD_X: i32 = 8;
const CHIP_DOT: i32 = 8;
const CHIP_GAP: i32 = 8;

const QUEUE_BADGE_H: i32 = 22;
const QUEUE_BADGE_MIN_W: i32 = 22;
const QUEUE_BADGE_PAD: i32 = 7;

const GRIP_DOT: i32 = 3;
const GRIP_GAP: i32 = 4;

const PIP_W: i32 = 32;
const PIP_H: i32 = 18;
const PIP_GAP: i32 = 3;
const DISMISS_PAD: i32 = 6;
const DISMISS_LABEL_GAP: i32 = 8;

const PREVIEW_GAP: i32 = 10;

const POLL_HIDDEN_MS: u32 = 150;
const POLL_SHOWN_MS: u32 = 500;
const ESC_TICK_MS: u32 = 80;
const ANIM_TICK_MS: u32 = 30;

#[derive(Clone, Copy, Default, Debug)]
struct Rectf {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

impl Rectf {
    fn new(l: i32, t: i32, r: i32, b: i32) -> Self {
        Self { left: l as f32, top: t as f32, right: r as f32, bottom: b as f32 }
    }
    fn w(&self) -> f32 {
        self.right - self.left
    }
    fn h(&self) -> f32 {
        self.bottom - self.top
    }
    fn contains(&self, x: i32, y: i32) -> bool {
        let x = x as f32;
        let y = y as f32;
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
    fn to_d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F {
            left: self.left,
            top: self.top,
            right: self.right,
            bottom: self.bottom,
        }
    }
}

#[derive(Clone, Default, Debug)]
struct Layout {
    top: Rectf,
    chip: Rectf,
    chip_dot: Rectf,
    chip_name: Rectf,
    chip_sep: Rectf,
    chip_hash: Rectf,
    grip: Rectf,
    queue_badge: Rectf,
    message_panel: Rectf,
    message_text: Rectf,
    message_scroll_track: Rectf,
    options: Vec<Rectf>,
    options_block: Rectf,
    preview_panel: Rectf,
    input: Rectf,
    footer: Rectf,
    enter_legend: Rectf,
    dismiss: Rectf,
    dismiss_pip1: Rectf,
    dismiss_pip2: Rectf,
    dismiss_progress: Rectf,
    dismiss_label: Rectf,
    /// Hit zone for the draggable top row.
    drag_zone_bottom_y: i32,
    /// Total height the message text needs at its width — for scroll bounds.
    message_total_h: f32,
}

// ===========================================================================
// Helpers
// ===========================================================================
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_no_nul(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

unsafe fn state_ptr(hwnd: HWND) -> Option<*const WindowState> {
    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    if p == 0 {
        None
    } else {
        Some(p as *const WindowState)
    }
}

// ===========================================================================
// Custom text input — buffer + cursor + selection + word-jump + clipboard
// ===========================================================================

/// Sorted (start, end) of the selection if one is active.
fn input_selection_range(state: &WindowState) -> Option<(usize, usize)> {
    let c = state.input_cursor.get();
    let a = state.input_anchor.get()?;
    if a == c {
        return None;
    }
    Some(if a < c { (a, c) } else { (c, a) })
}

/// Replace the selection (or insert at cursor) with `s`. Marks the caret
/// solid-on so the user can see where it landed.
fn input_insert(state: &WindowState, s: &str) {
    let (start, end) = match input_selection_range(state) {
        Some(r) => r,
        None => {
            let c = state.input_cursor.get();
            (c, c)
        }
    };
    let mut text = state.input_text.borrow_mut();
    text.replace_range(start..end, s);
    state.input_cursor.set(start + s.len());
    state.input_anchor.set(None);
    state.caret_moved_at.set(Instant::now());
    state.caret_on.set(true);
}

/// Remove the selection (no-op if none). Returns true if anything changed.
fn input_delete_selection(state: &WindowState) -> bool {
    if let Some((s, e)) = input_selection_range(state) {
        state.input_text.borrow_mut().replace_range(s..e, "");
        state.input_cursor.set(s);
        state.input_anchor.set(None);
        state.caret_moved_at.set(Instant::now());
        state.caret_on.set(true);
        return true;
    }
    false
}

/// Walk byte index `from` forward (or backward) in `text` to the next word
/// boundary. Word = run of non-whitespace; we step over a run of whitespace
/// and the run of word chars after it (forward), or before it (backward).
fn step_word(text: &str, from: usize, forward: bool) -> usize {
    let bytes = text.as_bytes();
    let is_ws = |b: u8| b == b' ' || b == b'\t' || b == b'\n' || b == b'\r';
    if forward {
        let mut i = from;
        // Skip current run of non-whitespace
        while i < bytes.len() && !is_ws(bytes[i]) {
            i = next_char_boundary(text, i);
        }
        // Skip whitespace
        while i < bytes.len() && is_ws(bytes[i]) {
            i = next_char_boundary(text, i);
        }
        i
    } else {
        let mut i = from;
        // Walk back over whitespace
        while i > 0 {
            let p = prev_char_boundary(text, i);
            if is_ws(bytes[p]) {
                i = p;
            } else {
                break;
            }
        }
        // Walk back over non-whitespace
        while i > 0 {
            let p = prev_char_boundary(text, i);
            if is_ws(bytes[p]) {
                break;
            }
            i = p;
        }
        i
    }
}

fn next_char_boundary(text: &str, from: usize) -> usize {
    let bytes = text.as_bytes();
    let mut i = from + 1;
    while i < bytes.len() && (bytes[i] & 0xC0) == 0x80 {
        i += 1;
    }
    i.min(bytes.len())
}

fn prev_char_boundary(text: &str, from: usize) -> usize {
    if from == 0 {
        return 0;
    }
    let bytes = text.as_bytes();
    let mut i = from - 1;
    while i > 0 && (bytes[i] & 0xC0) == 0x80 {
        i -= 1;
    }
    i
}

/// Build a DWrite text layout for the input box at its visible width.
unsafe fn build_input_layout(
    state: &WindowState,
    text: &str,
    width: f32,
    height: f32,
) -> Option<IDWriteTextLayout> {
    let fmt = state.fmt_option.borrow();
    let fmt = fmt.as_ref()?;
    make_text_layout(text, fmt, width, height)
}

/// Set the system clipboard to `text`. Best-effort — silently no-ops on
/// failure (e.g. another app holds the clipboard).
unsafe fn clipboard_set(hwnd: HWND, text: &str) {
    if OpenClipboard(hwnd).is_err() {
        return;
    }
    let _ = EmptyClipboard();
    let utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = utf16.len() * 2;
    let hmem = match GlobalAlloc(GMEM_MOVEABLE, bytes) {
        Ok(h) => h,
        Err(_) => {
            let _ = CloseClipboard();
            return;
        }
    };
    let dst = GlobalLock(hmem) as *mut u16;
    if !dst.is_null() {
        std::ptr::copy_nonoverlapping(utf16.as_ptr(), dst, utf16.len());
        let _ = GlobalUnlock(hmem);
        let _ = SetClipboardData(CF_UNICODETEXT.0 as u32, HANDLE(hmem.0 as isize));
    }
    let _ = CloseClipboard();
}

/// Read the system clipboard as UTF-16 text.
unsafe fn clipboard_get(hwnd: HWND) -> Option<String> {
    if OpenClipboard(hwnd).is_err() {
        return None;
    }
    let h = match GetClipboardData(CF_UNICODETEXT.0 as u32) {
        Ok(h) => h,
        Err(_) => {
            let _ = CloseClipboard();
            return None;
        }
    };
    let hg = HGLOBAL(h.0 as *mut core::ffi::c_void);
    let ptr = GlobalLock(hg) as *const u16;
    if ptr.is_null() {
        let _ = CloseClipboard();
        return None;
    }
    let mut len = 0usize;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    let s = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
    let _ = GlobalUnlock(hg);
    let _ = CloseClipboard();
    Some(s.replace("\r\n", "\n").replace('\r', "\n"))
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

// ===========================================================================
// D2D / DirectWrite factories
// ===========================================================================
struct UnsafeWrap<T>(T);
unsafe impl<T> Send for UnsafeWrap<T> {}
unsafe impl<T> Sync for UnsafeWrap<T> {}

static D2D_FACTORY: OnceLock<UnsafeWrap<ID2D1Factory>> = OnceLock::new();
static DWRITE_FACTORY: OnceLock<UnsafeWrap<IDWriteFactory>> = OnceLock::new();

fn d2d_factory() -> &'static ID2D1Factory {
    let w = D2D_FACTORY.get_or_init(|| unsafe {
        let f: ID2D1Factory =
            D2D1CreateFactory::<ID2D1Factory>(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)
                .expect("D2D1CreateFactory");
        UnsafeWrap(f)
    });
    &w.0
}

fn dwrite_factory() -> &'static IDWriteFactory {
    let w = DWRITE_FACTORY.get_or_init(|| unsafe {
        let f: IDWriteFactory =
            DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).expect("DWriteCreateFactory");
        UnsafeWrap(f)
    });
    &w.0
}

unsafe fn create_text_format(
    family: &str,
    weight: DWRITE_FONT_WEIGHT,
    size_px: f32,
) -> IDWriteTextFormat {
    let family_w = wide(family);
    let locale_w = wide("en-us");
    let f = dwrite_factory()
        .CreateTextFormat(
            PCWSTR(family_w.as_ptr()),
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size_px,
            PCWSTR(locale_w.as_ptr()),
        )
        .expect("CreateTextFormat");
    f
}

unsafe fn make_text_layout(
    text: &str,
    format: &IDWriteTextFormat,
    max_w: f32,
    max_h: f32,
) -> Option<IDWriteTextLayout> {
    let w = wide_no_nul(text);
    let len = w.len() as u32;
    dwrite_factory()
        .CreateTextLayout(&w, format, max_w.max(1.0), max_h.max(1.0))
        .ok()
        .map(|layout| {
            let _ = len; // suppress unused
            layout
        })
}

unsafe fn measure_layout_height(layout: &IDWriteTextLayout) -> f32 {
    let mut m = DWRITE_TEXT_METRICS::default();
    let _ = layout.GetMetrics(&mut m);
    m.height
}

unsafe fn measure_layout_width(layout: &IDWriteTextLayout) -> f32 {
    let mut m = DWRITE_TEXT_METRICS::default();
    let _ = layout.GetMetrics(&mut m);
    m.widthIncludingTrailingWhitespace.max(m.width)
}

/// Build a text layout from already-parsed markdown and apply per-range
/// font weight / style / family so bold / italic / code render correctly.
/// The caller still owns the `StyledText` and uses its `code_blocks` / inline
/// `spans` to paint backgrounds (offsets are UTF-16, matching DirectWrite).
///
/// Mono ranges keep `format`'s point size — Cascadia Mono at the body size
/// has slightly different metrics than Segoe UI, so the line containing code
/// may be a couple px taller. We accept that — it's better than introducing a
/// font-size mismatch that disrupts baselines.
unsafe fn make_styled_layout(
    parsed: &markdown::StyledText,
    format: &IDWriteTextFormat,
    max_w: f32,
    max_h: f32,
) -> Option<IDWriteTextLayout> {
    let layout = make_text_layout(&parsed.text, format, max_w, max_h)?;
    apply_markdown_styles(&layout, parsed);
    Some(layout)
}

unsafe fn apply_markdown_styles(layout: &IDWriteTextLayout, parsed: &markdown::StyledText) {
    let mono_family = wide("Cascadia Mono");
    let mono_pcwstr = PCWSTR(mono_family.as_ptr());

    for span in &parsed.spans {
        let range = DWRITE_TEXT_RANGE {
            startPosition: span.start,
            length: span.len,
        };
        match span.style {
            markdown::Style::Bold => {
                let _ = layout.SetFontWeight(DWRITE_FONT_WEIGHT_BOLD, range);
            }
            markdown::Style::Italic => {
                let _ = layout.SetFontStyle(DWRITE_FONT_STYLE_ITALIC, range);
            }
            markdown::Style::InlineCode => {
                let _ = layout.SetFontFamilyName(mono_pcwstr, range);
            }
        }
    }

    for cb in &parsed.code_blocks {
        let range = DWRITE_TEXT_RANGE {
            startPosition: cb.start,
            length: cb.len,
        };
        let _ = layout.SetFontFamilyName(mono_pcwstr, range);
    }
}

/// Paint a tinted background behind a UTF-16 range of glyphs in `layout`.
/// Two flavors: inline (snug to glyph extent, `full_width_*` = None) and
/// block (each line extended to `[full_width_left, full_width_right]`,
/// pass Some). `h_pad` / `v_pad` widen the rect on each side after width
/// computation — inline code gets a couple px horizontal breathing room.
#[allow(clippy::too_many_arguments)]
unsafe fn paint_text_range_bg(
    rt: &ID2D1RenderTarget,
    layout: &IDWriteTextLayout,
    brush: &ID2D1SolidColorBrush,
    start: u32,
    length: u32,
    origin_x: f32,
    origin_y: f32,
    full_width_left: Option<f32>,
    full_width_right: Option<f32>,
    h_pad: f32,
    v_pad: f32,
) {
    if length == 0 {
        return;
    }
    // Two-call pattern: probe for required count, then allocate + fetch.
    let mut count: u32 = 0;
    let _ = layout.HitTestTextRange(start, length, origin_x, origin_y, None, &mut count);
    if count == 0 {
        return;
    }
    let mut metrics = vec![DWRITE_HIT_TEST_METRICS::default(); count as usize];
    let mut actual: u32 = 0;
    let _ = layout.HitTestTextRange(
        start,
        length,
        origin_x,
        origin_y,
        Some(&mut metrics),
        &mut actual,
    );
    for m in &metrics[..actual as usize] {
        let (left, right) = match (full_width_left, full_width_right) {
            (Some(l), Some(r)) => (l, r),
            _ => (m.left - h_pad, m.left + m.width + h_pad),
        };
        let r = D2D_RECT_F {
            left,
            top: m.top - v_pad,
            right,
            bottom: m.top + m.height + v_pad,
        };
        rt.FillRectangle(&r, brush);
    }
}

// ===========================================================================
// WindowState
// ===========================================================================
struct WindowState {
    args: Args,
    palette: &'static Palette,
    mode: OptionMode,

    /// Markdown-parsed message body, computed once from `args.message`. Used
    /// for both measurement (compute_window_size) and paint. Recomputing per
    /// WM_PAINT would be cheap, but the message never changes after launch.
    parsed_message: markdown::StyledText,

    req_path: PathBuf,
    outcome: RefCell<Option<Outcome>>,
    is_head_shown: Cell<bool>,
    last_counter: RefCell<String>,

    // -------- Custom text input (replaces the Win32 EDIT child) -----------
    // The EDIT child caused permanent flicker because D2D's HwndRenderTarget
    // back buffer wipes the child's pixels every Present(). Painting the
    // input ourselves means D2D owns every pixel of the popup.
    input_text: RefCell<String>,
    /// Byte index into input_text (always at a char boundary).
    input_cursor: Cell<usize>,
    /// The other end of an active selection. None = no selection.
    input_anchor: Cell<Option<usize>>,
    /// Logical focus (the popup itself owns OS focus; this is just whether
    /// caret + keystrokes apply to the input vs. eg. a focused option).
    input_focused: Cell<bool>,
    /// Caret blink toggle.
    caret_on: Cell<bool>,
    /// When the cursor last moved — caret is solid-on for the first ~250ms
    /// after every cursor move so the user can see where it landed.
    caret_moved_at: Cell<Instant>,
    /// True while the user is mouse-dragging a selection.
    input_selecting: Cell<bool>,

    drag_zone_bottom_y: Cell<i32>,
    window_size: Cell<(i32, i32)>,

    selected: RefCell<Vec<bool>>,
    hover_opt: Cell<isize>,
    focus_opt: Cell<isize>,
    hover_dismiss: Cell<bool>,
    hover_top_row: Cell<bool>,
    scroll_y: Cell<f32>,

    dismiss: Cell<DismissState>,
    anim_active: Cell<bool>,

    // D2D resources (rebuilt on demand). Stored as the base interface so all
    // draw methods (Clear, FillRoundedRectangle, …) are callable directly;
    // we only need the Hwnd-specific CreateHwndRenderTarget at construction.
    render_target: RefCell<Option<ID2D1RenderTarget>>,
    fmt_title: RefCell<Option<IDWriteTextFormat>>,
    fmt_body: RefCell<Option<IDWriteTextFormat>>,
    fmt_mono: RefCell<Option<IDWriteTextFormat>>,
    fmt_legend: RefCell<Option<IDWriteTextFormat>>,
    fmt_pip: RefCell<Option<IDWriteTextFormat>>,
    fmt_option: RefCell<Option<IDWriteTextFormat>>,
    fmt_optnum: RefCell<Option<IDWriteTextFormat>>,
    fmt_chip: RefCell<Option<IDWriteTextFormat>>,
    fmt_badge: RefCell<Option<IDWriteTextFormat>>,
    fmt_preview: RefCell<Option<IDWriteTextFormat>>,

    // Cached layout from most recent paint — used for hit testing.
    cached_layout: RefCell<Option<Layout>>,
}

impl WindowState {
    fn ensure_text_formats(&self) {
        unsafe {
            if self.fmt_title.borrow().is_none() {
                *self.fmt_title.borrow_mut() = Some(create_text_format(
                    "Segoe UI Semibold",
                    DWRITE_FONT_WEIGHT_SEMI_BOLD,
                    13.0,
                ));
            }
            if self.fmt_body.borrow().is_none() {
                let f = create_text_format("Segoe UI", DWRITE_FONT_WEIGHT_REGULAR, 14.5);
                let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP);
                *self.fmt_body.borrow_mut() = Some(f);
            }
            if self.fmt_mono.borrow().is_none() {
                *self.fmt_mono.borrow_mut() = Some(create_text_format(
                    "Cascadia Mono",
                    DWRITE_FONT_WEIGHT_REGULAR,
                    10.5,
                ));
            }
            if self.fmt_legend.borrow().is_none() {
                *self.fmt_legend.borrow_mut() = Some(create_text_format(
                    "Segoe UI",
                    DWRITE_FONT_WEIGHT_REGULAR,
                    11.5,
                ));
            }
            if self.fmt_pip.borrow().is_none() {
                let f = create_text_format(
                    "Cascadia Mono",
                    DWRITE_FONT_WEIGHT_REGULAR,
                    9.5,
                );
                let _ = f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
                let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
                *self.fmt_pip.borrow_mut() = Some(f);
            }
            if self.fmt_option.borrow().is_none() {
                let f = create_text_format("Segoe UI", DWRITE_FONT_WEIGHT_REGULAR, 13.5);
                let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP);
                *self.fmt_option.borrow_mut() = Some(f);
            }
            if self.fmt_optnum.borrow().is_none() {
                *self.fmt_optnum.borrow_mut() = Some(create_text_format(
                    "Cascadia Mono",
                    DWRITE_FONT_WEIGHT_REGULAR,
                    11.5,
                ));
            }
            if self.fmt_chip.borrow().is_none() {
                *self.fmt_chip.borrow_mut() = Some(create_text_format(
                    "Segoe UI",
                    DWRITE_FONT_WEIGHT_MEDIUM,
                    12.0,
                ));
            }
            if self.fmt_badge.borrow().is_none() {
                let f = create_text_format("Cascadia Mono", DWRITE_FONT_WEIGHT_MEDIUM, 12.0);
                let _ = f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
                let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
                *self.fmt_badge.borrow_mut() = Some(f);
            }
            if self.fmt_preview.borrow().is_none() {
                let f = create_text_format("Cascadia Mono", DWRITE_FONT_WEIGHT_REGULAR, 11.0);
                let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
                *self.fmt_preview.borrow_mut() = Some(f);
            }
        }
    }

    fn ensure_render_target(&self, hwnd: HWND) {
        if self.render_target.borrow().is_some() {
            return;
        }
        unsafe {
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            let size = D2D_SIZE_U {
                width: (rc.right - rc.left).max(1) as u32,
                height: (rc.bottom - rc.top).max(1) as u32,
            };
            let rt_props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 0.0,
                dpiY: 0.0,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd,
                pixelSize: size,
                presentOptions: D2D1_PRESENT_OPTIONS_NONE,
            };
            if let Ok(hwnd_rt) = d2d_factory().CreateHwndRenderTarget(&rt_props, &hwnd_props) {
                // Cast to the base render-target interface so all draw
                // methods are callable directly without auto-deref games.
                if let Ok(rt) = hwnd_rt.cast::<ID2D1RenderTarget>() {
                    rt.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                    rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE);
                    *self.render_target.borrow_mut() = Some(rt);
                }
            }
        }
    }

    fn drop_d2d(&self) {
        self.render_target.borrow_mut().take();
    }
}

// ===========================================================================
// Layout: compute all rects from the window size and the current state.
// ===========================================================================
fn layout_window(client_w: i32, client_h: i32, state: &WindowState) -> Layout {
    let mut lay = Layout::default();

    let content_w = client_w - 2 * PAD;
    let mut y = PAD;

    // ----- Top row (chip · grip · badge) ------------------------------------
    let top = Rectf::new(PAD, y, PAD + content_w, y + TOP_ROW_H);
    lay.top = top;
    lay.drag_zone_bottom_y = (y + TOP_ROW_H) as i32;

    // Chip: measure project name width if we have one
    let chip_text = if state.args.project.is_empty() {
        "(unset)"
    } else {
        state.args.project.as_str()
    };
    let chip_text_w = unsafe {
        if let Some(fmt) = state.fmt_chip.borrow().as_ref() {
            let layout = make_text_layout(chip_text, fmt, 400.0, 30.0)
                .map(|l| measure_layout_width(&l))
                .unwrap_or(80.0);
            layout
        } else {
            80.0
        }
    };
    let mut chip_w = CHIP_PAD_X as f32 + CHIP_DOT as f32 + CHIP_GAP as f32 + chip_text_w
        + CHIP_PAD_X as f32 + 2.0;
    if !state.args.session.is_empty() {
        let hash_w = unsafe {
            if let Some(fmt) = state.fmt_mono.borrow().as_ref() {
                make_text_layout(&state.args.session, fmt, 200.0, 30.0)
                    .map(|l| measure_layout_width(&l))
                    .unwrap_or(50.0)
            } else {
                50.0
            }
        };
        chip_w += CHIP_GAP as f32 + 2.0 + CHIP_GAP as f32 + hash_w + 4.0;
    }
    let chip_left = PAD as f32;
    let chip = Rectf {
        left: chip_left,
        top: y as f32,
        right: chip_left + chip_w,
        bottom: (y + CHIP_H) as f32,
    };
    lay.chip = chip;
    lay.chip_dot = Rectf {
        left: chip.left + CHIP_PAD_X as f32,
        top: (y as f32) + (CHIP_H as f32 - CHIP_DOT as f32) / 2.0,
        right: chip.left + CHIP_PAD_X as f32 + CHIP_DOT as f32,
        bottom: (y as f32) + (CHIP_H as f32 + CHIP_DOT as f32) / 2.0,
    };
    lay.chip_name = Rectf {
        left: lay.chip_dot.right + CHIP_GAP as f32,
        top: y as f32,
        right: lay.chip_dot.right + CHIP_GAP as f32 + chip_text_w,
        bottom: (y + CHIP_H) as f32,
    };
    if !state.args.session.is_empty() {
        lay.chip_sep = Rectf {
            left: lay.chip_name.right + CHIP_GAP as f32,
            top: lay.chip_name.top + 6.0,
            right: lay.chip_name.right + CHIP_GAP as f32 + 1.0,
            bottom: lay.chip_name.top + 6.0 + 12.0,
        };
        let hash_w = unsafe {
            if let Some(fmt) = state.fmt_mono.borrow().as_ref() {
                make_text_layout(&state.args.session, fmt, 200.0, 30.0)
                    .map(|l| measure_layout_width(&l))
                    .unwrap_or(50.0)
            } else {
                50.0
            }
        };
        lay.chip_hash = Rectf {
            left: lay.chip_sep.right + CHIP_GAP as f32,
            top: y as f32,
            right: lay.chip_sep.right + CHIP_GAP as f32 + hash_w,
            bottom: (y + CHIP_H) as f32,
        };
    }

    // Queue badge (right-aligned, drawn only if counter non-empty).
    let counter = state.last_counter.borrow().clone();
    let badge_text = counter.clone();
    let badge_w = if badge_text.is_empty() {
        0
    } else {
        let text_w = unsafe {
            if let Some(fmt) = state.fmt_badge.borrow().as_ref() {
                make_text_layout(&badge_text, fmt, 80.0, 30.0)
                    .map(|l| measure_layout_width(&l))
                    .unwrap_or(14.0)
            } else {
                14.0
            }
        };
        (QUEUE_BADGE_MIN_W).max((text_w as i32) + 2 * QUEUE_BADGE_PAD)
    };
    if badge_w > 0 {
        let badge_right = (PAD + content_w) as f32;
        let badge_left = badge_right - badge_w as f32;
        lay.queue_badge = Rectf {
            left: badge_left,
            top: (y + (TOP_ROW_H - QUEUE_BADGE_H) / 2) as f32,
            right: badge_right,
            bottom: (y + (TOP_ROW_H - QUEUE_BADGE_H) / 2 + QUEUE_BADGE_H) as f32,
        };
    }

    // Grip — 4×2 dots, centered horizontally
    let grip_total_w = 4 * GRIP_DOT + 3 * GRIP_GAP;
    let grip_total_h = 2 * GRIP_DOT + GRIP_GAP;
    let grip_left = PAD + content_w / 2 - grip_total_w / 2;
    let grip_top = y + (TOP_ROW_H - grip_total_h) / 2;
    lay.grip = Rectf::new(
        grip_left,
        grip_top,
        grip_left + grip_total_w,
        grip_top + grip_total_h,
    );

    y += TOP_ROW_H + GAP;

    // ----- Message panel ----------------------------------------------------
    // Footer: input + below it, footer row with legend & dismiss
    let footer_total_h = INPUT_H + GAP / 2 + FOOTER_H;
    let footer_top = client_h - PAD - footer_total_h;

    // Options block (laid out from footer_top - GAP upward); we measure it
    // here, then everything between message_panel.bottom and options.top
    // belongs to the panel.
    let opts_inner_w = content_w - 2 * OPT_PAD_X;
    let mut opt_rects: Vec<Rectf> = Vec::new();
    let mut preview_panel = Rectf::default();

    let need_options = !state.args.options.is_empty();
    let mode = state.mode;

    if need_options {
        match mode {
            OptionMode::Preview => {
                // Two-column grid: options list (1fr) | preview pane (1.05fr)
                let total_grid_w = content_w;
                let gap = PREVIEW_GAP;
                let left_w = ((total_grid_w - gap) as f32 * (1.0 / 2.05)).round() as i32;
                let right_w = total_grid_w - gap - left_w;
                let opt_left_x = PAD;
                let opt_label_w = left_w - 2 * OPT_PAD_X - OPT_NUM_W - OPT_INNER_GAP;
                let mut total_h = 0;
                let mut heights: Vec<f32> = Vec::new();
                for label in &state.args.options {
                    let h = measure_option_height(
                        state,
                        label,
                        opt_label_w as f32,
                        OptionMode::Single,
                    );
                    heights.push(h);
                    total_h += h as i32 + OPT_GAP;
                }
                if !heights.is_empty() {
                    total_h -= OPT_GAP;
                }
                let block_top = footer_top - GAP - total_h;
                let mut cy = block_top as f32;
                for (i, _label) in state.args.options.iter().enumerate() {
                    let h = heights[i];
                    opt_rects.push(Rectf {
                        left: opt_left_x as f32,
                        top: cy,
                        right: (opt_left_x + left_w) as f32,
                        bottom: cy + h,
                    });
                    cy += h + OPT_GAP as f32;
                }
                preview_panel = Rectf {
                    left: (opt_left_x + left_w + gap) as f32,
                    top: block_top as f32,
                    right: (opt_left_x + left_w + gap + right_w) as f32,
                    bottom: (footer_top - GAP) as f32,
                };
                lay.options_block = Rectf {
                    left: opt_left_x as f32,
                    top: block_top as f32,
                    right: (opt_left_x + total_grid_w) as f32,
                    bottom: (footer_top - GAP) as f32,
                };
            }
            _ => {
                let mut total_h = 0;
                let mut heights: Vec<f32> = Vec::new();
                for (i, label) in state.args.options.iter().enumerate() {
                    let h = measure_option_height(state, label, opts_inner_w as f32, mode);
                    heights.push(h);
                    total_h += h as i32;
                    if i + 1 < state.args.options.len() {
                        total_h += OPT_GAP;
                    }
                }
                let block_top = footer_top - GAP - total_h;
                let mut cy = block_top as f32;
                for h in &heights {
                    opt_rects.push(Rectf {
                        left: PAD as f32,
                        top: cy,
                        right: (PAD + content_w) as f32,
                        bottom: cy + *h,
                    });
                    cy += *h + OPT_GAP as f32;
                }
                lay.options_block = Rectf {
                    left: PAD as f32,
                    top: block_top as f32,
                    right: (PAD + content_w) as f32,
                    bottom: (footer_top - GAP) as f32,
                };
            }
        }
    }

    let panel_top = y;
    let panel_bottom = if need_options {
        lay.options_block.top as i32 - GAP
    } else {
        footer_top - GAP
    };
    let panel = Rectf::new(PAD, panel_top, PAD + content_w, panel_bottom);
    lay.message_panel = panel;
    lay.message_text = Rectf {
        left: panel.left + PANEL_PAD_X as f32,
        top: panel.top + PANEL_PAD_Y as f32,
        right: panel.right - PANEL_PAD_X as f32,
        bottom: panel.bottom - PANEL_PAD_Y as f32,
    };

    // Measure full wrapped message height for scrolling.
    let msg_text_w = lay.message_text.w();
    let full_h = unsafe {
        if let Some(fmt) = state.fmt_body.borrow().as_ref() {
            make_styled_layout(&state.parsed_message, fmt, msg_text_w, 99999.0)
                .map(|l| measure_layout_height(&l))
                .unwrap_or(0.0)
        } else {
            0.0
        }
    };
    lay.message_total_h = full_h;

    let visible_h = lay.message_text.h();
    if full_h > visible_h + 0.5 {
        // We need a scroll track on the right of the panel.
        let track_x_right = lay.message_panel.right - PANEL_SCROLL_INSET as f32;
        let track_x_left = track_x_right - PANEL_SCROLL_W as f32;
        lay.message_scroll_track = Rectf {
            left: track_x_left,
            top: lay.message_panel.top + PANEL_SCROLL_INSET as f32,
            right: track_x_right,
            bottom: lay.message_panel.bottom - PANEL_SCROLL_INSET as f32,
        };
        // Shrink text width so it doesn't overlap the scrollbar.
        lay.message_text.right = track_x_left - 4.0;
    }

    lay.options = opt_rects;
    lay.preview_panel = preview_panel;

    // ----- Input EDIT -------------------------------------------------------
    let input_top = footer_top;
    lay.input = Rectf::new(PAD, input_top, PAD + content_w, input_top + INPUT_H);

    // ----- Footer row -------------------------------------------------------
    let footer_y = client_h - PAD - FOOTER_H;
    lay.footer = Rectf::new(PAD, footer_y, PAD + content_w, footer_y + FOOTER_H);

    // Enter legend on the left.
    let legend_text_w = unsafe {
        if let Some(fmt) = state.fmt_legend.borrow().as_ref() {
            make_text_layout("Enter to send", fmt, 200.0, 30.0)
                .map(|l| measure_layout_width(&l))
                .unwrap_or(80.0)
        } else {
            80.0
        }
    };
    // Enter pip width + gap + label width
    let pip_legend_w = PIP_W as f32;
    lay.enter_legend = Rectf {
        left: PAD as f32 + 2.0,
        top: footer_y as f32,
        right: PAD as f32 + 2.0 + pip_legend_w + 6.0 + legend_text_w,
        bottom: (footer_y + FOOTER_H) as f32,
    };

    // Dismiss cluster on the right.
    let dismiss_pips_w = 2 * PIP_W + PIP_GAP;
    // Measure with a small safety margin — DirectWrite returns the ideal
    // width but the actual layout can need an extra pixel or two to avoid
    // wrapping the trailing glyph.
    let dismiss_label_w = unsafe {
        if let Some(fmt) = state.fmt_legend.borrow().as_ref() {
            make_text_layout("Dismiss", fmt, 200.0, 30.0)
                .map(|l| measure_layout_width(&l))
                .unwrap_or(56.0)
        } else {
            56.0
        }
    } + 6.0;
    let dismiss_total_w = DISMISS_PAD * 2
        + dismiss_pips_w
        + DISMISS_LABEL_GAP
        + dismiss_label_w as i32;
    let dismiss_right = (PAD + content_w) as i32;
    let dismiss_left = dismiss_right - dismiss_total_w;
    let dismiss_rect = Rectf::new(dismiss_left, footer_y, dismiss_right, footer_y + FOOTER_H);
    lay.dismiss = dismiss_rect;

    let pip1_left = dismiss_rect.left + DISMISS_PAD as f32;
    let pip_top = footer_y as f32 + (FOOTER_H as f32 - PIP_H as f32) / 2.0 - 2.0;
    lay.dismiss_pip1 = Rectf {
        left: pip1_left,
        top: pip_top,
        right: pip1_left + PIP_W as f32,
        bottom: pip_top + PIP_H as f32,
    };
    let pip2_left = lay.dismiss_pip1.right + PIP_GAP as f32;
    lay.dismiss_pip2 = Rectf {
        left: pip2_left,
        top: pip_top,
        right: pip2_left + PIP_W as f32,
        bottom: pip_top + PIP_H as f32,
    };
    lay.dismiss_progress = Rectf {
        left: lay.dismiss_pip1.left,
        top: lay.dismiss_pip2.bottom + 2.0,
        right: lay.dismiss_pip2.right,
        bottom: lay.dismiss_pip2.bottom + 2.0 + 2.0,
    };
    lay.dismiss_label = Rectf {
        left: lay.dismiss_pip2.right + DISMISS_LABEL_GAP as f32,
        top: footer_y as f32,
        right: dismiss_rect.right - DISMISS_PAD as f32,
        bottom: (footer_y + FOOTER_H) as f32,
    };

    lay
}

/// Measure the height of an option card given the available label width and
/// the active option mode. Always at least OPT_MIN_H.
fn measure_option_height(state: &WindowState, label: &str, label_w: f32, mode: OptionMode) -> f32 {
    let text_h = unsafe {
        if let Some(fmt) = state.fmt_option.borrow().as_ref() {
            make_text_layout(label, fmt, label_w.max(40.0), 600.0)
                .map(|l| measure_layout_height(&l))
                .unwrap_or(18.0)
        } else {
            18.0
        }
    };
    let inner_h = match mode {
        OptionMode::Approve => 22.0,
        _ => text_h.max(18.0),
    };
    let h = inner_h + 2.0 * OPT_PAD_Y as f32;
    h.max(OPT_MIN_H as f32)
}

/// Plan the initial window size based on the message + options.
fn plan_window_size(state: &WindowState) -> (i32, i32) {
    // Width starts at the default; grow only if the longest option label
    // would clip badly. The popup is content-driven height, fixed-ish width.
    let mut width = POPUP_DEFAULT_W;
    if !state.args.options.is_empty() {
        let longest_w = unsafe {
            let mut max_w: f32 = 0.0;
            if let Some(fmt) = state.fmt_option.borrow().as_ref() {
                for label in &state.args.options {
                    if let Some(l) = make_text_layout(label, fmt, 4000.0, 30.0) {
                        let w = measure_layout_width(&l);
                        if w > max_w {
                            max_w = w;
                        }
                    }
                }
            }
            max_w
        };
        let needed = longest_w as i32 + OPT_NUM_W + OPT_INNER_GAP + 2 * OPT_PAD_X + 2 * PAD + 24;
        width = width.max(needed).min(POPUP_MAX_W);
    }
    width = width.clamp(POPUP_MIN_W, POPUP_MAX_W);

    // Estimate height: simulate a layout pass at the chosen width with a
    // generous initial client height, then read back what we need.
    let lay = layout_window(width, POPUP_MAX_H, state);
    // Replay: compute the actual required client height.
    //   y = PAD + TOP_ROW_H + GAP + panel_h + GAP + options_block + GAP + INPUT_H + (GAP/2) + FOOTER_H + PAD
    let panel_h_needed = lay.message_total_h.min(PANEL_MAX_H as f32) + 2.0 * PANEL_PAD_Y as f32;
    let panel_h = panel_h_needed.max(60.0); // minimum panel height
    let options_h: f32 = if state.args.options.is_empty() {
        0.0
    } else {
        match state.mode {
            OptionMode::Preview => {
                let opts_inner_w = ((width - 2 * PAD) / 2 - 2 * OPT_PAD_X) as f32;
                let mut total = 0.0;
                for label in &state.args.options {
                    total += measure_option_height(state, label, opts_inner_w, OptionMode::Single)
                        + OPT_GAP as f32;
                }
                total - OPT_GAP as f32
            }
            _ => {
                let opts_inner_w = ((width - 2 * PAD) - 2 * OPT_PAD_X) as f32;
                let mut total = 0.0;
                for (i, label) in state.args.options.iter().enumerate() {
                    total += measure_option_height(state, label, opts_inner_w, state.mode);
                    if i + 1 < state.args.options.len() {
                        total += OPT_GAP as f32;
                    }
                }
                total
            }
        }
    };
    let options_block_total = if state.args.options.is_empty() {
        0
    } else {
        options_h as i32 + GAP
    };
    let raw_h = PAD
        + TOP_ROW_H
        + GAP
        + panel_h as i32
        + GAP
        + options_block_total
        + INPUT_H
        + GAP / 2
        + FOOTER_H
        + PAD;
    let height = raw_h.clamp(POPUP_MIN_H, POPUP_MAX_H);
    (width, height)
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
                        if (*state).is_head_shown.get() {
                            let single = SINGLE_ESC.swap(false, Ordering::SeqCst);
                            let double = DOUBLE_ESC.swap(false, Ordering::SeqCst);
                            let cur = (*state).dismiss.get();
                            let (next, dismiss) =
                                advance_dismiss(cur, single, double, Instant::now());
                            (*state).dismiss.set(next);
                            if dismiss {
                                *(*state).outcome.borrow_mut() = Some(Outcome::Dismissed);
                                let _ = DestroyWindow(hwnd);
                                return LRESULT(0);
                            }
                            // Repaint footer region while non-rest, or once
                            // on the transition back to rest.
                            invalidate_footer(hwnd, &*state);
                            // Manage the animation timer.
                            let want_anim =
                                matches!(next.phase, DismissPhase::Armed | DismissPhase::Timeout);
                            if want_anim && !(*state).anim_active.get() {
                                SetTimer(hwnd, ID_ANIM_TIMER, ANIM_TICK_MS, None);
                                (*state).anim_active.set(true);
                            } else if !want_anim && (*state).anim_active.get() {
                                let _ = KillTimer(hwnd, ID_ANIM_TIMER);
                                (*state).anim_active.set(false);
                            }
                        }
                    }
                }
                ID_POLL_TIMER => {
                    poll_queue(hwnd);
                }
                ID_ANIM_TIMER => {
                    if let Some(state) = state_ptr(hwnd) {
                        invalidate_footer(hwnd, &*state);
                    }
                }
                ID_CARET_TIMER => {
                    if let Some(state) = state_ptr(hwnd) {
                        let state = &*state;
                        // After a fresh cursor move, hold the caret solid-
                        // on for a beat so the user can spot where it
                        // landed; then resume blinking.
                        let solid = state.caret_moved_at.get().elapsed().as_millis()
                            < CARET_SOLID_AFTER_MOVE_MS;
                        let new_on = if solid { true } else { !state.caret_on.get() };
                        if new_on != state.caret_on.get() {
                            state.caret_on.set(new_on);
                            if let Some(lay) = state.cached_layout.borrow().as_ref() {
                                invalidate_rectf(hwnd, &lay.input);
                            }
                        }
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1), // D2D paints the whole client area
        WM_SIZE => {
            if let Some(state) = state_ptr(hwnd) {
                (*state).drop_d2d();
            }
            LRESULT(0)
        }
        WM_NCHITTEST => {
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
        WM_SETCURSOR => {
            // Show the I-beam over the input area; arrow elsewhere.
            if let Some(state) = state_ptr(hwnd) {
                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);
                let _ = ScreenToClient(hwnd, &mut pt);
                if let Some(lay) = (*state).cached_layout.borrow().as_ref() {
                    if lay.input.contains(pt.x, pt.y) {
                        let _ = SetCursor(LoadCursorW(HINSTANCE::default(), IDC_IBEAM).unwrap_or_default());
                        return LRESULT(1);
                    }
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CHAR => {
            if let Some(state) = state_ptr(hwnd) {
                let state = &*state;
                let ch = wparam.0 as u16;
                // Filter control characters except tab — we handle Backspace,
                // Enter, etc. in WM_KEYDOWN.
                if ch >= 0x20 || ch == 0x09 {
                    if let Some(c) = char::from_u32(ch as u32) {
                        let s = c.to_string();
                        input_insert(state, &s);
                        if let Some(lay) = state.cached_layout.borrow().as_ref() {
                            invalidate_rectf(hwnd, &lay.input);
                        }
                    }
                }
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if let Some(state) = state_ptr(hwnd) {
                if handle_input_keydown(hwnd, &*state, wparam.0 as u32) {
                    return LRESULT(0);
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_MOUSEMOVE => {
            if let Some(state) = state_ptr(hwnd) {
                let x = (lparam.0 & 0xFFFF) as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                handle_mouse_move(hwnd, &*state, x, y);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            if let Some(state) = state_ptr(hwnd) {
                let x = (lparam.0 & 0xFFFF) as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                handle_mouse_down(hwnd, &*state, x, y);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if let Some(state) = state_ptr(hwnd) {
                let state = &*state;
                if state.input_selecting.get() {
                    state.input_selecting.set(false);
                    let _ = ReleaseCapture();
                }
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            if let Some(state) = state_ptr(hwnd) {
                let delta = ((wparam.0 >> 16) & 0xFFFF) as i16 as i32;
                handle_wheel(hwnd, &*state, delta);
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
                (*state).drop_d2d();
                if (*state).anim_active.get() {
                    let _ = KillTimer(hwnd, ID_ANIM_TIMER);
                }
            }
            let _ = KillTimer(hwnd, ID_ESC_TIMER);
            let _ = KillTimer(hwnd, ID_POLL_TIMER);
            let _ = KillTimer(hwnd, ID_CARET_TIMER);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn invalidate_footer(hwnd: HWND, state: &WindowState) {
    if let Some(lay) = state.cached_layout.borrow().as_ref() {
        let r = RECT {
            left: lay.footer.left as i32 - 4,
            top: lay.footer.top as i32 - 2,
            right: lay.footer.right as i32 + 4,
            bottom: lay.footer.bottom as i32 + 2,
        };
        let _ = InvalidateRect(hwnd, Some(&r), BOOL(0));
    } else {
        let _ = InvalidateRect(hwnd, None, BOOL(0));
    }
}

unsafe fn handle_mouse_move(hwnd: HWND, state: &WindowState, x: i32, y: i32) {
    let lay = match state.cached_layout.borrow().clone() {
        Some(l) => l,
        None => return,
    };

    // During an active text-selection drag, every mouse-move extends the
    // selection to the current caret position.
    if state.input_selecting.get() {
        let pos = input_pos_for_mouse(state, &lay, x, y);
        if pos != state.input_cursor.get() {
            state.input_cursor.set(pos);
            state.caret_moved_at.set(Instant::now());
            state.caret_on.set(true);
            invalidate_rectf(hwnd, &lay.input);
        }
        return;
    }

    let on_top = lay.top.contains(x, y);
    if state.hover_top_row.get() != on_top {
        state.hover_top_row.set(on_top);
        invalidate_rectf(hwnd, &lay.top);
    }
    let on_dismiss = lay.dismiss.contains(x, y);
    if state.hover_dismiss.get() != on_dismiss {
        state.hover_dismiss.set(on_dismiss);
        invalidate_rectf(hwnd, &lay.dismiss);
    }
    let mut hover_idx: isize = -1;
    for (i, r) in lay.options.iter().enumerate() {
        if r.contains(x, y) {
            hover_idx = i as isize;
            break;
        }
    }
    let prev_hover = state.hover_opt.get();
    if prev_hover != hover_idx {
        state.hover_opt.set(hover_idx);
        if prev_hover >= 0 {
            if let Some(r) = lay.options.get(prev_hover as usize) {
                invalidate_rectf(hwnd, r);
            }
        }
        if hover_idx >= 0 {
            if let Some(r) = lay.options.get(hover_idx as usize) {
                invalidate_rectf(hwnd, r);
            }
        }
    }
}

/// Invalidate a Rectf in the parent window, leaving everything outside it
/// (including the EDIT child) untouched.
unsafe fn invalidate_rectf(hwnd: HWND, r: &Rectf) {
    let rc = RECT {
        left: r.left as i32 - 2,
        top: r.top as i32 - 2,
        right: r.right as i32 + 2,
        bottom: r.bottom as i32 + 2,
    };
    let _ = InvalidateRect(hwnd, Some(&rc), BOOL(0));
}

/// Returns true if the key was consumed by the input field.
unsafe fn handle_input_keydown(hwnd: HWND, state: &WindowState, vk: u32) -> bool {
    let ctrl = (GetKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000) != 0;
    let shift = (GetKeyState(VK_SHIFT.0 as i32) as u16 & 0x8000) != 0;
    let mut moved = false;
    let mut changed = false;

    // Helper to begin/extend selection based on Shift.
    let start_or_extend = |state: &WindowState| {
        if state.input_anchor.get().is_none() {
            state.input_anchor.set(Some(state.input_cursor.get()));
        }
    };
    let collapse = |state: &WindowState| {
        state.input_anchor.set(None);
    };

    match vk {
        v if v == VK_LEFT.0 as u32 => {
            if shift {
                start_or_extend(state);
            } else {
                collapse(state);
            }
            let cur = state.input_cursor.get();
            let text = state.input_text.borrow();
            let next = if ctrl {
                step_word(&text, cur, false)
            } else {
                prev_char_boundary(&text, cur)
            };
            drop(text);
            state.input_cursor.set(next);
            moved = true;
        }
        v if v == VK_RIGHT.0 as u32 => {
            if shift {
                start_or_extend(state);
            } else {
                collapse(state);
            }
            let cur = state.input_cursor.get();
            let text = state.input_text.borrow();
            let next = if ctrl {
                step_word(&text, cur, true)
            } else {
                next_char_boundary(&text, cur)
            };
            drop(text);
            state.input_cursor.set(next);
            moved = true;
        }
        v if v == VK_HOME.0 as u32 => {
            if shift {
                start_or_extend(state);
            } else {
                collapse(state);
            }
            let cur = state.input_cursor.get();
            let text = state.input_text.borrow();
            // Beginning of current line (or whole text if Ctrl).
            let next = if ctrl {
                0
            } else {
                text[..cur].rfind('\n').map(|i| i + 1).unwrap_or(0)
            };
            drop(text);
            state.input_cursor.set(next);
            moved = true;
        }
        v if v == VK_END.0 as u32 => {
            if shift {
                start_or_extend(state);
            } else {
                collapse(state);
            }
            let cur = state.input_cursor.get();
            let text = state.input_text.borrow();
            let next = if ctrl {
                text.len()
            } else {
                text[cur..].find('\n').map(|i| cur + i).unwrap_or(text.len())
            };
            drop(text);
            state.input_cursor.set(next);
            moved = true;
        }
        v if v == VK_UP.0 as u32 || v == VK_DOWN.0 as u32 => {
            if shift {
                start_or_extend(state);
            } else {
                collapse(state);
            }
            let going_down = vk == VK_DOWN.0 as u32;
            move_caret_vertical(state, going_down);
            moved = true;
        }
        v if v == VK_BACK.0 as u32 => {
            if !input_delete_selection(state) {
                let cur = state.input_cursor.get();
                if cur > 0 {
                    let text = state.input_text.borrow();
                    let start = if ctrl {
                        step_word(&text, cur, false)
                    } else {
                        prev_char_boundary(&text, cur)
                    };
                    drop(text);
                    state.input_text.borrow_mut().replace_range(start..cur, "");
                    state.input_cursor.set(start);
                    state.caret_moved_at.set(Instant::now());
                    state.caret_on.set(true);
                    changed = true;
                }
            } else {
                changed = true;
            }
        }
        v if v == VK_DELETE.0 as u32 => {
            if !input_delete_selection(state) {
                let cur = state.input_cursor.get();
                let text = state.input_text.borrow();
                if cur < text.len() {
                    let end = if ctrl {
                        step_word(&text, cur, true)
                    } else {
                        next_char_boundary(&text, cur)
                    };
                    drop(text);
                    state.input_text.borrow_mut().replace_range(cur..end, "");
                    state.caret_moved_at.set(Instant::now());
                    state.caret_on.set(true);
                    changed = true;
                }
            } else {
                changed = true;
            }
        }
        v if v == VK_RETURN.0 as u32 => {
            if shift {
                input_insert(state, "\n");
                changed = true;
            } else {
                on_enter(hwnd, state);
                return true;
            }
        }
        v if ctrl && v == VK_A.0 as u32 => {
            let text_len = state.input_text.borrow().len();
            state.input_anchor.set(Some(0));
            state.input_cursor.set(text_len);
            state.caret_moved_at.set(Instant::now());
            state.caret_on.set(true);
            moved = true;
        }
        v if ctrl && (v == VK_C.0 as u32 || v == VK_X.0 as u32) => {
            if let Some((s, e)) = input_selection_range(state) {
                let selected = state.input_text.borrow()[s..e].to_string();
                clipboard_set(hwnd, &selected);
                if v == VK_X.0 as u32 {
                    let _ = input_delete_selection(state);
                    changed = true;
                }
            }
        }
        v if ctrl && v == VK_V.0 as u32 => {
            if let Some(s) = clipboard_get(hwnd) {
                if !s.is_empty() {
                    input_insert(state, &s);
                    changed = true;
                }
            }
        }
        _ => return false,
    }

    if moved || changed {
        if let Some(lay) = state.cached_layout.borrow().as_ref() {
            invalidate_rectf(hwnd, &lay.input);
        }
    }
    true
}

/// Move the caret one visual line up (or down) using DWrite hit-testing.
/// Preserves approximate x-position across lines.
unsafe fn move_caret_vertical(state: &WindowState, going_down: bool) {
    let lay = state.cached_layout.borrow().clone();
    let lay = match lay {
        Some(l) => l,
        None => return,
    };
    let text = state.input_text.borrow().clone();
    if text.is_empty() {
        return;
    }
    let inner_w = (lay.input.w() - 2.0 * INPUT_TEXT_PAD_X).max(40.0);
    let inner_h = (lay.input.h() - 2.0 * INPUT_TEXT_PAD_Y).max(20.0);
    let layout = match build_input_layout(state, &text, inner_w, inner_h) {
        Some(l) => l,
        None => return,
    };
    let cur = state.input_cursor.get();
    let utf16_pos = utf16_pos_for_byte(&text, cur);
    let mut x = 0.0f32;
    let mut y = 0.0f32;
    let mut m = DWRITE_HIT_TEST_METRICS::default();
    let _ = layout.HitTestTextPosition(utf16_pos, false, &mut x, &mut y, &mut m);
    let target_y = if going_down {
        y + m.height + 1.0
    } else {
        (y - m.height + 1.0).max(0.0)
    };
    let mut is_trailing = BOOL(0);
    let mut is_inside = BOOL(0);
    let mut m2 = DWRITE_HIT_TEST_METRICS::default();
    let _ = layout.HitTestPoint(x, target_y, &mut is_trailing, &mut is_inside, &mut m2);
    let new_utf16 =
        m2.textPosition + if is_trailing.as_bool() { m2.length } else { 0 };
    let new_byte = byte_pos_for_utf16(&text, new_utf16 as usize);
    state.input_cursor.set(new_byte);
    state.caret_moved_at.set(Instant::now());
    state.caret_on.set(true);
}

/// Map a Rust byte index in a UTF-8 string to a UTF-16 code-unit index
/// (DWrite operates in UTF-16).
fn utf16_pos_for_byte(text: &str, byte_pos: usize) -> u32 {
    let mut count: u32 = 0;
    let mut byte = 0usize;
    for c in text.chars() {
        if byte >= byte_pos {
            break;
        }
        count += c.len_utf16() as u32;
        byte += c.len_utf8();
    }
    count
}

/// Inverse of utf16_pos_for_byte.
fn byte_pos_for_utf16(text: &str, utf16_pos: usize) -> usize {
    let mut utf16 = 0usize;
    let mut byte = 0usize;
    for c in text.chars() {
        if utf16 >= utf16_pos {
            break;
        }
        utf16 += c.len_utf16();
        byte += c.len_utf8();
    }
    byte.min(text.len())
}

/// Hit-test (x, y) against the input area to figure out where the caret
/// should land. Returns the byte index in input_text.
unsafe fn input_pos_for_mouse(state: &WindowState, lay: &Layout, x: i32, y: i32) -> usize {
    let text = state.input_text.borrow().clone();
    if text.is_empty() {
        return 0;
    }
    let inner_x = (x as f32 - lay.input.left - INPUT_TEXT_PAD_X).max(0.0);
    let inner_y = (y as f32 - lay.input.top - INPUT_TEXT_PAD_Y).max(0.0);
    let inner_w = (lay.input.w() - 2.0 * INPUT_TEXT_PAD_X).max(40.0);
    let inner_h = (lay.input.h() - 2.0 * INPUT_TEXT_PAD_Y).max(20.0);
    let layout = match build_input_layout(state, &text, inner_w, inner_h) {
        Some(l) => l,
        None => return text.len(),
    };
    let mut is_trailing = BOOL(0);
    let mut is_inside = BOOL(0);
    let mut m = DWRITE_HIT_TEST_METRICS::default();
    let _ = layout.HitTestPoint(inner_x, inner_y, &mut is_trailing, &mut is_inside, &mut m);
    let utf16_pos = m.textPosition + if is_trailing.as_bool() { m.length } else { 0 };
    byte_pos_for_utf16(&text, utf16_pos as usize)
}

unsafe fn handle_mouse_down(hwnd: HWND, state: &WindowState, x: i32, y: i32) {
    let lay = match state.cached_layout.borrow().clone() {
        Some(l) => l,
        None => return,
    };

    // Click inside the input area positions the caret + starts a selection
    // drag. Shift+click extends from the existing anchor.
    if lay.input.contains(x, y) {
        let pos = input_pos_for_mouse(state, &lay, x, y);
        let shift = (GetKeyState(VK_SHIFT.0 as i32) as u16 & 0x8000) != 0;
        if shift {
            if state.input_anchor.get().is_none() {
                state.input_anchor.set(Some(state.input_cursor.get()));
            }
        } else {
            state.input_anchor.set(Some(pos));
        }
        state.input_cursor.set(pos);
        state.input_focused.set(true);
        state.input_selecting.set(true);
        state.caret_moved_at.set(Instant::now());
        state.caret_on.set(true);
        let _ = SetCapture(hwnd);
        invalidate_rectf(hwnd, &lay.input);
        return;
    }

    if lay.dismiss.contains(x, y) {
        SINGLE_ESC.store(true, Ordering::SeqCst);
        LAST_ESC.with(|c| {
            let mut last = c.borrow_mut();
            let now = Instant::now();
            if let Some(prev) = *last {
                if now.duration_since(prev).as_millis() <= DISMISS_ARM_MS {
                    DOUBLE_ESC.store(true, Ordering::SeqCst);
                }
            }
            *last = Some(now);
        });
        return;
    }

    for (i, r) in lay.options.iter().enumerate() {
        if r.contains(x, y) {
            match state.mode {
                OptionMode::Multi => {
                    let mut sel = state.selected.borrow_mut();
                    if sel.len() < state.args.options.len() {
                        sel.resize(state.args.options.len(), false);
                    }
                    sel[i] = !sel[i];
                    drop(sel);
                    state.focus_opt.set(i as isize);
                    invalidate_rectf(hwnd, r);
                    return;
                }
                OptionMode::Preview => {
                    let prev = state.focus_opt.get();
                    state.focus_opt.set(i as isize);
                    if prev >= 0 {
                        if let Some(pr) = lay.options.get(prev as usize) {
                            invalidate_rectf(hwnd, pr);
                        }
                    }
                    invalidate_rectf(hwnd, r);
                    invalidate_rectf(hwnd, &lay.preview_panel);
                    return;
                }
                OptionMode::Single | OptionMode::Approve => {
                    let label = state.args.options[i].clone();
                    *state.outcome.borrow_mut() = Some(Outcome::Answered(label));
                    let _ = DestroyWindow(hwnd);
                    return;
                }
            }
        }
    }

    if lay.message_scroll_track.w() > 0.5 && lay.message_scroll_track.contains(x, y) {
        let cur = state.scroll_y.get();
        let page = lay.message_text.h();
        let max_scroll =
            (lay.message_total_h - lay.message_text.h()).max(0.0);
        let mid_y = (lay.message_scroll_track.top + lay.message_scroll_track.bottom) / 2.0;
        let dir = if (y as f32) < mid_y { -1.0 } else { 1.0 };
        let next = (cur + dir * page * 0.8).clamp(0.0, max_scroll);
        state.scroll_y.set(next);
        invalidate_rectf(hwnd, &lay.message_panel);
    }
}

unsafe fn handle_wheel(hwnd: HWND, state: &WindowState, delta: i32) {
    let lay = match state.cached_layout.borrow().clone() {
        Some(l) => l,
        None => return,
    };
    let max_scroll = (lay.message_total_h - lay.message_text.h()).max(0.0);
    if max_scroll < 0.5 {
        return;
    }
    let step = -(delta as f32) * (40.0 / 120.0);
    let next = (state.scroll_y.get() + step).clamp(0.0, max_scroll);
    state.scroll_y.set(next);
    // Only invalidate the message panel — leaving the EDIT child + footer
    // alone so scrolling doesn't make the user's typed text flash.
    invalidate_rectf(hwnd, &lay.message_panel);
}

// ===========================================================================
// Poll loop
// ===========================================================================
unsafe fn poll_queue(hwnd: HWND) {
    let state = match state_ptr(hwnd) {
        Some(s) => s,
        None => return,
    };
    cleanup_stale_queue(&(*state).req_path);
    let (pos, total) = queue_position(&(*state).req_path);
    if pos == 0 {
        *(*state).outcome.borrow_mut() = Some(Outcome::Dismissed);
        let _ = DestroyWindow(hwnd);
        return;
    }
    let new_counter = format_counter(pos, total);
    if *(*state).last_counter.borrow() != new_counter {
        *(*state).last_counter.borrow_mut() = new_counter;
        let _ = InvalidateRect(hwnd, None, BOOL(0));
    }
    if pos == 1 && !(*state).is_head_shown.get() {
        reset_esc_flags();
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
        SetTimer(hwnd, ID_POLL_TIMER, POLL_SHOWN_MS, None);
    }
}

// ===========================================================================
// Initial layout pass + caret-blink timer (no child windows — the input is
// painted directly in WM_PAINT).
// ===========================================================================
unsafe fn create_children(hwnd: HWND, state_ptr: *mut WindowState) {
    let state = &mut *state_ptr;

    state.ensure_text_formats();
    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let lay = layout_window(rc.right, rc.bottom, state);
    state.drag_zone_bottom_y.set(lay.drag_zone_bottom_y);
    *state.cached_layout.borrow_mut() = Some(lay.clone());

    // Caret blink — kept always-on so the user can see they have input focus.
    SetTimer(hwnd, ID_CARET_TIMER, CARET_BLINK_MS, None);
}

// ===========================================================================
// Paint
// ===========================================================================
unsafe fn paint(hwnd: HWND) {
    let state = match state_ptr(hwnd) {
        Some(s) => s,
        None => {
            let mut ps = PAINTSTRUCT::default();
            let _ = BeginPaint(hwnd, &mut ps);
            let _ = EndPaint(hwnd, &ps);
            return;
        }
    };
    let state = &*state;
    state.ensure_text_formats();
    state.ensure_render_target(hwnd);

    // Validate the paint area; we paint everything via D2D below.
    let mut ps = PAINTSTRUCT::default();
    let _ = BeginPaint(hwnd, &mut ps);

    let rt = {
        let g = state.render_target.borrow();
        match g.as_ref() {
            Some(r) => r.clone(),
            None => {
                let _ = EndPaint(hwnd, &ps);
                return;
            }
        }
    };

    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let client_w = rc.right;
    let client_h = rc.bottom;
    let lay = layout_window(client_w, client_h, state);
    *state.cached_layout.borrow_mut() = Some(lay.clone());
    state.drag_zone_bottom_y.set(lay.drag_zone_bottom_y);

    let p = state.palette;

    rt.BeginDraw();

    let brush = rt
        .CreateSolidColorBrush(&argb_to_color(p.body), None)
        .ok();
    if let Some(brush) = brush {
        // Paint the popup background everywhere EXCEPT inside the EDIT
        // child's rect. Calling Clear() would zero the entire D2D surface
        // (Clear ignores any clip), and EndDraw() then presents that
        // cleared surface to the HWND on top of the EDIT's painted text —
        // which is what was making the user's typed input disappear between
        // EDIT-internal repaints. With FillRectangle we control the exact
        // pixels we touch.
        let edit_top = lay.input.top + 2.0;
        let edit_bot = lay.input.bottom - 2.0;
        let edit_left = lay.input.left + 2.0;
        let edit_right = lay.input.right - 2.0;
        let cw = client_w as f32;
        let ch = client_h as f32;
        brush.SetColor(&argb_to_color(p.bg));
        rt.FillRectangle(
            &D2D_RECT_F { left: 0.0, top: 0.0, right: cw, bottom: edit_top },
            &brush,
        );
        rt.FillRectangle(
            &D2D_RECT_F { left: 0.0, top: edit_bot, right: cw, bottom: ch },
            &brush,
        );
        rt.FillRectangle(
            &D2D_RECT_F { left: 0.0, top: edit_top, right: edit_left, bottom: edit_bot },
            &brush,
        );
        rt.FillRectangle(
            &D2D_RECT_F { left: edit_right, top: edit_top, right: cw, bottom: edit_bot },
            &brush,
        );

        // ---------- Window outline (subtle 1px) ------------------------------
        let outline_rect = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: 0.5,
                top: 0.5,
                right: client_w as f32 - 0.5,
                bottom: client_h as f32 - 0.5,
            },
            radiusX: POPUP_RADIUS,
            radiusY: POPUP_RADIUS,
        };
        brush.SetColor(&argb_to_color(0x10_FF_FF_FF));
        rt.DrawRoundedRectangle(&outline_rect, &brush, 1.0, None);

        // ---------- Chip (two-fill bordered rounded rectangle) -------------
        // Two-fill technique (variant D in the pill-lab comparison): outer
        // rect in the border color, inner rect 1 px smaller in the fill
        // color. Pixel-perfect 1 px border regardless of DPI / subpixel
        // alignment, which is what fixed the "just an oval" look.
        fill_with_border(
            &rt, &brush, lay.chip.to_d2d(), 6.0, 1.0, p.chip, p.chip_border,
        );

        // Dot halo + dot
        let dot_cx = (lay.chip_dot.left + lay.chip_dot.right) / 2.0;
        let dot_cy = (lay.chip_dot.top + lay.chip_dot.bottom) / 2.0;
        let halo = D2D1_ELLIPSE {
            point: D2D_POINT_2F { x: dot_cx, y: dot_cy },
            radiusX: (CHIP_DOT as f32) / 2.0 + 3.0,
            radiusY: (CHIP_DOT as f32) / 2.0 + 3.0,
        };
        brush.SetColor(&argb_to_color(p.accent_soft));
        rt.FillEllipse(&halo, &brush);
        let dot = D2D1_ELLIPSE {
            point: D2D_POINT_2F { x: dot_cx, y: dot_cy },
            radiusX: (CHIP_DOT as f32) / 2.0,
            radiusY: (CHIP_DOT as f32) / 2.0,
        };
        brush.SetColor(&argb_to_color(p.accent));
        rt.FillEllipse(&dot, &brush);

        // Chip name text
        if let Some(fmt) = state.fmt_chip.borrow().as_ref() {
            if let Some(layout) = make_text_layout(
                if state.args.project.is_empty() {
                    "(unset)"
                } else {
                    state.args.project.as_str()
                },
                fmt,
                lay.chip_name.w(),
                lay.chip_name.h(),
            ) {
                let _ = layout.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
                brush.SetColor(&argb_to_color(p.title));
                rt.DrawTextLayout(
                    D2D_POINT_2F {
                        x: lay.chip_name.left,
                        y: lay.chip_name.top,
                    },
                    &layout,
                    &brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        }

        // Optional separator + session hash
        if !state.args.session.is_empty() {
            brush.SetColor(&argb_to_color(p.chip_border));
            rt.FillRectangle(&lay.chip_sep.to_d2d(), &brush);
            if let Some(fmt) = state.fmt_mono.borrow().as_ref() {
                if let Some(layout) = make_text_layout(
                    &state.args.session,
                    fmt,
                    lay.chip_hash.w(),
                    lay.chip_hash.h(),
                ) {
                    let _ = layout.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
                    brush.SetColor(&argb_to_color(p.dim));
                    rt.DrawTextLayout(
                        D2D_POINT_2F {
                            x: lay.chip_hash.left,
                            y: lay.chip_hash.top,
                        },
                        &layout,
                        &brush,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                    );
                }
            }
        }

        // ---------- Grip -----------------------------------------------------
        let grip_alpha = if state.hover_top_row.get() { 0.72 } else { 0.32 };
        let grip_color = mix_alpha(p.dim, grip_alpha);
        brush.SetColor(&argb_to_color(grip_color));
        for row in 0..2 {
            for col in 0..4 {
                let cx = lay.grip.left + (col as f32) * (GRIP_DOT + GRIP_GAP) as f32 + GRIP_DOT as f32 / 2.0;
                let cy = lay.grip.top + (row as f32) * (GRIP_DOT + GRIP_GAP) as f32 + GRIP_DOT as f32 / 2.0;
                let dot = D2D1_ELLIPSE {
                    point: D2D_POINT_2F { x: cx, y: cy },
                    radiusX: GRIP_DOT as f32 / 2.0,
                    radiusY: GRIP_DOT as f32 / 2.0,
                };
                rt.FillEllipse(&dot, &brush);
            }
        }

        // ---------- Queue badge --------------------------------------------
        if lay.queue_badge.w() > 0.5 {
            fill_with_border(
                &rt, &brush, lay.queue_badge.to_d2d(), 6.0, 1.0, p.chip, p.chip_border,
            );
            let counter = state.last_counter.borrow().clone();
            if let Some(fmt) = state.fmt_badge.borrow().as_ref() {
                if let Some(layout) = make_text_layout(
                    &counter,
                    fmt,
                    lay.queue_badge.w(),
                    lay.queue_badge.h(),
                ) {
                    brush.SetColor(&argb_to_color(p.title));
                    rt.DrawTextLayout(
                        D2D_POINT_2F {
                            x: lay.queue_badge.left,
                            y: lay.queue_badge.top,
                        },
                        &layout,
                        &brush,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                    );
                }
            }
        }

        // ---------- Message panel -------------------------------------------
        let panel = D2D1_ROUNDED_RECT {
            rect: lay.message_panel.to_d2d(),
            radiusX: PANEL_RADIUS,
            radiusY: PANEL_RADIUS,
        };
        brush.SetColor(&argb_to_color(p.panel));
        rt.FillRoundedRectangle(&panel, &brush);

        // Clip text to the message text area
        rt.PushAxisAlignedClip(&lay.message_text.to_d2d(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        if let Some(fmt) = state.fmt_body.borrow().as_ref() {
            if let Some(layout) = make_styled_layout(
                &state.parsed_message,
                fmt,
                lay.message_text.w(),
                99999.0,
            ) {
                let origin_x = lay.message_text.left;
                let origin_y = lay.message_text.top - state.scroll_y.get();

                // Paint code backgrounds BEFORE the text so glyphs sit on
                // top of the tint. Inline code: snug to the glyph extent.
                // Fenced blocks: extend each line to the full message
                // width so the block reads as a continuous panel.
                let parsed = &state.parsed_message;
                if !parsed.spans.is_empty() || !parsed.code_blocks.is_empty() {
                    brush.SetColor(&argb_to_color(p.code_bg));
                    for span in &parsed.spans {
                        if span.style == markdown::Style::InlineCode {
                            paint_text_range_bg(
                                &rt,
                                &layout,
                                &brush,
                                span.start,
                                span.len,
                                origin_x,
                                origin_y,
                                /* full_width_left */ None,
                                /* full_width_right */ None,
                                /* h_pad */ 2.0,
                                /* v_pad */ 0.0,
                            );
                        }
                    }
                    for cb in &parsed.code_blocks {
                        paint_text_range_bg(
                            &rt,
                            &layout,
                            &brush,
                            cb.start,
                            cb.len,
                            origin_x,
                            origin_y,
                            Some(lay.message_text.left),
                            Some(lay.message_text.right),
                            /* h_pad */ 0.0,
                            /* v_pad */ 2.0,
                        );
                    }
                }

                brush.SetColor(&argb_to_color(p.body));
                rt.DrawTextLayout(
                    D2D_POINT_2F { x: origin_x, y: origin_y },
                    &layout,
                    &brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        }
        rt.PopAxisAlignedClip();

        // Scrollbar thumb
        if lay.message_scroll_track.w() > 0.5 {
            let track_h = lay.message_scroll_track.h();
            let ratio = (lay.message_text.h() / lay.message_total_h).min(1.0);
            let thumb_h = (track_h * ratio).max(20.0);
            let scroll_max = (lay.message_total_h - lay.message_text.h()).max(1.0);
            let thumb_y = lay.message_scroll_track.top
                + (track_h - thumb_h) * (state.scroll_y.get() / scroll_max).clamp(0.0, 1.0);
            let thumb_rect = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: lay.message_scroll_track.left,
                    top: thumb_y,
                    right: lay.message_scroll_track.right,
                    bottom: thumb_y + thumb_h,
                },
                radiusX: 4.0,
                radiusY: 4.0,
            };
            brush.SetColor(&argb_to_color(p.scroll_thumb));
            rt.FillRoundedRectangle(&thumb_rect, &brush);
        }

        // ---------- Options --------------------------------------------------
        if !state.args.options.is_empty() {
            paint_options(&rt, &brush, state, &lay);
        }

        // ---------- Preview panel (preview mode) -----------------------------
        if state.mode == OptionMode::Preview && lay.preview_panel.w() > 0.5 {
            let prev_panel = D2D1_ROUNDED_RECT {
                rect: lay.preview_panel.to_d2d(),
                radiusX: OPT_RADIUS,
                radiusY: OPT_RADIUS,
            };
            brush.SetColor(&argb_to_color(p.panel));
            rt.FillRoundedRectangle(&prev_panel, &brush);

            let focus_idx = state.focus_opt.get().max(0) as usize;
            let preview_text = state
                .args
                .previews
                .get(focus_idx)
                .cloned()
                .unwrap_or_default();
            let inner = Rectf {
                left: lay.preview_panel.left + 14.0,
                top: lay.preview_panel.top + 12.0,
                right: lay.preview_panel.right - 14.0,
                bottom: lay.preview_panel.bottom - 12.0,
            };
            // Label
            if let Some(fmt) = state.fmt_optnum.borrow().as_ref() {
                let label = format!("PREVIEW · OPTION {}", focus_idx + 1);
                if let Some(layout) = make_text_layout(&label, fmt, inner.w(), 24.0) {
                    brush.SetColor(&argb_to_color(p.dim));
                    rt.DrawTextLayout(
                        D2D_POINT_2F { x: inner.left, y: inner.top },
                        &layout,
                        &brush,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                    );
                }
            }
            if let Some(fmt) = state.fmt_preview.borrow().as_ref() {
                if let Some(layout) =
                    make_text_layout(&preview_text, fmt, inner.w(), inner.h() - 24.0)
                {
                    brush.SetColor(&argb_to_color(p.body));
                    rt.PushAxisAlignedClip(
                        &Rectf {
                            left: inner.left,
                            top: inner.top + 22.0,
                            right: inner.right,
                            bottom: inner.bottom,
                        }
                        .to_d2d(),
                        D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    );
                    rt.DrawTextLayout(
                        D2D_POINT_2F {
                            x: inner.left,
                            y: inner.top + 22.0,
                        },
                        &layout,
                        &brush,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                    );
                    rt.PopAxisAlignedClip();
                }
            }
        }

        // ---------- Input field (custom-painted, no child window) ----------
        paint_input(&rt, &brush, state, &lay);

        // ---------- Footer (legend + dismiss) -------------------------------
        paint_footer(&rt, &brush, state, &lay);
    }

    let mut tag1: u64 = 0;
    let mut tag2: u64 = 0;
    let r = rt.EndDraw(Some(&mut tag1), Some(&mut tag2));
    if let Err(e) = r {
        if e.code() == D2DERR_RECREATE_TARGET {
            state.drop_d2d();
        }
    }

    let _ = EndPaint(hwnd, &ps);
}

unsafe fn paint_options(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    state: &WindowState,
    lay: &Layout,
) {
    let p = state.palette;
    let hover = state.hover_opt.get();
    let focus = state.focus_opt.get();
    let sel = state.selected.borrow();

    for (i, r) in lay.options.iter().enumerate() {
        let is_hover = hover == i as isize;
        let is_focus = focus == i as isize;
        let is_checked = sel.get(i).copied().unwrap_or(false);

        let rr = D2D1_ROUNDED_RECT {
            rect: r.to_d2d(),
            radiusX: OPT_RADIUS,
            radiusY: OPT_RADIUS,
        };

        match state.mode {
            OptionMode::Approve => {
                // Solid accent fill.
                brush.SetColor(&argb_to_color(p.accent));
                rt.FillRoundedRectangle(&rr, brush);
                if let Some(fmt) = state.fmt_option.borrow().as_ref() {
                    let label = state.args.options[i].clone();
                    if let Some(layout) = make_text_layout(&label, fmt, r.w(), r.h()) {
                        let _ = layout.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
                        let _ = layout.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
                        // Dark text on accent
                        brush.SetColor(&argb_to_color(0xFF_0D_0E_10));
                        rt.DrawTextLayout(
                            D2D_POINT_2F { x: r.left, y: r.top },
                            &layout,
                            brush,
                            D2D1_DRAW_TEXT_OPTIONS_NONE,
                        );
                    }
                }
                continue;
            }
            _ => {}
        }

        // Bordered fill (two-fill technique). Focused = accent border at
        // 2 px so it pops clearly; hover swaps the inner fill to the
        // option_hover tone.
        let fill = if is_hover { p.option_hover } else { p.option_bg };
        let (border_w, border_color) = if is_focus {
            (2.0, p.accent)
        } else {
            (1.0, p.option_border)
        };
        fill_with_border(rt, brush, r.to_d2d(), OPT_RADIUS, border_w, fill, border_color);

        // Inner content
        match state.mode {
            OptionMode::Multi => {
                let cx = r.left + OPT_PAD_X as f32;
                let cy = r.top + OPT_PAD_Y as f32;
                let box_rect = D2D_RECT_F {
                    left: cx,
                    top: cy,
                    right: cx + OPT_CHECK_SIZE as f32,
                    bottom: cy + OPT_CHECK_SIZE as f32,
                };
                let box_rr = D2D1_ROUNDED_RECT {
                    rect: box_rect,
                    radiusX: 4.0,
                    radiusY: 4.0,
                };
                if is_checked {
                    brush.SetColor(&argb_to_color(p.accent));
                    rt.FillRoundedRectangle(&box_rr, brush);
                    // Draw checkmark
                    brush.SetColor(&argb_to_color(0xFF_0D_0E_10));
                    let cw = OPT_CHECK_SIZE as f32;
                    let pts = [
                        D2D_POINT_2F {
                            x: cx + 0.22 * cw,
                            y: cy + 0.52 * cw,
                        },
                        D2D_POINT_2F {
                            x: cx + 0.42 * cw,
                            y: cy + 0.72 * cw,
                        },
                        D2D_POINT_2F {
                            x: cx + 0.78 * cw,
                            y: cy + 0.30 * cw,
                        },
                    ];
                    rt.DrawLine(pts[0], pts[1], brush, 1.7, None);
                    rt.DrawLine(pts[1], pts[2], brush, 1.7, None);
                } else {
                    brush.SetColor(&argb_to_color(p.option_border));
                    rt.DrawRoundedRectangle(&box_rr, brush, 1.5, None);
                }

                if let Some(fmt) = state.fmt_option.borrow().as_ref() {
                    let label = state.args.options[i].clone();
                    let label_x = cx + OPT_CHECK_SIZE as f32 + OPT_INNER_GAP as f32;
                    let label_w = r.right - label_x - OPT_PAD_X as f32;
                    if let Some(layout) = make_text_layout(&label, fmt, label_w, r.h()) {
                        brush.SetColor(&argb_to_color(p.body));
                        rt.DrawTextLayout(
                            D2D_POINT_2F {
                                x: label_x,
                                y: r.top + OPT_PAD_Y as f32 - 1.0,
                            },
                            &layout,
                            brush,
                            D2D1_DRAW_TEXT_OPTIONS_NONE,
                        );
                    }
                }
            }
            _ => {
                // Single / Preview
                if let Some(fmt) = state.fmt_optnum.borrow().as_ref() {
                    let num = format!("{}.", i + 1);
                    if let Some(layout) = make_text_layout(&num, fmt, OPT_NUM_W as f32, r.h()) {
                        brush.SetColor(&argb_to_color(p.option_number));
                        rt.DrawTextLayout(
                            D2D_POINT_2F {
                                x: r.left + OPT_PAD_X as f32,
                                y: r.top + OPT_PAD_Y as f32,
                            },
                            &layout,
                            brush,
                            D2D1_DRAW_TEXT_OPTIONS_NONE,
                        );
                    }
                }
                if let Some(fmt) = state.fmt_option.borrow().as_ref() {
                    let label = state.args.options[i].clone();
                    let label_x = r.left + OPT_PAD_X as f32 + OPT_NUM_W as f32 + OPT_INNER_GAP as f32;
                    let label_w = (r.right - label_x - OPT_PAD_X as f32).max(40.0);
                    if let Some(layout) = make_text_layout(&label, fmt, label_w, r.h()) {
                        brush.SetColor(&argb_to_color(p.body));
                        rt.DrawTextLayout(
                            D2D_POINT_2F {
                                x: label_x,
                                y: r.top + OPT_PAD_Y as f32 - 1.0,
                            },
                            &layout,
                            brush,
                            D2D1_DRAW_TEXT_OPTIONS_NONE,
                        );
                    }
                }
            }
        }
    }
}

/// Draws a bordered rounded rectangle using the two-fill technique:
/// outer rect filled in `border_argb`, inner rect (inset by `border_w` on
/// every side, radius reduced to match) filled in `fill_argb`. No stroke
/// involved — the visible border is the ring left between the two fills,
/// so it's pixel-perfect regardless of subpixel position or DPI.
unsafe fn fill_with_border(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    rect: D2D_RECT_F,
    radius: f32,
    border_w: f32,
    fill_argb: u32,
    border_argb: u32,
) {
    brush.SetColor(&argb_to_color(border_argb));
    let outer = D2D1_ROUNDED_RECT {
        rect,
        radiusX: radius,
        radiusY: radius,
    };
    rt.FillRoundedRectangle(&outer, brush);
    let inner_rect = D2D_RECT_F {
        left: rect.left + border_w,
        top: rect.top + border_w,
        right: rect.right - border_w,
        bottom: rect.bottom - border_w,
    };
    let inner = D2D1_ROUNDED_RECT {
        rect: inner_rect,
        radiusX: (radius - border_w).max(0.0),
        radiusY: (radius - border_w).max(0.0),
    };
    brush.SetColor(&argb_to_color(fill_argb));
    rt.FillRoundedRectangle(&inner, brush);
}

unsafe fn paint_input(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    state: &WindowState,
    lay: &Layout,
) {
    let p = state.palette;

    // Two-fill bordered input field. Focused = accent border at 2 px;
    // unfocused = input_border at 1 px.
    let (border_w, border_color) = if state.input_focused.get() {
        (2.0, p.accent)
    } else {
        (1.0, p.input_border)
    };
    fill_with_border(
        rt, brush, lay.input.to_d2d(), INPUT_RADIUS, border_w, p.input_bg, border_color,
    );

    // Inner text rect (where text/caret render). Clip to it so very long
    // single-line text doesn't escape the rounded corners.
    let text_rect = D2D_RECT_F {
        left: lay.input.left + INPUT_TEXT_PAD_X,
        top: lay.input.top + INPUT_TEXT_PAD_Y,
        right: lay.input.right - INPUT_TEXT_PAD_X,
        bottom: lay.input.bottom - INPUT_TEXT_PAD_Y,
    };
    rt.PushAxisAlignedClip(&text_rect, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);

    let text = state.input_text.borrow().clone();
    let inner_w = (text_rect.right - text_rect.left).max(40.0);
    let inner_h = (text_rect.bottom - text_rect.top).max(20.0);

    if text.is_empty() {
        // Placeholder — uses dim color so it doesn't shout. Only shown when
        // input is empty (i.e. caret at position 0 with no selection).
        if !state.args.placeholder.is_empty() {
            if let Some(layout) =
                build_input_layout(state, &state.args.placeholder, inner_w, inner_h)
            {
                brush.SetColor(&argb_to_color(p.dim));
                rt.DrawTextLayout(
                    D2D_POINT_2F { x: text_rect.left, y: text_rect.top },
                    &layout,
                    brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        }
    } else if let Some(layout) = build_input_layout(state, &text, inner_w, inner_h) {
        // Selection highlight (drawn under the text).
        if let Some((s, e)) = input_selection_range(state) {
            let s16 = utf16_pos_for_byte(&text, s);
            let e16 = utf16_pos_for_byte(&text, e);
            let length = e16 - s16;
            let mut count: u32 = 0;
            // First call: get required count.
            let _ = layout.HitTestTextRange(
                s16,
                length,
                text_rect.left,
                text_rect.top,
                None,
                &mut count,
            );
            if count > 0 {
                let mut metrics = vec![DWRITE_HIT_TEST_METRICS::default(); count as usize];
                let mut actual: u32 = 0;
                let _ = layout.HitTestTextRange(
                    s16,
                    length,
                    text_rect.left,
                    text_rect.top,
                    Some(&mut metrics),
                    &mut actual,
                );
                brush.SetColor(&argb_to_color(p.accent_soft));
                for m in &metrics[..actual as usize] {
                    let r = D2D_RECT_F {
                        left: m.left,
                        top: m.top,
                        right: m.left + m.width,
                        bottom: m.top + m.height,
                    };
                    rt.FillRectangle(&r, brush);
                }
            }
        }

        // The text itself.
        brush.SetColor(&argb_to_color(p.body));
        rt.DrawTextLayout(
            D2D_POINT_2F { x: text_rect.left, y: text_rect.top },
            &layout,
            brush,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
        );
    }

    // Caret — only when input is focused and the blink phase is on.
    if state.input_focused.get() && state.caret_on.get() {
        let cur = state.input_cursor.get();
        let cur16 = utf16_pos_for_byte(&text, cur);
        let layout = build_input_layout(state, &text, inner_w, inner_h);
        if let Some(layout) = layout {
            let mut cx = 0.0f32;
            let mut cy = 0.0f32;
            let mut m = DWRITE_HIT_TEST_METRICS::default();
            let _ = layout.HitTestTextPosition(cur16, false, &mut cx, &mut cy, &mut m);
            let caret_x = text_rect.left + cx;
            let caret_top = text_rect.top + cy;
            let caret_bot = caret_top + m.height.max(14.0);
            brush.SetColor(&argb_to_color(p.body));
            rt.DrawLine(
                D2D_POINT_2F { x: caret_x.round() + 0.5, y: caret_top },
                D2D_POINT_2F { x: caret_x.round() + 0.5, y: caret_bot },
                brush,
                1.0,
                None,
            );
        }
    }

    rt.PopAxisAlignedClip();
}

unsafe fn paint_footer(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    state: &WindowState,
    lay: &Layout,
) {
    let p = state.palette;

    // ---------- Enter legend ----------
    // Pip text is plain ASCII "Enter" to guarantee font coverage. The
    // unicode return symbol (U+23CE) isn't reliably in Cascadia Mono.
    let pip_top = lay.footer.top + (lay.footer.h() - PIP_H as f32) / 2.0 - 1.0;
    let enter_pip = D2D_RECT_F {
        left: lay.enter_legend.left,
        top: pip_top,
        right: lay.enter_legend.left + (PIP_W as f32 + 12.0),
        bottom: pip_top + PIP_H as f32,
    };
    paint_pip(rt, brush, p, enter_pip, "Enter", false);

    if let Some(fmt) = state.fmt_legend.borrow().as_ref() {
        if let Some(layout) = make_text_layout("to send", fmt, 240.0, lay.footer.h()) {
            let _ = layout.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            brush.SetColor(&argb_to_color(p.dim));
            rt.DrawTextLayout(
                D2D_POINT_2F {
                    x: enter_pip.right + 6.0,
                    y: lay.footer.top,
                },
                &layout,
                brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
    }

    // ---------- Dismiss cluster ----------
    let dismiss = state.dismiss.get();
    let now = Instant::now();
    let progress = dismiss_progress(dismiss, now);

    // Background highlight on hover.
    if state.hover_dismiss.get() {
        let bg = D2D1_ROUNDED_RECT {
            rect: lay.dismiss.to_d2d(),
            radiusX: 8.0,
            radiusY: 8.0,
        };
        brush.SetColor(&argb_to_color(p.chip));
        rt.FillRoundedRectangle(&bg, brush);
    }

    let pip1_armed =
        matches!(dismiss.phase, DismissPhase::Armed | DismissPhase::Done | DismissPhase::Timeout);
    let pip2_done = dismiss.phase == DismissPhase::Done;
    paint_pip(rt, brush, p, lay.dismiss_pip1.to_d2d(), "Esc", pip1_armed);
    paint_pip(rt, brush, p, lay.dismiss_pip2.to_d2d(), "Esc", pip2_done);

    // Progress bar
    if matches!(dismiss.phase, DismissPhase::Armed | DismissPhase::Timeout) {
        let track = lay.dismiss_progress.to_d2d();
        brush.SetColor(&argb_to_color(p.chip_border));
        let track_rr = D2D1_ROUNDED_RECT {
            rect: track,
            radiusX: 1.0,
            radiusY: 1.0,
        };
        rt.FillRoundedRectangle(&track_rr, brush);
        let bar_w = (track.right - track.left) * progress;
        if bar_w > 0.3 {
            let bar = D2D_RECT_F {
                left: track.left,
                top: track.top,
                right: track.left + bar_w,
                bottom: track.bottom,
            };
            brush.SetColor(&argb_to_color(p.accent));
            let bar_rr = D2D1_ROUNDED_RECT {
                rect: bar,
                radiusX: 1.0,
                radiusY: 1.0,
            };
            rt.FillRoundedRectangle(&bar_rr, brush);
        }
    }

    // Dismiss label
    if let Some(fmt) = state.fmt_legend.borrow().as_ref() {
        if let Some(layout) =
            make_text_layout("Dismiss", fmt, lay.dismiss_label.w(), lay.dismiss_label.h())
        {
            let _ = layout.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            brush.SetColor(&argb_to_color(p.dim));
            rt.DrawTextLayout(
                D2D_POINT_2F {
                    x: lay.dismiss_label.left,
                    y: lay.dismiss_label.top,
                },
                &layout,
                brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
    }
}

unsafe fn paint_pip(
    rt: &ID2D1RenderTarget,
    brush: &ID2D1SolidColorBrush,
    p: &Palette,
    rect: D2D_RECT_F,
    label: &str,
    is_armed: bool,
) {
    let rr = D2D1_ROUNDED_RECT {
        rect,
        radiusX: 4.0,
        radiusY: 4.0,
    };
    if is_armed {
        // Solid accent fill — no separate border needed since fill IS
        // the visible color.
        brush.SetColor(&argb_to_color(p.accent));
        rt.FillRoundedRectangle(&rr, brush);
    } else {
        fill_with_border(rt, brush, rect, 4.0, 1.0, p.chip, p.chip_border);
    }
    // bottom shadow line (the design uses border-bottom-width:2 to evoke a key)
    let bottom_line_y = rect.bottom - 1.0;
    rt.DrawLine(
        D2D_POINT_2F { x: rect.left + 2.0, y: bottom_line_y },
        D2D_POINT_2F { x: rect.right - 2.0, y: bottom_line_y },
        brush,
        1.0,
        None,
    );

    // Label text
    let fmt = create_text_format("Cascadia Mono", DWRITE_FONT_WEIGHT_REGULAR, 9.5);
    let _ = fmt.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
    let _ = fmt.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
    if let Some(layout) =
        make_text_layout(label, &fmt, rect.right - rect.left, rect.bottom - rect.top)
    {
        let txt_color = if is_armed { 0xFF_0D_0E_10 } else { p.dim };
        brush.SetColor(&argb_to_color(txt_color));
        rt.DrawTextLayout(
            D2D_POINT_2F { x: rect.left, y: rect.top },
            &layout,
            brush,
            D2D1_DRAW_TEXT_OPTIONS_NONE,
        );
    }
}

/// Compose a foreground color over an opaque background by alpha factor.
/// Useful for the grip dots that need to fade in/out without re-allocating
/// a brush per shade.
fn mix_alpha(argb: u32, alpha: f32) -> u32 {
    let a = (((argb >> 24) & 0xFF) as f32 * alpha).round() as u32;
    (a.clamp(0, 0xFF) << 24) | (argb & 0x00FF_FFFF)
}

// ===========================================================================
// Create the top-level window
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
    let x = -10000;
    let y = -10000;

    let title = wide(&(*state).args.title);

    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
        PCWSTR(class_name.as_ptr()),
        PCWSTR(title.as_ptr()),
        WS_POPUP,
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
    Ok(hwnd)
}

// ===========================================================================
// run_window: queue + show + collect outcome
// ===========================================================================
fn run_window(args: Args) -> Outcome {
    let pid = unsafe { GetCurrentProcessId() };
    let req_path = register_request(&args, pid);

    // Resolve palette + mode now.
    let persistent = load_state_from(&state_path());
    let palette = resolve_palette(&args, &persistent);
    let mode = if args.mode.is_empty() {
        if args.options.iter().any(|o| o.eq_ignore_ascii_case("Approve")) && args.options.len() == 1
        {
            OptionMode::Approve
        } else {
            OptionMode::Single
        }
    } else {
        parse_option_mode(&args.mode)
    };

    let initial_selected = vec![false; args.options.len()];

    let parsed_message = markdown::parse(&args.message);

    let state = Box::into_raw(Box::new(WindowState {
        args: args.clone(),
        palette,
        mode,
        parsed_message,
        req_path: req_path.clone(),
        outcome: RefCell::new(None),
        is_head_shown: Cell::new(false),
        last_counter: RefCell::new(String::new()),
        input_text: RefCell::new(String::new()),
        input_cursor: Cell::new(0),
        input_anchor: Cell::new(None),
        input_focused: Cell::new(true),
        caret_on: Cell::new(true),
        caret_moved_at: Cell::new(Instant::now()),
        input_selecting: Cell::new(false),
        drag_zone_bottom_y: Cell::new(0),
        window_size: Cell::new((POPUP_DEFAULT_W, POPUP_MIN_H)),
        selected: RefCell::new(initial_selected),
        hover_opt: Cell::new(-1),
        focus_opt: Cell::new(if args.options.is_empty() { -1 } else { 0 }),
        hover_dismiss: Cell::new(false),
        hover_top_row: Cell::new(false),
        scroll_y: Cell::new(0.0),
        dismiss: Cell::new(DismissState::default()),
        anim_active: Cell::new(false),
        render_target: RefCell::new(None),
        fmt_title: RefCell::new(None),
        fmt_body: RefCell::new(None),
        fmt_mono: RefCell::new(None),
        fmt_legend: RefCell::new(None),
        fmt_pip: RefCell::new(None),
        fmt_option: RefCell::new(None),
        fmt_optnum: RefCell::new(None),
        fmt_chip: RefCell::new(None),
        fmt_badge: RefCell::new(None),
        fmt_preview: RefCell::new(None),
        cached_layout: RefCell::new(None),
    }));

    unsafe {
        (*state).ensure_text_formats();
        let size = plan_window_size(&*state);
        (*state).window_size.set(size);
    }

    let outcome = unsafe {
        install_keyboard_hook();
        let _hwnd = match create_window(state) {
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
            // Enter / Shift+Enter / Ctrl+anything is handled inside the
            // wndproc now (handle_input_keydown). TranslateMessage produces
            // WM_CHAR events for printable keys; we don't use
            // IsDialogMessageW because there are no child controls left.
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        remove_keyboard_hook();
        let state = Box::from_raw(state);
        state.outcome.into_inner().unwrap_or(Outcome::Dismissed)
    };

    let _ = fs::remove_file(&req_path);
    outcome
}

unsafe fn on_enter(hwnd: HWND, state: &WindowState) {
    let typed = state.input_text.borrow().trim().to_string();
    if !typed.is_empty() {
        *state.outcome.borrow_mut() = Some(Outcome::Answered(typed));
        let _ = DestroyWindow(hwnd);
        return;
    }
    match state.mode {
        OptionMode::Multi => {
            let sel = state.selected.borrow();
            let chosen: Vec<String> = state
                .args
                .options
                .iter()
                .enumerate()
                .filter(|(i, _)| sel.get(*i).copied().unwrap_or(false))
                .map(|(_, label)| label.clone())
                .collect();
            if !chosen.is_empty() {
                let answer = chosen.join("\n");
                *state.outcome.borrow_mut() = Some(Outcome::Answered(answer));
                let _ = DestroyWindow(hwnd);
            }
        }
        OptionMode::Approve => {
            *state.outcome.borrow_mut() =
                Some(Outcome::Answered("Approve".into()));
            let _ = DestroyWindow(hwnd);
        }
        OptionMode::Single | OptionMode::Preview => {
            let idx = state.focus_opt.get();
            if idx >= 0 {
                if let Some(label) = state.args.options.get(idx as usize) {
                    *state.outcome.borrow_mut() = Some(Outcome::Answered(label.clone()));
                    let _ = DestroyWindow(hwnd);
                }
            }
        }
    }
}

// ===========================================================================
// main
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
            // Honor the on/off toggle (R9 / D1): when disabled, the hook
            // exits 0 silently with no decision JSON — Claude Code proceeds
            // through its normal flow with no interruption.
            let persistent = load_state_from(&state_path());
            if !persistent.enabled {
                std::process::exit(0);
            }
            let payload = read_stdin_payload();
            if event == HookEvent::Gate && should_skip_gate(&payload) {
                std::process::exit(0);
            }
            let args = derive_args(event, &payload);
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
// Tests
// ===========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    // ---- palette lookup ----
    #[test]
    fn palette_lookup_known_names() {
        for name in ["slate", "ocean", "amber", "forest", "plum", "default"] {
            assert_eq!(palette_by_name(name).name, name);
        }
    }

    #[test]
    fn palette_lookup_is_case_insensitive() {
        assert_eq!(palette_by_name("SLATE").name, "slate");
        assert_eq!(palette_by_name("PluM").name, "plum");
    }

    #[test]
    fn palette_lookup_falls_back_to_default() {
        assert_eq!(palette_by_name("nonsense").name, "default");
    }

    // ---- palette resolution from args + state ----
    #[test]
    fn resolve_palette_prefers_cli_flag() {
        let mut a = Args::default();
        a.palette = "ocean".into();
        a.project = "claude-integration".into();
        let mut s = PersistentState::default();
        s.palettes
            .insert("claude-integration".into(), "amber".into());
        assert_eq!(resolve_palette(&a, &s).name, "ocean");
    }

    #[test]
    fn resolve_palette_uses_project_mapping() {
        let mut a = Args::default();
        a.project = "claude-integration".into();
        let mut s = PersistentState::default();
        s.palettes
            .insert("claude-integration".into(), "amber".into());
        assert_eq!(resolve_palette(&a, &s).name, "amber");
    }

    #[test]
    fn resolve_palette_default_when_no_mapping() {
        let a = Args::default();
        let s = PersistentState::default();
        assert_eq!(resolve_palette(&a, &s).name, "default");
    }

    // ---- format_counter ----
    #[test]
    fn counter_empty_when_alone() {
        assert_eq!(format_counter(1, 1), "");
        assert_eq!(format_counter(1, 0), "");
    }

    #[test]
    fn counter_is_bare_number_when_queued() {
        assert_eq!(format_counter(1, 3), "3");
        assert_eq!(format_counter(2, 5), "5");
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
        let work = (1920, 0, 3840, 1080);
        let (x, y) = clamp_to_work(100, 100, work, 480, 280);
        assert_eq!((x, y), (1920, 100));
    }

    // ---- Position round-trip via the new schema ----
    #[test]
    fn position_roundtrip_through_file() {
        let dir = std::env::temp_dir().join(format!("fp-pos-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("state.json");
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

    #[test]
    fn state_enabled_defaults_to_true() {
        // Empty state file → enabled is implicit true.
        let st = serde_json::from_str::<PersistentState>("{}").unwrap();
        assert!(st.enabled);
    }

    #[test]
    fn state_enabled_roundtrip_false() {
        let dir = std::env::temp_dir().join(format!("fp-en-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("state.json");
        let mut s = PersistentState::default();
        s.enabled = false;
        save_state_to(&path, &s).unwrap();
        let after = load_state_from(&path);
        assert!(!after.enabled, "enabled=false must survive disk roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn state_enabled_persists_through_position_save() {
        // Toggling off, then dragging the window, must not silently
        // re-enable the system.
        let dir = std::env::temp_dir().join(format!("fp-enpos-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("state.json");
        let mut s = PersistentState::default();
        s.enabled = false;
        save_state_to(&path, &s).unwrap();
        save_position_to(&path, Position { x: 42, y: 99 }).unwrap();
        let after = load_state_from(&path);
        assert!(!after.enabled);
        assert_eq!(after.x, Some(42));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn state_preserves_palette_mapping_through_position_save() {
        let dir = std::env::temp_dir().join(format!("fp-state-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("state.json");
        let mut s = PersistentState::default();
        s.palettes.insert("alpha".into(), "ocean".into());
        s.palettes.insert("beta".into(), "amber".into());
        save_state_to(&path, &s).unwrap();
        save_position_to(&path, Position { x: 7, y: 9 }).unwrap();
        let after = load_state_from(&path);
        assert_eq!(after.palettes.get("alpha").map(|s| s.as_str()), Some("ocean"));
        assert_eq!(after.palettes.get("beta").map(|s| s.as_str()), Some("amber"));
        assert_eq!(after.x, Some(7));
        assert_eq!(after.y, Some(9));
        let _ = std::fs::remove_dir_all(&dir);
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
        assert!(a.placeholder.starts_with("Reply"));
    }

    #[test]
    fn derive_stop_prefers_last_assistant_message_over_transcript() {
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
    fn derive_stop_picks_project_from_cwd() {
        let p = serde_json::json!({"cwd": r"C:\Users\dev\my-cool-project"});
        let a = derive_args(HookEvent::Stop, &p);
        assert_eq!(a.project, "my-cool-project");
    }

    #[test]
    fn derive_stop_picks_session_hash_from_payload() {
        let p = serde_json::json!({"session_id": "0123456789abcdef"});
        let a = derive_args(HookEvent::Stop, &p);
        assert_eq!(a.session, "0123456");
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
        assert_eq!(a.mode, "single");
    }

    #[test]
    fn derive_question_picks_multi_when_multiselect_true() {
        let p = serde_json::json!({
            "tool_input": {
                "questions": [{
                    "question": "Pick any:",
                    "multiSelect": true,
                    "options": [{"label":"A"}, {"label":"B"}]
                }]
            }
        });
        let a = derive_args(HookEvent::Question, &p);
        assert_eq!(a.mode, "multi");
    }

    #[test]
    fn derive_question_picks_preview_when_any_option_has_preview() {
        let p = serde_json::json!({
            "tool_input": {
                "questions": [{
                    "question": "Pick a style:",
                    "options": [
                        {"label":"A","preview":"AAA"},
                        {"label":"B"}
                    ]
                }]
            }
        });
        let a = derive_args(HookEvent::Question, &p);
        assert_eq!(a.mode, "preview");
        assert_eq!(a.previews, vec!["AAA".to_string(), "".to_string()]);
    }

    #[test]
    fn derive_question_handles_exit_plan_mode_as_approve() {
        let p = serde_json::json!({"tool_name": "ExitPlanMode"});
        let a = derive_args(HookEvent::Question, &p);
        assert!(a.title.contains("Plan"));
        assert_eq!(a.options, vec!["Approve".to_string()]);
        assert_eq!(a.mode, "approve");
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

    #[test]
    fn decision_question_handles_multiline_multi_answer() {
        let d = build_decision_json(
            HookEvent::Question,
            &Outcome::Answered("A\nB\nC".into()),
        )
        .unwrap();
        let pr = d.pointer("/hookSpecificOutput/permissionDecisionReason");
        assert_eq!(pr.and_then(|v| v.as_str()), Some("A\nB\nC"));
    }

    // ---- Notification ----
    #[test]
    fn derive_notification_uses_payload_message() {
        let p = serde_json::json!({
            "hook_event_name": "Notification",
            "message": "Claude needs your permission to use Bash"
        });
        let a = derive_args(HookEvent::Notification, &p);
        assert_eq!(a.event, "Notification");
        assert_eq!(a.title, "Claude needs attention");
        assert_eq!(a.message, "Claude needs your permission to use Bash");
        assert!(a.options.is_empty());
    }

    #[test]
    fn derive_notification_falls_back_when_message_missing() {
        let p = serde_json::json!({});
        let a = derive_args(HookEvent::Notification, &p);
        assert!(!a.message.trim().is_empty(), "fallback message expected");
    }

    #[test]
    fn derive_notification_falls_back_when_message_whitespace() {
        let p = serde_json::json!({"message": "   "});
        let a = derive_args(HookEvent::Notification, &p);
        assert!(!a.message.trim().is_empty(), "fallback message expected");
    }

    #[test]
    fn decision_notification_is_always_none_dismiss() {
        let d = build_decision_json(HookEvent::Notification, &Outcome::Dismissed);
        assert!(d.is_none());
    }

    #[test]
    fn decision_notification_is_always_none_even_with_reply() {
        // Notification is informational — typed text must NOT influence the
        // underlying Claude Code notification flow.
        let d = build_decision_json(
            HookEvent::Notification,
            &Outcome::Answered("ignore me".into()),
        );
        assert!(d.is_none());
    }

    // ---- PermissionRequest ----
    #[test]
    fn derive_permission_shows_command_like_gate() {
        let p = serde_json::json!({
            "tool_name": "Bash",
            "tool_input": {"command": "rm -rf node_modules"}
        });
        let a = derive_args(HookEvent::Permission, &p);
        assert_eq!(a.event, "Permission");
        assert_eq!(a.title, "Permission needed");
        assert_eq!(a.message, "Run: rm -rf node_modules");
        assert_eq!(a.options, vec!["Allow".to_string(), "Deny".to_string()]);
    }

    #[test]
    fn derive_permission_falls_back_to_tool_name() {
        let p = serde_json::json!({"tool_name": "Write"});
        let a = derive_args(HookEvent::Permission, &p);
        assert_eq!(a.message, "Allow Write?");
    }

    #[test]
    fn decision_permission_allow_uses_decision_object() {
        let d = build_decision_json(HookEvent::Permission, &Outcome::Answered("Allow".into()))
            .unwrap();
        let evt = d.pointer("/hookSpecificOutput/hookEventName");
        assert_eq!(evt.and_then(|v| v.as_str()), Some("PermissionRequest"));
        let behavior = d.pointer("/hookSpecificOutput/decision/behavior");
        assert_eq!(behavior.and_then(|v| v.as_str()), Some("allow"));
        // Critical: no reason field anywhere on decision.
        assert!(d.pointer("/hookSpecificOutput/decision/reason").is_none());
        assert!(d
            .pointer("/hookSpecificOutput/permissionDecisionReason")
            .is_none());
    }

    #[test]
    fn decision_permission_deny_uses_decision_object() {
        let d = build_decision_json(HookEvent::Permission, &Outcome::Answered("Deny".into()))
            .unwrap();
        let behavior = d.pointer("/hookSpecificOutput/decision/behavior");
        assert_eq!(behavior.and_then(|v| v.as_str()), Some("deny"));
    }

    #[test]
    fn decision_permission_free_text_collapses_to_deny() {
        // PermissionRequest has no reason field — any non-Allow answer becomes
        // a plain `deny` and the text is dropped. The user's reasoning does
        // NOT reach Claude (this is a known limitation of the upstream schema).
        let d = build_decision_json(
            HookEvent::Permission,
            &Outcome::Answered("not safe right now".into()),
        )
        .unwrap();
        let behavior = d.pointer("/hookSpecificOutput/decision/behavior");
        assert_eq!(behavior.and_then(|v| v.as_str()), Some("deny"));
        let dump = serde_json::to_string(&d).unwrap();
        assert!(
            !dump.contains("not safe right now"),
            "free-text reason must not leak into Permission output: {}",
            dump
        );
    }

    #[test]
    fn decision_permission_dismiss_is_none() {
        let d = build_decision_json(HookEvent::Permission, &Outcome::Dismissed);
        assert!(d.is_none());
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

    // ---- option mode parsing ----
    #[test]
    fn option_mode_parses_known_modes() {
        assert_eq!(parse_option_mode("single"), OptionMode::Single);
        assert_eq!(parse_option_mode("multi"), OptionMode::Multi);
        assert_eq!(parse_option_mode("preview"), OptionMode::Preview);
        assert_eq!(parse_option_mode("approve"), OptionMode::Approve);
    }

    #[test]
    fn option_mode_falls_back_to_single() {
        assert_eq!(parse_option_mode(""), OptionMode::Single);
        assert_eq!(parse_option_mode("nonsense"), OptionMode::Single);
    }

    // ---- previews / options separator parsing ----
    #[test]
    fn split_pipe_handles_empty_and_whitespace() {
        assert!(split_pipe("").is_empty());
        assert_eq!(split_pipe("a|b|c"), vec!["a", "b", "c"]);
        assert_eq!(split_pipe("a | b "), vec!["a", "b"]);
    }

    #[test]
    fn split_comma_handles_empty_and_whitespace() {
        assert!(split_comma("").is_empty());
        assert_eq!(split_comma("a,b,c"), vec!["a", "b", "c"]);
        assert_eq!(split_comma(" a , b "), vec!["a", "b"]);
    }

    // ---- dismiss state machine ----
    fn make_state(phase: DismissPhase, t: Instant) -> DismissState {
        DismissState { phase, since: t }
    }

    #[test]
    fn dismiss_single_esc_arms() {
        let t = Instant::now();
        let (next, done) = advance_dismiss(make_state(DismissPhase::Rest, t), true, false, t);
        assert_eq!(next.phase, DismissPhase::Armed);
        assert!(!done);
    }

    #[test]
    fn dismiss_double_within_window_completes() {
        let t = Instant::now();
        let armed = make_state(DismissPhase::Armed, t);
        let (next, done) =
            advance_dismiss(armed, false, true, t + Duration::from_millis(200));
        assert_eq!(next.phase, DismissPhase::Done);
        assert!(done);
    }

    #[test]
    fn dismiss_armed_times_out_after_window() {
        let t = Instant::now();
        let armed = make_state(DismissPhase::Armed, t);
        let (next, done) = advance_dismiss(
            armed,
            false,
            false,
            t + Duration::from_millis(DISMISS_ARM_MS as u64 + 50),
        );
        assert_eq!(next.phase, DismissPhase::Timeout);
        assert!(!done);
    }

    #[test]
    fn dismiss_timeout_returns_to_rest() {
        let t = Instant::now();
        let to = make_state(DismissPhase::Timeout, t);
        let (next, _done) = advance_dismiss(
            to,
            false,
            false,
            t + Duration::from_millis(DISMISS_TIMEOUT_HOLD_MS as u64 + 50),
        );
        assert_eq!(next.phase, DismissPhase::Rest);
    }

    #[test]
    fn dismiss_progress_drains_during_armed() {
        let t = Instant::now();
        let armed = make_state(DismissPhase::Armed, t);
        let p_start = dismiss_progress(armed, t);
        let p_mid = dismiss_progress(armed, t + Duration::from_millis(300));
        let p_end =
            dismiss_progress(armed, t + Duration::from_millis(DISMISS_ARM_MS as u64));
        assert!(p_start > p_mid);
        assert!(p_mid > p_end);
        assert!((p_end - 0.0).abs() < 0.01);
    }

    // ---- queue path generation ----
    #[test]
    fn register_creates_file_with_pid_in_name() {
        let testdir = std::env::temp_dir().join(format!("fp-q-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&testdir);
        std::env::set_var("LOCALAPPDATA", &testdir);

        static FAKE_PID: AtomicU32 = AtomicU32::new(0);
        let pid = FAKE_PID.fetch_add(1, Ordering::SeqCst) + 99001;

        let mut args = Args::default();
        args.event = "Stop".into();
        args.title = "t".into();
        args.message = "m".into();
        args.options = vec!["A".into(), "B".into()];
        let path = register_request(&args, pid);
        assert!(path.exists(), "request file should exist at {:?}", path);
        assert_eq!(parse_pid(&path), Some(pid));

        let raw = std::fs::read_to_string(&path).unwrap();
        let parsed: Args = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed.event, "Stop");
        assert_eq!(parsed.options, vec!["A".to_string(), "B".to_string()]);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&testdir);
    }

    // ---- project basename ----
    #[test]
    fn project_basename_from_typical_path() {
        assert_eq!(
            project_from_cwd(r"C:\Users\dev\some-project"),
            "some-project"
        );
        assert_eq!(project_from_cwd("/home/dev/another"), "another");
    }

    // ---- markdown adapter ----
    use crate::markdown::{self, Style};

    fn span_at<'a>(out: &'a markdown::StyledText, style: Style) -> &'a markdown::Span {
        out.spans
            .iter()
            .find(|s| s.style == style)
            .unwrap_or_else(|| panic!("no span with style {:?} in {:?}", style, out.spans))
    }

    fn substr_utf16(text: &str, start: u32, len: u32) -> String {
        let units: Vec<u16> = text.encode_utf16().collect();
        String::from_utf16_lossy(&units[start as usize..(start + len) as usize])
    }

    #[test]
    fn markdown_plain_text_is_unchanged() {
        let out = markdown::parse("just plain text");
        assert_eq!(out.text, "just plain text");
        assert!(out.spans.is_empty());
        assert!(out.code_blocks.is_empty());
    }

    #[test]
    fn markdown_bold_strips_syntax_and_records_span() {
        let out = markdown::parse("hello **world** end");
        assert_eq!(out.text, "hello world end");
        let s = span_at(&out, Style::Bold);
        assert_eq!(substr_utf16(&out.text, s.start, s.len), "world");
    }

    #[test]
    fn markdown_italic_supports_both_markers() {
        let star = markdown::parse("a *one* b");
        assert_eq!(star.text, "a one b");
        let s = span_at(&star, Style::Italic);
        assert_eq!(substr_utf16(&star.text, s.start, s.len), "one");

        let under = markdown::parse("a _one_ b");
        assert_eq!(under.text, "a one b");
        let s = span_at(&under, Style::Italic);
        assert_eq!(substr_utf16(&under.text, s.start, s.len), "one");
    }

    #[test]
    fn markdown_inline_code_records_span_without_backticks() {
        let out = markdown::parse("use `foo()` here");
        assert_eq!(out.text, "use foo() here");
        let s = span_at(&out, Style::InlineCode);
        assert_eq!(substr_utf16(&out.text, s.start, s.len), "foo()");
    }

    #[test]
    fn markdown_fenced_code_block_records_range() {
        let out = markdown::parse("intro\n\n```\nfn main() {}\n```\n\nafter");
        assert!(out.text.contains("fn main() {}"));
        assert!(out.text.contains("intro"));
        assert!(out.text.contains("after"));
        assert!(!out.text.contains("```"));
        assert_eq!(out.code_blocks.len(), 1);
        let cb = out.code_blocks[0];
        assert_eq!(substr_utf16(&out.text, cb.start, cb.len), "fn main() {}");
    }

    #[test]
    fn markdown_fenced_code_block_preserves_internal_newlines() {
        let out = markdown::parse("```\nline1\nline2\n```");
        assert_eq!(out.code_blocks.len(), 1);
        let cb = out.code_blocks[0];
        assert_eq!(substr_utf16(&out.text, cb.start, cb.len), "line1\nline2");
    }

    #[test]
    fn markdown_unbalanced_delimiters_pass_through() {
        // pulldown-cmark treats a lone `*` as literal text.
        let out = markdown::parse("a * b c");
        assert_eq!(out.text, "a * b c");
        assert!(out.spans.is_empty());
    }

    #[test]
    fn markdown_escaped_chars_are_literal() {
        let out = markdown::parse(r"literal \*not italic\*");
        assert_eq!(out.text, "literal *not italic*");
        assert!(out.spans.is_empty());
    }

    #[test]
    fn markdown_mixed_bold_italic() {
        let out = markdown::parse("***both***");
        // pulldown-cmark parses *** as bold+italic nested.
        assert_eq!(out.text, "both");
        assert!(out.spans.iter().any(|s| s.style == Style::Bold));
        assert!(out.spans.iter().any(|s| s.style == Style::Italic));
    }

    #[test]
    fn markdown_paragraph_breaks_become_blank_lines() {
        let out = markdown::parse("first\n\nsecond");
        assert_eq!(out.text, "first\n\nsecond");
    }

    #[test]
    fn markdown_softbreak_becomes_space() {
        // A single \n in CommonMark is a soft break = space in output.
        let out = markdown::parse("line one\nline two");
        assert_eq!(out.text, "line one line two");
    }

    #[test]
    fn markdown_list_items_get_bullet_prefix() {
        let out = markdown::parse("- one\n- two\n- three");
        assert!(out.text.contains("• one"));
        assert!(out.text.contains("• two"));
        assert!(out.text.contains("• three"));
    }

    #[test]
    fn markdown_heading_becomes_bold_line() {
        let out = markdown::parse("# Title\n\nbody");
        assert!(out.text.starts_with("Title"));
        assert!(out.text.contains("body"));
        let s = span_at(&out, Style::Bold);
        assert_eq!(substr_utf16(&out.text, s.start, s.len), "Title");
    }

    #[test]
    fn markdown_link_text_passes_through_without_url() {
        let out = markdown::parse("see [the docs](https://example.com) for more");
        assert!(out.text.contains("the docs"));
        assert!(out.text.contains("for more"));
        assert!(!out.text.contains("https"));
        assert!(!out.text.contains("]("));
    }

    #[test]
    fn markdown_utf16_offsets_handle_supplementary_chars() {
        // 🦀 is a supplementary character: 2 UTF-16 code units, 4 UTF-8 bytes.
        let out = markdown::parse("**🦀 rust**");
        assert_eq!(out.text, "🦀 rust");
        let s = span_at(&out, Style::Bold);
        let units: Vec<u16> = out.text.encode_utf16().collect();
        // Span should cover all 7 UTF-16 units: 🦀 (2) + space (1) + "rust" (4)
        assert_eq!(s.start, 0);
        assert_eq!(s.len as usize, units.len());
    }
}
