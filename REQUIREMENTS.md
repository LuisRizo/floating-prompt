# Product Requirements — floating-prompt (Rust)

Single source of truth for what the Rust floating-prompt window must do.
Append-only spec (don't rewrite history — strike through obsolete clauses).

---

## R1 — Behavioral contract (existing, locked)

`floating-prompt.exe --event <Stop|Question|Gate> --title <s> --message <s> --options "A,B,C"`

- User answers (option button OR free text via Send/Enter) → answer text to stdout, exit code **0**.
- Double-Esc anywhere (focus-independent global hook) → no stdout, exit code **10**.
- Window starts topmost + no-activate (does not steal focus). Click activates it so the edit box can receive input.

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

## Deferred (planned, not yet specified)

These are intended future features. Captured here so they aren't lost, but
explicitly **out of scope for the current milestone**. Each will get its own
R-section with acceptance criteria when picked up.

- **D1 — Skill to toggle system on/off.** A user-invocable Claude Code skill
  (e.g. `/floating-prompt on`, `/floating-prompt off`) that flips a flag the
  `.exe` checks at hook-fire time. When off, the hook exits 0 immediately with
  no UI and no decision JSON. State persists in `state.json` alongside the
  window position.

- **D2 — Markdown / code rendering in the message body.** Today the message is
  drawn as a single wrapped text block. Need to render fenced code blocks
  (monospace, background tint) and basic inline markdown (bold, italic, inline
  `code`). Likely requires moving from raw `DrawTextW` to a richer control
  (RichEdit or Direct2D/DirectWrite). Affects `compute_window_size` /
  `measure_message_height`.

- **D3 — Numeric quick-select for options.** Each option button gets a 1-based
  index shown on the label (e.g. `1. Approve`, `2. Deny`). Typing the digit
  with focus in the textbox either:
  - immediately submits that option (if textbox is empty), OR
  - inserts the digit as a normal character (if textbox already has text).
  Goal: keyboard-only flow through multi-question Claude AskUserQuestion
  sequences without reaching for the mouse.

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
