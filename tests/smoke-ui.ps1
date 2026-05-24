# Smoke-test the new UI in a SANDBOXED LOCALAPPDATA so test windows never
# share a queue with the user's real Claude Code floating-prompt sessions.
#
# Strategy per case:
#   1. Set $env:LOCALAPPDATA to a fresh temp dir for THIS process tree.
#   2. Launch floating-prompt.exe with the test args. It inherits our env
#      and creates its queue + state in the sandbox dir (no interference).
#   3. Locate the popup window via EnumWindows -> matching pid.
#   4. Screenshot the window rect (plus a small margin).
#   5. PostMessage WM_CLOSE to the popup so it tears down cleanly. No
#      SendKeys/SendInput, no risk of dismissing the user's real popups.
#   6. Wait for the .exe to exit; if it doesn't, kill it.
# After all cases: restore $env:LOCALAPPDATA and remove the sandbox dir.

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class U {
    public const uint WM_CLOSE = 0x0010;
    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumProc cb, IntPtr lp);
    public delegate bool EnumProc(IntPtr h, IntPtr lp);
    [DllImport("user32.dll")]
    public static extern int GetWindowThreadProcessId(IntPtr h, out int pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern int GetClassNameW(IntPtr h, StringBuilder n, int max);
    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr h);
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")]
    public static extern bool PostMessageW(IntPtr h, uint msg, IntPtr wp, IntPtr lp);
}
'@

$exe = Join-Path $PSScriptRoot "..\target\release\floating-prompt.exe"
$out = Join-Path $PSScriptRoot "screenshots"
if (-not (Test-Path $out)) { New-Item -ItemType Directory -Path $out | Out-Null }
Get-ChildItem $out -Filter *.png -ErrorAction SilentlyContinue | Remove-Item -Force

# --- SANDBOX ---
$origLocalAppData = $env:LOCALAPPDATA
$sandbox = Join-Path $env:TEMP ("fp-smoke-" + ([guid]::NewGuid().Guid.Substring(0,8)))
New-Item -ItemType Directory -Path $sandbox -Force | Out-Null
$env:LOCALAPPDATA = $sandbox
Write-Host "Sandbox: $sandbox"

function Find-Popup-ByPid {
    param([int]$ProcId)
    $script:hwndFound = [IntPtr]::Zero
    $script:targetPid = $ProcId
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        $cb = [U+EnumProc]{
            param($h, $lp)
            $wpid = 0
            [void][U]::GetWindowThreadProcessId($h, [ref]$wpid)
            if ($wpid -eq $script:targetPid -and [U]::IsWindowVisible($h)) {
                $cn = New-Object System.Text.StringBuilder 64
                [void][U]::GetClassNameW($h, $cn, 64)
                if ($cn.ToString() -eq "FloatingPromptShell") {
                    $script:hwndFound = $h
                    return $false
                }
            }
            return $true
        }
        [void][U]::EnumWindows($cb, [IntPtr]::Zero)
        if ($script:hwndFound -ne [IntPtr]::Zero) { return $script:hwndFound }
        Start-Sleep -Milliseconds 100
    }
    return [IntPtr]::Zero
}

function Q { param([string]$s) '"' + ($s -replace '"', '""') + '"' }

function Capture-Case {
    param(
        [string]$Name,
        [string]$Title,
        [string]$Message,
        [string]$Options
    )
    Write-Host "Case: $Name"
    # Build a properly-quoted command line. Start-Process -ArgumentList in PS 5.1
    # does NOT auto-quote args containing spaces, so the long message would
    # be split into many args at every space. Use ProcessStartInfo.Arguments
    # so we control the raw command line directly.
    $cmdline = "$(Q '--title') $(Q $Title) $(Q '--message') $(Q $Message)"
    if ($Options -and $Options.Length -gt 0) {
        $cmdline += " $(Q '--options') $(Q $Options)"
    }
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    $psi.Arguments = $cmdline
    $psi.UseShellExecute = $false
    $p = [System.Diagnostics.Process]::Start($psi)
    $null = $p.Handle

    $hwnd = Find-Popup-ByPid -ProcId $p.Id
    if ($hwnd -eq [IntPtr]::Zero) {
        Write-Warning "  could not find visible popup window for pid $($p.Id)"
        try { $p.Kill() } catch {}
        return
    }
    Start-Sleep -Milliseconds 350  # let paint settle

    $r = New-Object U+RECT
    $null = [U]::GetWindowRect($hwnd, [ref]$r)
    $margin = 16
    $sx = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $x = [Math]::Max(0, $r.Left   - $margin)
    $y = [Math]::Max(0, $r.Top    - $margin)
    $w = [Math]::Min($sx.Width  - $x, ($r.Right  - $r.Left) + 2 * $margin)
    $h = [Math]::Min($sx.Height - $y, ($r.Bottom - $r.Top)  + 2 * $margin)
    Write-Host ("  window rect: {0}x{1} at ({2},{3})" -f ($r.Right - $r.Left), ($r.Bottom - $r.Top), $r.Left, $r.Top)

    $bmp = New-Object System.Drawing.Bitmap $w, $h
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $src = New-Object System.Drawing.Point $x, $y
    $size = New-Object System.Drawing.Size $w, $h
    $g.CopyFromScreen($src, ([System.Drawing.Point]::Empty), $size)
    $shot = Join-Path $out "$Name.png"
    $bmp.Save($shot)
    $g.Dispose(); $bmp.Dispose()

    # Clean teardown via WM_CLOSE (the wndproc treats this as a dismiss).
    [void][U]::PostMessageW($hwnd, [U]::WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    if (-not $p.WaitForExit(3000)) {
        Write-Warning "  process did not exit on WM_CLOSE; killing"
        try { $p.Kill() } catch {}
    }
    Start-Sleep -Milliseconds 150
}

try {
    $long = "I refactored the auth middleware to use the new session store. Three files changed: src/auth.ts, src/session.ts, and tests/auth.test.ts. The migration is backward compatible - existing sessions in the old cookie format are read once and rewritten in the new format on next request. I ran the test suite and all 47 cases pass. Want me to ship it or tighten anything first?"

    $huge = @(
        "I dug into the failing migration and here is the full picture.",
        "",
        "Root cause: the new sessions table has a NOT NULL constraint on user_id, but the backfill script was reading from the old cookie blob which sometimes encoded the user id as a string of length zero when the session predated the 2024 cookie rotation. About 0.4 percent of rows fall into that bucket - around 200,000 rows out of 50 million.",
        "",
        "What I changed:",
        "1. The backfill now logs and skips any row where the parsed user_id is empty, instead of crashing the whole migration.",
        "2. Added a follow-up scan that re-resolves skipped rows via the audit log, which has a clean user_id field for 92 percent of cases.",
        "3. The remaining ~16,000 truly-orphaned rows get a NULL user_id, and the column constraint is relaxed to NULL + a partial index so legacy rows do not block new inserts.",
        "",
        "Verified locally against a snapshot of prod: full migration runs in 4m12s, no errors. The query plan for the partial index looks healthy.",
        "",
        "Three open questions for you before I ship:",
        "- Do we want to surface the 16,000 orphans to ops, or silently drop them? My read is silently drop, since they cannot be reached anyway, but it is your call.",
        "- The audit log re-resolution adds about 90 seconds to the migration. Acceptable, or should it run as a background job after the main migration completes?",
        "- I left feature flag auth.session_v2 OFF after migration. We can flip it next deploy or stage it through GrowthBook - your preference.",
        "",
        "Reply with go/no-go plus any of the above answers, or double-Esc to let me stop and I will wait."
    ) -join "`n"

    Capture-Case -Name "01-min-short" `
        -Title "Agent finished - reply or dismiss" `
        -Message "Done." `
        -Options ""

    Capture-Case -Name "02-long-message" `
        -Title "Agent finished - reply or dismiss" `
        -Message $long `
        -Options ""

    Capture-Case -Name "03-two-options" `
        -Title "Permission needed" `
        -Message "Run: rm -rf node_modules" `
        -Options "Allow,Deny"

    Capture-Case -Name "04-four-options" `
        -Title "Agent has a question" `
        -Message "Which date library should we use?" `
        -Options "date-fns,Luxon,Day.js,Moment (legacy)"

    Capture-Case -Name "05-long-labels" `
        -Title "Agent has a question" `
        -Message "How should we handle the migration failure on row 1042?" `
        -Options "Skip the row and continue with the rest of the batch,Roll back the whole migration and retry from scratch,Pause for manual inspection of the bad row"

    Capture-Case -Name "06-message-plus-options" `
        -Title "Plan ready" `
        -Message $long `
        -Options "Approve,Request changes"

    Capture-Case -Name "07-huge-response" `
        -Title "Agent finished - reply or dismiss" `
        -Message $huge `
        -Options ""

    Capture-Case -Name "08-huge-plus-options" `
        -Title "Plan ready" `
        -Message $huge `
        -Options "Ship it,Hold for review"
}
finally {
    $env:LOCALAPPDATA = $origLocalAppData
    Remove-Item -Recurse -Force $sandbox -ErrorAction SilentlyContinue
    Write-Host ""
    Write-Host "Sandbox cleaned. Real LOCALAPPDATA restored."
    Write-Host "All screenshots in: $out"
    Get-ChildItem $out -Filter *.png | Sort-Object Name | Select-Object Name, Length
}
