# floating-prompt (Rust / Win32 shell)

A native Windows shell for the Claude Code floating window, written against the
`windows` crate (0.62). This is the **shell only** - it creates a correctly
configured window and wires the behavioral contract. Visual design (buttons,
text box, fonts, theming) is intentionally left as stubs.

## What the shell already does

- Registers a window class and creates a **topmost, no-activate, tool-window**
  positioned bottom-right of the primary work area.
  - `WS_EX_TOPMOST` - floats above other windows.
  - `WS_EX_NOACTIVATE` + `SW_SHOWNOACTIVATE` - appears **without stealing focus**
    from whatever you're typing in.
  - `WS_EX_TOOLWINDOW` - no taskbar button.
- Installs a process-global `WH_KEYBOARD_LL` hook for **focus-independent
  double-Esc** (two Esc within 600 ms), polled from the UI thread via a timer.
- Implements the **outcome contract** identical to the PowerShell window:
  - answered -> prints the answer to stdout, exits `0`
  - dismissed (double-Esc, or window X) -> prints nothing, exits `10`
- Parses CLI args matching the PowerShell params:
  `--event`, `--title`, `--message`, `--options`.

## Drop-in compatibility

The exit-code + stdout contract is the same as `Show-AgentPrompt.ps1`, so once
built you can point `launch.ps1` at the exe instead of the WPF script:

```powershell
# was:
$answer = & $windowScript -Title $title -Message $message -Options $options
# becomes:
$answer = & "C:\path\to\floating-prompt.exe" --event $Event --title $title --message $message --options $options
$code = $LASTEXITCODE   # 0 = answered, 10 = dismissed  (unchanged)
```

## Build

Requires the Rust toolchain with the MSVC target (default on Windows).

```powershell
cd floating-prompt
cargo build --release
# -> target\release\floating-prompt.exe
```

Run it directly to see the shell:

```powershell
.\target\release\floating-prompt.exe --event Stop --title "Agent finished" --message "Type to continue" --options "Allow,Deny"
```

Press Enter to simulate an answer (prints the first option, or "ok"); double-Esc
to dismiss.

## Where design plugs in (marked in src/main.rs)

- `paint_placeholder` - replace with the real layout/painting.
- The `WM_KEYDOWN` Enter stub - replace with real edit-control text retrieval.
- The `WM_COMMAND` handler - map button control IDs to option labels and set
  `Outcome::Answered`.
- `Outcome` - already the single source of truth for stdout/exit; new controls
  just need to set it before `DestroyWindow`.

## Not yet verified

This was written without a compiler available in the authoring environment, so
the first `cargo build` may surface a few `windows`-crate API nits to fix. The
likeliest spots, given churn between crate versions:

- `SystemParametersInfoW` arg types / the `SPI_GETWORKAREA` out-param cast.
- `HHOOK` / `HINSTANCE` null construction (`HHOOK(null_mut())`, `.into()`).
- `CallNextHookEx` / `SetWindowsHookExW` taking `Option<HHOOK>` vs `HHOOK`.
- `DrawTextW` mutable-buffer signature.

If any fail to compile, the fix is almost always a type wrapper or `Option<>`
adjustment, not a logic change. The window/hook/contract structure is sound.
```
