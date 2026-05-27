---
name: floating-prompt
description: Configure the floating-prompt popup (the window that appears on Stop / AskUserQuestion / Gate hooks). Subcommands - `/floating-prompt on`, `/floating-prompt off`, `/floating-prompt status`, `/floating-prompt palette <name>`, `/floating-prompt palettes`, `/floating-prompt show`.
---

# /floating-prompt

Controls per-project configuration for `floating-prompt.exe` (the popup that
appears on Stop / AskUserQuestion / Gate hooks). Settings live in
`%LOCALAPPDATA%\floating-prompt\state.json` and are picked up by the `.exe`
on its next launch — no restart needed.

## Subcommands

### `/floating-prompt on` / `/floating-prompt off`

Turn the popup system on or off globally. When **off**, every hook
invocation (Stop, AskUserQuestion, Bash gate, etc.) exits silently with
exit code 0 and emits no decision JSON — Claude Code proceeds as if no
hook were registered. When **on** (the default), hooks behave normally.

To apply: read `state.json` (creating it if missing), set
`enabled = true` or `false`, write it back.

```powershell
$state_path = Join-Path $env:LOCALAPPDATA 'floating-prompt\state.json'
$dir = Split-Path $state_path -Parent
if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
$state = if (Test-Path $state_path) { Get-Content $state_path -Raw | ConvertFrom-Json } else { New-Object PSObject }
$state | Add-Member -NotePropertyName enabled -NotePropertyValue $false -Force   # use $true for /on
$json = $state | ConvertTo-Json -Depth 5
# Write UTF-8 WITHOUT BOM. PS 5.1's `Set-Content -Encoding UTF8` adds one,
# and the .exe (older builds without the BOM-strip fix) silently rejects
# the file, falling back to defaults and re-enabling the popup.
[System.IO.File]::WriteAllText($state_path, $json, (New-Object System.Text.UTF8Encoding($false)))
```

The change is picked up by the next hook invocation; no restart of Claude
Code is needed. While off, you can still launch the popup manually via
`floating-prompt.exe --message "..."` — the disable flag only suppresses
hook-mode invocations.

### `/floating-prompt status`

Read `state.json` and report a single line: whether the system is on or
off, plus how many per-project palette mappings exist. If the file
doesn't exist, report "on (default — no state file yet)".

### `/floating-prompt palette <name>`

Sets the palette used when the popup fires from the **current project**
(`basename(cwd)`). Available palettes:

- **slate** — cool gray, soft blue accent (good default for backend work).
- **ocean** — deep blue/teal with a mint accent.
- **amber** — warm dark brown with an amber accent.
- **forest** — deep green with a sage accent.
- **plum** — dark purple with a lavender accent.
- **default** — neutral near-black with a near-white accent.

To apply:

1. Read `%LOCALAPPDATA%\floating-prompt\state.json` (if it exists; default
   to `{}` if not).
2. Parse it as JSON. Make sure the `"palettes"` key exists as an object;
   create it if missing.
3. Set `palettes[<basename(cwd)>] = <name>`. Use the project name exactly
   as it appears as the last segment of `cwd` (case-sensitive).
4. Write the file back, pretty-printed.

In PowerShell:

```powershell
$state_path = Join-Path $env:LOCALAPPDATA 'floating-prompt\state.json'
$dir = Split-Path $state_path -Parent
if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
$state = if (Test-Path $state_path) { Get-Content $state_path -Raw | ConvertFrom-Json } else { @{} }
if (-not $state.palettes) { $state | Add-Member -NotePropertyName palettes -NotePropertyValue @{} -Force }
$project = Split-Path -Leaf (Get-Location)
$state.palettes.$project = '<name>'
$json = $state | ConvertTo-Json -Depth 5
# Write UTF-8 WITHOUT BOM (see /off block above for why).
[System.IO.File]::WriteAllText($state_path, $json, (New-Object System.Text.UTF8Encoding($false)))
```

Then tell the user: "Palette set to **<name>** for project `<project>`. It'll
take effect the next time a hook fires."

### `/floating-prompt palettes`

Just print the six palette names from the list above (no file I/O).

### `/floating-prompt show`

Read and print `%LOCALAPPDATA%\floating-prompt\state.json` (pretty-formatted)
so the user can audit the current configuration. If the file doesn't exist,
say so.

## Conventions

- The .exe also accepts `--palette <name>` directly (e.g. for ad-hoc
  testing); the CLI flag overrides any per-project mapping.
- If a user sets an unknown palette name, the .exe falls back to `default`
  silently — so palette typos won't break the popup.
- The state file is per-machine (in `%LOCALAPPDATA%`, not roamed).
