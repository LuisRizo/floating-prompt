# HANDOFF — Floating Prompt for Claude Code (Windows)

Context dump for continuing local implementation + testing in a Claude Code
session. Read this top to bottom once; it captures the whole arc, the working
contract, every file, what's verified vs not, and the exact next steps.

---

## 1. What we're building

A Superwhisper-style floating window for Claude Code on **Windows**. When a
Claude Code turn ends (or the agent needs input), a small always-on-top window
appears. The turn **hangs** until the user either:

- **answers** in the window (text or option) -> the answer is injected back so
  Claude continues, OR
- **double-presses Esc anywhere** (focus-independent) -> the turn is allowed to
  end.

Branding-free. No speech-to-text. Windows-only (no cross-platform requirement).

There are TWO implementations of the window:
1. **PowerShell + WPF** — works today, tested live by the user. This is the
   reference / source of truth for behavior.
2. **Rust + `windows` crate (Win32)** — a SHELL only, not yet compiled. Meant to
   eventually replace the PowerShell window. Design (real controls) not done.

---

## 2. THE CONTRACT (do not break this)

Both window implementations and the hook glue agree on one contract:

```
window invoked with: --title <s> --message <s> --options "A,B,C" (--event <e>)
  user answers   -> print answer to STDOUT, exit code 0
  user dismisses -> print nothing,          exit code 10
```

`launch.ps1` (the hook glue) reads that and emits the Claude Code hook JSON:

- **Stop event**, answered  -> `{"decision":"block","reason":"<answer>"}` exit 0
  (Claude does NOT stop; gets another turn with `<answer>` as the instruction)
- **Stop event**, dismissed -> no output, exit 0  (turn ends normally)
- **PreToolUse (Gate)**, answered Allow/Deny/text ->
  `{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow|deny","permissionDecisionReason":"..."}}`
- **PreToolUse (Question)**, answered -> deny + reason = the user's text.

---

## 3. HOW THE HANG WORKS (the key mechanism)

- Hooks run **synchronously**. A synchronous Stop hook blocks Claude Code the
  entire time it runs.
- The window's blocking show (`ShowDialog()` in WPF / the message loop in Rust)
  holds the hook process open -> the turn is frozen until the window closes.
- Answer -> emit `decision:block` JSON -> Claude continues with the answer.
- Dismiss (double-Esc) -> exit 0 no JSON -> turn proceeds/ends.

**Double-Esc is focus-independent** via a global `WH_KEYBOARD_LL` keyboard hook
(two Esc within 600 ms). It must work without the window having focus.

---

## 4. HARD-WON LESSONS (mistakes already made + fixed — don't repeat)

1. **No async on the hooks.** `async:true` makes the turn NOT wait. Must be
   synchronous to hang. (Long `timeout`, e.g. 3600, so the window can sit open.)

2. **Do NOT bail on `stop_hook_active`.** Earlier code treated `stop_hook_active==true`
   as "infinite loop, exit". WRONG: that flag is true on EVERY Stop following a
   blocked Stop, i.e. our normal conversational case. The bail made the window
   appear only ONCE then go silent forever. FIX: removed the guard entirely. The
   human is the loop terminator (we never auto-inject; blocking only happens when
   the user types). This was the user's reported "only works for a single loop"
   bug. Already fixed in current launch.ps1.

3. **`PermissionRequest` deny is unreliable.** It's a valid event but there's a
   known Claude Code bug where its deny decision is ignored and the prompt shows
   anyway. So we do ALL permission gating through **`PreToolUse`** returning
   `permissionDecision` (allow/deny/ask), which is honored. `PermissionRequest`
   was removed.

4. **`${CLAUDE_PROJECT_DIR}` does NOT expand in global `~/.claude/settings.json`.**
   Use absolute paths there (e.g. `C:\Users\<name>\.claude\hooks\launch.ps1`).
   `${CLAUDE_PLUGIN_ROOT}` DOES expand for plugin installs. `${CLAUDE_PROJECT_DIR}`
   works in PROJECT `.claude/settings.json`.

5. **"Stop hook error: <text>" label is COSMETIC.** Claude Code prefixes blocked-
   Stop feedback with "error" even when the block is intentional and working.
   Not a bug in our code. (There's also a separate known bug where plugin-installed
   Stop hooks that block via EXIT CODE 2 halt instead of continue — so we use the
   JSON `decision:"block"` + exit 0 form, NOT exit code 2, and prefer the
   `.claude/settings.json` install over the plugin install for blocking hooks.)

6. **No em-dashes / non-ASCII in .ps1 files.** They break depending on save
   encoding. Both scripts are currently pure ASCII. Keep them that way. Save
   .ps1 as UTF-8.

---

## 5. CLAUDE CODE HOOK FACTS (verified against docs this session)

- Hook input arrives as JSON on **stdin**. Common fields: `session_id`,
  `transcript_path`, `cwd`, `permission_mode`, `hook_event_name`,
  `stop_hook_active` (on Stop).
- Output: exit 0 + JSON on stdout for structured control; OR exit 2 to block via
  stderr. **Pick one, not both.** We use exit-0-JSON.
- `decision:"block"` + `reason` on **Stop** = continue the conversation, reason
  injected. (Top-level `decision`, not hookSpecificOutput, for Stop.)
- **PreToolUse** uses `hookSpecificOutput.permissionDecision` =
  allow|deny|ask|defer + `permissionDecisionReason`.
- `AskUserQuestion` DOES fire PreToolUse. In an INTERACTIVE session you can't
  fill the tool's own answer field; you can only deny + feed text back. TRUE
  in-tool answering requires headless `-p` mode with PreToolUse defer->allow
  round-trip (NOT built; future work).
- Hook config `shell:"powershell"` runs the command via PowerShell directly on
  Windows. Hooks are picked up on edit (script edits need no restart; settings
  JSON changes are watched too, but a restart is safest).
- Verify hooks with `/hooks` inside Claude Code. Debug with `claude --debug`.
- Matcher syntax: plain `A|B` = exact tool names; regex if other chars present.
  `if:"Bash(rm *)"` permission-rule syntax narrows further (good for the Gate).

---

## 6. FILE INVENTORY

All paths below are what was produced. The **live, working** set is `v2/`.

```
v2/                              <- CURRENT WORKING VERSION (PowerShell)
  hooks/launch.ps1               <- hook glue: stdin JSON -> window -> decision JSON
  hooks/Show-AgentPrompt.ps1     <- WPF window + global double-Esc (WH_KEYBOARD_LL)
  hooks/hooks.json               <- plugin hooks config (uses ${CLAUDE_PLUGIN_ROOT})
  settings.fragment.json         <- ABSOLUTE-path config for ~/.claude/settings.json
  .claude-plugin/plugin.json     <- plugin manifest (v0.3.0)
  .claude-plugin/marketplace.json
  README.md                      <- install (global / project / plugin) + caveats

rust/floating-prompt/            <- Rust Win32 SHELL (NOT compiled, no design)
  Cargo.toml                     <- windows crate 0.62, feature flags set
  src/main.rs                    <- topmost no-activate window + double-Esc + contract
  README.md                      <- build + integration + likely-to-fix API spots

(early drafts, ignore: ./Show-AgentPrompt.ps1, ./Test-AgentPrompt.ps1, ./plugin/*)
```

---

## 7. INSTALL (current working PowerShell version, GLOBAL)

```powershell
mkdir "$env:USERPROFILE\.claude\hooks"
copy v2\hooks\launch.ps1            "$env:USERPROFILE\.claude\hooks\"
copy v2\hooks\Show-AgentPrompt.ps1  "$env:USERPROFILE\.claude\hooks\"
notepad "$env:USERPROFILE\.claude\settings.json"   # merge the "hooks" block from
                                                   # v2/settings.fragment.json,
                                                   # replace <you> -> your username
# restart Claude Code; verify with /hooks
```

Hooks registered: `Stop`, `PreToolUse` matcher `AskUserQuestion|ExitPlanMode`
(-> -Event Question), `PreToolUse` matcher `Bash|Write|Edit|MultiEdit`
(-> -Event Gate).

Standalone window test (no agent):
```powershell
'{ "stop_hook_active": false }' | powershell -File launch.ps1 -Event Stop
```

---

## 8. KNOWN OPEN ITEMS / NEXT STEPS

Priority order for the local session:

1. **Tune the Gate matcher.** `Bash|Write|Edit|MultiEdit` currently pops a window
   before EVERY such tool call — too noisy. Narrow with `if:"Bash(rm *)"` or a
   curated risky-command list, or drop Gate and keep only Stop + Question.

2. **Compile the Rust shell.** No Rust toolchain was available when it was
   written, so it is UNVERIFIED. `cd rust/floating-prompt && cargo build --release`.
   Likely fix spots (windows-crate version drift): `SystemParametersInfoW` arg
   types + `SPI_GETWORKAREA` out-param cast; `HHOOK`/`HINSTANCE` null construction;
   `CallNextHookEx`/`SetWindowsHookExW` `Option<HHOOK>` vs `HHOOK`; `DrawTextW`
   mutable buffer. Logic/structure is believed sound; expect type-wrapper nits.

3. **Add real UI to the Rust shell** (after it compiles). Design hook points are
   marked in src/main.rs: `paint_placeholder`, the `WM_KEYDOWN` Enter stub,
   `WM_COMMAND` handler. New controls set `Outcome::Answered(text)` before
   `DestroyWindow`. Keep the stdout/exit-10 contract intact — then point
   launch.ps1 at floating-prompt.exe (one-line swap, see rust README).

4. **Multi-session handling.** Modal window blocks one hook at a time; concurrent
   Claude Code sessions queue rather than stack. A small manager (queue or stacked
   windows) is unbuilt. Consider session labeling via `session_id` from the hook
   payload.

5. **Headless true-answer mode (bigger).** For real in-tool AskUserQuestion
   answering, build a `claude -p` wrapper using PreToolUse defer -> collect answer
   in window -> `--resume` with allow + updatedInput. Out of scope so far.

6. **Global-hook restriction fallback.** `SetWindowsHookEx` can be blocked in
   locked-down/enterprise environments. Window X (also -> Dismissed) is the
   fallback; consider a visible Dismiss button too.

---

## 9. ENVIRONMENT NOTES

- User is on Windows (username appeared as `<you>` in paths — confirm).
- Claude Code: native Windows support, hooks run via PowerShell. The user's
  Claude Code recognized the hooks fine; the live loop test worked after the
  stop_hook_active fix.
- The user reports things tersely ("there's always a but") and catches real
  bugs — verify claims, push back when warranted (e.g. PermissionRequest IS a
  valid event even though we removed it for the deny-bug reason).
- Nothing in this project has been compiled/run by the assistant; the user is
  the test harness. The PowerShell path is confirmed working by the user; the
  Rust path is not yet built.

---

## 10. QUICK START FOR THE NEXT SESSION

> "Continue the Windows floating-prompt project. Read HANDOFF.md. Current state:
> the PowerShell version (v2/) works and is installed. Next I want to [pick from
> section 8]. The behavior contract in section 2 and the lessons in section 4
> are fixed — don't regress them."
