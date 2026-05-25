# Product Requirements — floating-prompt (Rust)

Single source of truth for what the Rust floating-prompt window must do.
Append-only spec (don't rewrite history — strike through obsolete clauses).

---

## R1 — Behavioral contract (existing, locked)

`floating-prompt.exe --event <Stop|Question|Gate> --title <s> --message <s> --options "A,B,C"`

- User answers (option button OR free text via Send/Enter) → answer text to stdout, exit code **0**.
- Double-Esc anywhere (focus-independent global hook) → no stdout, exit code **10**.
- Window starts topmost + no-activate (does not steal focus). Click activates it so the edit box can receive input.

## R7 — Queue badge format (UPDATED)

Replaces the original R2.3 wording. The queue indicator in the top row is
now a **bare number** rendered as a small monospace badge, NOT the
"`X of Y`" text from v0.2.

- R7.1: When `total ≥ 2`, the badge shows just the total (e.g. `"3"`).
- R7.2: When `total ≤ 1`, no badge is rendered at all.
- R7.3: The position-within-queue (`X`) is intentionally not shown — the
  user only cares "how many more are waiting", not "which one am I."

## R2 — Multi-session queue (NEW)

Multiple concurrent Claude Code sessions can hit hooks at the same time. Each hook
invocation spawns its own `floating-prompt.exe`, but the **user sees only ONE window
at a time**, with the rest queued in FIFO order.

**Acceptance criteria:**
- R2.1: At most one window is visible across all concurrent invocations.
- R2.2: When session A's window is dismissed/answered, session B's request is shown
  **immediately** (no perceptible gap, < 200 ms).
- R2.3: The window shows the queue position visually, e.g. `"1 of 3"` in the
  title row. The counter updates live as new requests arrive or are answered.
  When N = 1, the counter is hidden (no `"1 of 1"` noise).
- R2.4: Each spawned `.exe` returns its OWN answer to its OWN parent (launch.ps1
  process for that session). Cross-wiring answers between sessions is a critical bug.
- R2.5: When the queue drains, the coordinator process exits cleanly. A new
  invocation arriving later starts a fresh coordinator.

**Implementation note (informational, not normative):** the first `.exe` to start
becomes a *coordinator* that owns the window and listens on a named pipe. Subsequent
`.exe` invocations become *clients* that send their request over the pipe and block
on the response. This is an implementation detail — the external contract (R1) is
identical for clients and coordinators.

## R3 — Position persistence (NEW)

The user's last manual window position is remembered across all future invocations
and across coordinator restarts.

**Acceptance criteria:**
- R3.1: When the user drags the window to a new position and the drag completes,
  the new (x, y) is saved to `%LOCALAPPDATA%\floating-prompt\state.json`.
- R3.2: On window creation, if `state.json` exists and contains a valid (x, y),
  the window is placed there.
- R3.3: If no state file exists, the window defaults to bottom-right of the
  primary work area (with a 16 px margin), preserving R1 behavior for first-time
  users.
- R3.4: If the saved position would place the window fully off-screen (monitor
  layout changed since last save), clamp to the nearest visible work area.
- R3.5: Position is saved per-machine, per-user (in `%LOCALAPPDATA%`, not roamed).

## R4 — Tests

Both R2 and R3 ship with tests:

- R4.1: Rust unit tests for the position load/save round-trip, default fallback,
  and clamp-to-work-area logic (`cargo test`).
- R4.2: Rust unit tests for queue counter formatting (`""`, `"1 of 2"`, etc.).
- R4.3: A PowerShell harness (`tests/run-tests.ps1`) that:
  - Spawns multiple `.exe` instances in parallel with redirected stdout.
  - Documents the human interaction required at each step.
  - Asserts the observable contract afterwards (per-process exit codes,
    per-process stdout, state.json contents).

---

## R9 — Global enable / disable toggle (NEW)

The whole popup system can be toggled off without uninstalling the hooks
or editing `~/.claude/settings.json`.

- R9.1: An `enabled: bool` field in `state.json` (default `true` if
  missing — back-compat with v0.2 state files).
- R9.2: When `enabled == false`, the hook-mode entry point
  (`floating-prompt.exe --hook ...`) exits 0 silently with no UI and no
  decision JSON — Claude Code proceeds normally as if no hook ran.
- R9.3: CLI-mode invocations (`floating-prompt.exe --message ...`) ignore
  the flag — the user explicitly asked for a window.
- R9.4: The toggle is applied at the next hook fire; Claude Code does NOT
  need a restart (hot-reload via `state.json`).
- R9.5: Managed by the `/floating-prompt on` and `/floating-prompt off`
  skill subcommands. `/floating-prompt status` reports the current state.

## R5 — Per-project color palette (NEW)

The popup has a small palette family. Each project (= last segment of the
hook payload's `cwd`) can be mapped to one palette; otherwise the popup uses
`default`.

- R5.1: Six built-in palettes: `slate`, `ocean`, `amber`, `forest`,
  `plum`, `default`. Each defines a complete slot schema (bg, panel, chip,
  chip_border, accent, accent_soft, option_bg, option_hover, option_border,
  option_number, input_bg, input_border, body, title, dim, scroll_thumb).
- R5.2: Mapping persists in `state.json` under the `palettes` object:
  `{ "palettes": { "<project>": "<palette-name>" } }`.
- R5.3: Resolution order at popup launch:
  `--palette <name>` flag > `state.json[palettes][project]` > `default`.
- R5.4: Unknown palette names fall back to `default` (no crash, no popup
  about config errors).
- R5.5: A Claude Code skill at `skills/floating-prompt/SKILL.md` lets the
  user manage the mapping via `/floating-prompt palette <name>` — no exe
  code is needed for the write.

## R6 — Option modes (NEW)

The popup supports four option-interaction modes; one mode per invocation,
driven by `--mode <single|multi|preview|approve>`.

- R6.1: **Single** (default). Click an option = immediate submit with that
  label. Numeric `1.` / `2.` / `3.` prefix on each label as a visual cue.
- R6.2: **Multi**. Each option has a checkbox. Clicks toggle. Submit
  (Enter on empty input) returns the selected labels joined by `\n`.
- R6.3: **Preview**. Single-select, but each option has an associated
  preview (parallel `--previews "A|B|C"` separator is `|` because previews
  may contain `,`). Layout becomes two-column: options left, focused
  preview right.
- R6.4: **Approve**. The plan/approve UX: exactly one option, painted as
  a full-width accent-filled primary button. Click submits `"Approve"`.
  ExitPlanMode-derived prompts default to this mode automatically.
- R6.5: For Multi mode, an empty selection + empty input + Enter does
  nothing (no accidental empty submit). Esc-Esc still dismisses.
- R6.6: Mode is auto-derived in hook mode:
  - `ExitPlanMode` → Approve
  - AskUserQuestion `multiSelect:true` → Multi
  - any option with a non-empty `preview` field → Preview
  - else → Single

## R8 — Animated dismiss control (NEW)

The dismiss UX is both a keyboard legend AND a clickable target.

- R8.1: Visual: two adjacent pips reading `Esc` `Esc`, plus a faint
  `Dismiss` label, in the footer-right.
- R8.2: State machine: `Rest` → (Esc / click) → `Armed` → (second Esc /
  click within 600 ms) → `Done` (dismiss). If no second event within
  600 ms → `Timeout` (held ~250 ms) → back to `Rest`.
- R8.3: When `Armed`, a thin progress bar under the pips drains from
  full → empty over the 600 ms window. `Timeout` paints the bar nearly
  empty for the hold period.
- R8.4: Clicking the dismiss cluster behaves identically to pressing
  Esc — the same state machine drives it, so the user can confirm a
  dismiss with two clicks just like two Esc presses.
- R8.5: The double-Esc keyboard hook (R1) is still global / focus-
  independent.

## R10 — Markdown rendering in the message body (NEW)

The message body renders a CommonMark subset rather than raw text. Parsing is
done by `pulldown-cmark` (default features off); style application is via
per-range `IDWriteTextLayout::SetFontWeight` / `SetFontStyle` /
`SetFontFamilyName`. Code backgrounds are painted as filled rects under the
glyphs using `HitTestTextRange`.

- R10.1: **Inline styling** — `**bold**`, `*italic*`, `_italic_`, and
  `` `inline code` `` are rendered with the corresponding font weight, style,
  and family (Cascadia Mono for inline code). Syntax characters are stripped
  from the rendered text.
- R10.2: **Fenced code blocks** (``` ``` ```) render in Cascadia Mono with the
  palette's `code_bg` slot painted full-width on each line.
- R10.3: **Headings** (`#`, `##`, …) render as bold lines. Their text is
  preserved; the `#` markers are stripped.
- R10.4: **Lists** (`-`, `*`, `1.`) render with a `• ` prefix per item;
  numbering is dropped (out of scope for this pass).
- R10.5: **Horizontal rules** (`---`) render as a thin Unicode divider.
- R10.6: **Soft breaks** become a single space (CommonMark behavior). Hard
  breaks become `\n`. Paragraph breaks become a blank line.
- R10.7: **Out of scope (passthrough — inner text only, no styling):** links
  (URL dropped), images, blockquotes, tables, footnotes, raw HTML (dropped).
- R10.8: **Offsets are UTF-16 code units** to match DirectWrite's index
  space. Adapter tests verify supplementary characters (e.g. emoji).
- R10.9: The parsed `StyledText` is cached on `WindowState` at launch — the
  message never changes after, so per-`WM_PAINT` reparsing would be wasted
  work. The layout itself is rebuilt per measurement / paint because it
  depends on width.

Replaces deferred item D2 below.

## Deferred (planned, not yet specified)

These are intended future features. Captured here so they aren't lost, but
explicitly **out of scope for the current milestone**. Each will get its own
R-section with acceptance criteria when picked up.

- ~~**D2 — Markdown / code rendering in the message body.**~~ Shipped — see
  R10. Markdown subset (bold, italic, inline code, fenced code, headings,
  lists, rules) renders via pulldown-cmark + per-range IDWriteTextLayout.

- **D3 (PARTIAL) — Numeric quick-select for options.** The `1.` / `2.` /
  `3.` prefix on Single / Preview options is now rendered (visual cue
  from R6.1). The "type the digit on empty input → submit that option"
  keyboard handling is still deferred.

- **D4 — Slightly transparent window.** Apply per-pixel alpha (`WS_EX_LAYERED`
  + `SetLayeredWindowAttributes`) so the window is ~90% opaque. Configurable
  via `state.json`. Must not break click-to-activate or drag (HTCAPTION).

- **D5 — Local voice dictation with hotkey.** Push a global hotkey (e.g.
  `Ctrl+Shift+Space`) to start local speech-to-text capture; press again to
  stop, transcribe, and inject the text into the prompt's edit box (user can
  then edit and Send/Enter). Must run fully offline (no cloud STT). Likely
  whisper.cpp or Windows Speech Recognition. Microphone access is on-demand,
  not persistent.

---

## Anti-requirements (explicitly out of scope, for now)

- Cross-machine queueing (named pipe is per-machine — that's fine).
- Window size persistence (only position).
- Per-Claude-session colored chrome or session names in the queue counter
  (the counter is just N-of-M, not "Claude session abc123 of 3").
- True in-tool AskUserQuestion answering — still requires the headless `-p`
  wrapper described in HANDOFF.md §8 #5.
