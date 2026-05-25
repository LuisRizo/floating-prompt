# floating-prompt

A Windows-native floating popup for Claude Code hooks. When a turn ends or
Claude needs something from the user (`AskUserQuestion`, `ExitPlanMode`, a
Bash gate, etc.), this popup appears bottom-right of the screen, holds the
turn open until the user replies or dismisses with double-Esc, then injects
the answer back into the conversation.

Pure Win32 + Direct2D + DirectWrite. Every pixel is custom-painted —
there are no standard Windows controls inside the popup — so the visual
follows a designed style rather than the default Win32 chrome.

## Features

- **All hook events:** Stop, AskUserQuestion (single / multi / preview),
  ExitPlanMode (Approve), PreToolUse gates (Allow / Deny / explain),
  PermissionRequest (the targeted "Claude needs permission for tool X"
  signal — narrower than PreToolUse matchers, fires only when auto-mode
  bailed), and Notification (idle-wait alerts + other catch-all signals).
- **Multi-session queue:** concurrent Claude Code sessions queue rather
  than stack — one popup visible at a time, FIFO.
- **Focus-independent double-Esc** via a global `WH_KEYBOARD_LL` hook —
  works even when the popup isn't focused.
- **Per-project color palettes** (slate, ocean, amber, forest, plum,
  default), managed through the bundled `/floating-prompt` skill.
- **Position persistence:** drag the window and it stays where you put
  it across invocations (clamped if your monitor layout changes).
- **Custom text input** with full keyboard shortcuts: Ctrl+arrows for
  word jump, Ctrl+Backspace/Delete for word delete, Ctrl+A/C/X/V, Home /
  End line-bounds, Shift+Enter for newline, click-drag selection.
- **Animated dismiss legend** in the footer; the `Esc Esc` pips arm on
  the first press, drain over the 600 ms window, then reset.
- **Global on/off toggle** via `/floating-prompt off` — hooks become
  no-ops without uninstalling them or editing `settings.json`.
- **Markdown rendering** in the message body — `**bold**`, `*italic*`,
  `` `inline code` ``, fenced code blocks (mono + tinted bg), headings,
  lists, and horizontal rules. Parsed with `pulldown-cmark`; styles applied
  per-range to a single `IDWriteTextLayout`.

## Install

Installs as a standard Claude Code plugin — the plugin system reads
[`hooks.json`](hooks.json) and wires the 4 hook events automatically. No
`settings.json` editing.

Requires the Rust toolchain (get it from [rustup.rs](https://rustup.rs/) if
missing).

**Step 1** — build the binary:

```powershell
.\build.ps1
```

This runs `cargo build --release` and copies the resulting `.exe` to
`./hooks/floating-prompt.exe`, where `${CLAUDE_PLUGIN_ROOT}` will find it
after the plugin install.

**Step 2** — install the plugin from inside Claude Code:

```
/plugin marketplace add <absolute-path-to-this-repo>
/plugin install floating-prompt@floating-prompt-marketplace
```

The two slash commands register the local marketplace
([`marketplace.json`](marketplace.json)) and install the plugin
([`plugin.json`](plugin.json) + [`hooks.json`](hooks.json)). Hooks are
hot-loaded — no Claude Code restart needed.

Verify with `/hooks` and `/plugin list`.

To remove: `/plugin uninstall floating-prompt@floating-prompt-marketplace`
unwires every hook the plugin registered. Run `.\build.ps1` again any time
to rebuild after a source change, then `/plugin update floating-prompt` (or
reinstall) to pick up the new binary.

<details>
<summary>Manual install (no plugin system)</summary>

For setups where the plugin system isn't usable (legacy installs, shared
machines, project-scoped `settings.json`), the four hook blocks can be
merged manually instead:

```powershell
cargo build --release
copy target\release\floating-prompt.exe "$env:USERPROFILE\.claude\hooks\"
New-Item -ItemType Directory -Force "$env:USERPROFILE\.claude\skills\floating-prompt"
copy skills\floating-prompt\SKILL.md "$env:USERPROFILE\.claude\skills\floating-prompt\"
```

Then merge the contents of [`settings.fragment.json`](settings.fragment.json)
into `~/.claude/settings.json`, replacing `<you>` with your Windows
username (`$env:` doesn't expand inside JSON).

</details>

## Usage

After install the popup appears automatically on every Stop /
AskUserQuestion / ExitPlanMode. Nothing to do day-to-day.

### Configure per-project palette

```
/floating-prompt palette ocean      # set current project's palette
/floating-prompt palettes           # list all six names
/floating-prompt status             # report on/off + mapping count
/floating-prompt off                # disable globally (hooks no-op)
/floating-prompt on                 # re-enable
/floating-prompt show               # dump state.json
```

Mapping is stored in `%LOCALAPPDATA%\floating-prompt\state.json` under
`palettes`, keyed by `basename(cwd)`. Picked up on the next hook fire.

### Standalone CLI mode

Launch the popup directly (useful for ad-hoc prompts in scripts):

```powershell
.\target\release\floating-prompt.exe `
    --message "Pick one" `
    --options "Yes,No,Maybe" `
    --mode single `
    --placeholder "or type your own"
```

Prints the chosen option (or typed text) on stdout, exit 0 — or no
output and exit 10 if dismissed.

Flags:

| Flag | Purpose |
|---|---|
| `--message <s>` | Body text |
| `--options "A,B,C"` | Option labels (`,`-separated) |
| `--previews "A\|B\|C"` | Preview text per option (`\|`-separated, used only with `--mode preview`) |
| `--mode single\|multi\|preview\|approve` | Option behavior (default `single`) |
| `--placeholder <s>` | Input field placeholder text |
| `--palette <name>` | Force palette regardless of project mapping |
| `--project <s>` | Project name for palette lookup (defaults to `basename(cwd)`) |
| `--session <s>` | Session hash shown in the chip |

### Hook mode

When invoked with `--hook Stop|Question|Gate|Permission|Notification`, the
binary reads the Claude Code hook JSON from stdin, derives all args (event,
message, options, project, session) automatically, shows the popup, and emits
the appropriate decision JSON on stdout:

| Event | Outcome | Output |
|---|---|---|
| Stop | answered | `{"decision":"block","reason":"<answer>"}` (Claude gets another turn) |
| Stop | dismissed | _no output_ (turn ends) |
| Question | answered | `permissionDecision: deny` + the answer as the reason |
| Gate | `Allow` | `permissionDecision: allow` (PreToolUse output shape) |
| Gate | `Deny` | `permissionDecision: deny` |
| Gate | text | `permissionDecision: deny` + the text as the reason |
| Permission | `Allow` | `hookSpecificOutput.decision.behavior: allow` (PermissionRequest output shape — distinct from Gate) |
| Permission | `Deny` or text | `hookSpecificOutput.decision.behavior: deny`. **Free-text reason is dropped** — PermissionRequest has no reason field |
| Notification | any | _no output_ (the popup is purely informational — Claude Code's notification proceeds unchanged) |

**Permission vs Gate** — both surface as the same Allow/Deny popup, but they
hook different Claude Code events with different output schemas:
- `Permission` (PermissionRequest) fires *only* when auto-mode couldn't
  decide and the user would have seen the built-in permission prompt.
  Narrow trigger; clean output. Use this as your default permission UX.
- `Gate` (PreToolUse with a `matcher`) fires for *every* matched tool call
  — including auto-allowed ones — so it can be noisy. Use only when you
  want a hook to weigh in on every call to a specific tool, OR when you
  need to round-trip a free-text reason back to Claude.

## Build & test

```powershell
cargo build --release         # → target\release\floating-prompt.exe
cargo test --release          # 81 unit tests (55 core + 15 markdown + 5 notification + 6 permission)
.\tests\smoke-ui.ps1          # spawns + screenshots every canonical state
                              # (sandboxes %LOCALAPPDATA%, doesn't
                              #  interfere with real Claude sessions)
```

Requires the Rust toolchain on the MSVC target (default on Windows).
Builds against `windows` crate 0.52.

## Architecture notes

- **No child windows.** The popup is a single `WS_POPUP` whose entire
  client area is owned by a Direct2D `HwndRenderTarget`. The original
  design used an `EDIT` child for text input; this caused permanent
  flicker because D2D's `Present()` always blits its back buffer to the
  HWND surface, overwriting any child-window pixels. The fix was to
  remove the child and paint the input ourselves (caret, selection,
  clipboard, the lot).

- **Two-fill bordered shapes.** Rounded rectangles with visible borders
  use the two-fill technique: outer rect filled in border color, inner
  rect 1 px smaller in fill color. No stroke involved — D2D's
  center-aligned 1 px stroke half-aliases across two pixel rows at
  typical Windows DPI, leaving borders nearly invisible. Two-fill gives
  a pixel-perfect 1 px border regardless of subpixel position.

- **Multi-session queue** via a directory of request files at
  `%LOCALAPPDATA%\floating-prompt\queue\<millis>-<pid>.req.json`. The
  earliest-arriving process owns the visible window; later arrivals show
  their popup when their turn comes up. No named pipes / IPC — just
  filesystem polling.

- **Per-project palettes.** Six embedded `Palette` structs, 17 color
  slots each. Resolution at popup launch:
  `--palette` flag → `state.json["palettes"][project]` → `default`.

- **Markdown rendering.** `pulldown-cmark` parses the message into a
  cleaned `StyledText { text, spans, code_blocks }` (UTF-16 offsets, the
  index space DirectWrite uses). One `IDWriteTextLayout` carries the whole
  message; `SetFontWeight` / `SetFontStyle` / `SetFontFamilyName` apply
  per-range styles after creation. Code backgrounds are filled rects under
  the glyphs, computed via `HitTestTextRange`. The parsed result lives on
  `WindowState` so a paint doesn't re-parse.

- **Global keyboard hook.** `WH_KEYBOARD_LL` sets thread-local atomic
  flags for Esc presses; the UI thread polls them from a 30 ms timer so
  dismissal works even when the popup isn't focused.

## Layout

```
main.rs                          single-file Rust binary (~5 KLOC)
Cargo.toml                       windows 0.52 + serde_json + pulldown-cmark
plugin.json / hooks.json         Claude Code plugin manifest + hook bindings
marketplace.json                 single-plugin marketplace declaration
build.ps1                        cargo build + place exe at hooks/
settings.fragment.json           hook block reference (for manual install)
REQUIREMENTS.md                  R1-R9 spec + deferred items
DESIGN-BRIEF.md                  historical design brief for the redesign
skills/floating-prompt/SKILL.md  bundled /floating-prompt skill
design/                          design source assets (jsx, html, palettes.js)
tests/smoke-ui.ps1               sandboxed live-screenshot harness
```
