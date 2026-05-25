# Smoke-test the redesigned UI in a SANDBOXED LOCALAPPDATA so test windows
# never share a queue with the user's real Claude Code floating-prompt
# sessions.
#
# Strategy per case:
#   1. Set $env:LOCALAPPDATA to a fresh temp dir for THIS process tree.
#   2. Launch floating-prompt.exe with the test args. It inherits our env
#      and creates its queue + state in the sandbox dir (no interference).
#   3. Locate the popup window via EnumWindows -> matching pid.
#   4. Optionally inject a synthetic WM_MOUSEMOVE to a target rect-local
#      coordinate so hover-state captures land on a specific option.
#   5. Screenshot the window rect (plus a small margin).
#   6. PostMessage WM_CLOSE to the popup so it tears down cleanly.
#   7. Wait for the .exe to exit; if it doesn't, kill it.
# After all cases: restore $env:LOCALAPPDATA and remove the sandbox dir.

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class U {
    public const uint WM_CLOSE     = 0x0010;
    public const uint WM_MOUSEMOVE = 0x0200;
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
    [DllImport("user32.dll")]
    public static extern bool SendMessageW(IntPtr h, uint msg, IntPtr wp, IntPtr lp);
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
    for ($attempt = 0; $attempt -lt 40; $attempt++) {
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
        [string]$Options = "",
        [string]$Previews = "",
        [string]$Mode = "",
        [string]$Project = "claude-integration",
        [string]$Session = "",
        [string]$Palette = "",
        [string]$Placeholder = "",
        # Inject a synthetic WM_MOUSEMOVE to (x,y) client-local before capture
        # so a hover visual lands on a known target (e.g. second option).
        [int]$HoverX = -1,
        [int]$HoverY = -1
    )
    Write-Host "Case: $Name"
    $cmdline = "$(Q '--title') $(Q $Title) $(Q '--message') $(Q $Message)"
    if ($Options)     { $cmdline += " $(Q '--options') $(Q $Options)" }
    if ($Previews)    { $cmdline += " $(Q '--previews') $(Q $Previews)" }
    if ($Mode)        { $cmdline += " $(Q '--mode') $(Q $Mode)" }
    if ($Project)     { $cmdline += " $(Q '--project') $(Q $Project)" }
    if ($Session)     { $cmdline += " $(Q '--session') $(Q $Session)" }
    if ($Palette)     { $cmdline += " $(Q '--palette') $(Q $Palette)" }
    if ($Placeholder) { $cmdline += " $(Q '--placeholder') $(Q $Placeholder)" }

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
    Start-Sleep -Milliseconds 400  # let initial paint settle

    if ($HoverX -ge 0 -and $HoverY -ge 0) {
        $lp = (($HoverY -band 0xFFFF) -shl 16) -bor ($HoverX -band 0xFFFF)
        [void][U]::SendMessageW($hwnd, [U]::WM_MOUSEMOVE, [IntPtr]::Zero, [IntPtr]$lp)
        Start-Sleep -Milliseconds 120
    }

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

    [void][U]::PostMessageW($hwnd, [U]::WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    if (-not $p.WaitForExit(3000)) {
        Write-Warning "  process did not exit on WM_CLOSE; killing"
        try { $p.Kill() } catch {}
    }
    Start-Sleep -Milliseconds 120
}

try {
    $stopShort = "Done. Migrated 14 files and updated the snapshot tests. Two of the snapshots changed in ways worth eyeballing - ProfileMenu and SessionBadge."

    $stopLong = @(
        "I pulled the auth flow apart and re-built it around the new RefreshTokenLease primitive. Summary of what changed:",
        "",
        "Server - the legacy /oauth/refresh handler now delegates to LeaseStore.acquire() instead of writing directly to the cache. That lets a second concurrent request piggy-back on the same upstream refresh rather than firing its own. Tests for the lease race are in auth/lease.spec.ts - all passing.",
        "",
        "Client - replaced the imperative refresh() call with a React Query mutation, so the staleness check now happens in the query cache instead of being scattered across each call site. I removed useAuthRefresh entirely; the four places that called it now read from useSession().",
        "",
        "Migration note - three feature flags referenced the old code path; I left them in place but flipped them to no-ops so a rollback can flip them back without code changes. They should be deleted next sprint.",
        "",
        "Open question - the cookie domain logic in cookieGuard.ts still hard-codes the production host. I didn't touch it because the existing tests would have needed a rewrite, but it's the obvious next thing to fix.",
        "",
        "Ready for review. Want me to open the PR or hold while you look?"
    ) -join "`n"

    $planMsg = @(
        "Here's the plan for the dashboard refactor. I'll wait for approval before touching anything.",
        "",
        "1. Extract chart primitives. Pull LineChart, BarChart, SparkChart out of dashboard/widgets/ into a new charts/ package. They currently re-implement axes three different ways; one shared Axis component will replace all three.",
        "",
        "2. Lift the data layer. The widgets each fetch their own data with bespoke useEffect calls. I'll replace them with a single useDashboardData() hook backed by React Query, so the dashboard gets one coordinated refresh instead of nine independent ones.",
        "",
        "3. Consolidate the date-range picker. The picker currently lives inside OverviewWidget; I'll hoist it to DashboardShell so it controls every widget at once.",
        "",
        "Estimated diff: ~1,800 lines added, ~1,400 removed. No public API changes. Should I proceed?"
    ) -join "`n"

    $preview1 = "## v1.42.0`n`n### Features`n- charts: add SparkChart primitive`n- auth: lease-based refresh tokens`n`n### Fixes`n- session: handle stale cookies on`n  cross-subdomain navigation"
    $preview2 = " src/charts/index.ts          | +14 -0`n src/charts/SparkChart.tsx    | +88 -0`n src/auth/lease.ts            | +52 -3`n src/widgets/Overview.tsx     | +18 -41`n ----------------------------------------`n 4 files changed, +172 -44"
    $preview3 = "The 1.42 release reworks how`nthe client refreshes auth tokens`nand introduces a small charts`npackage extracted from the`ndashboard widgets."

    # ---------- 7 canonical states (mirrors design/Floating-Prompt.html) ----------
    Capture-Case -Name "01-stop-short" `
        -Title "Agent finished" `
        -Message $stopShort `
        -Placeholder "Reply to continue, or double-Esc to let Claude stop."

    Capture-Case -Name "02-stop-long-scrollable" `
        -Title "Agent finished" `
        -Message $stopLong `
        -Placeholder "Reply to continue, or double-Esc to let Claude stop."

    Capture-Case -Name "03-q-short-3-options" `
        -Title "Agent has a question" `
        -Message "Which approach for handling tokens that expire mid-request?" `
        -Options "Refresh on read,Refresh on a schedule,Defer until the next request" `
        -Mode "single" `
        -Project "auth-service" `
        -Placeholder "Type a custom answer..."

    Capture-Case -Name "04-q-long-queued" `
        -Title "Agent has a question" `
        -Message "The migration script left orphaned rows in three tables - user_sessions, device_grants, and audit_log - when it bailed on the failed batch. Which cleanup approach should I take?" `
        -Options "Roll back the whole migration and retry from scratch.,Run the cleanup script in dry-run mode first then apply.,Leave the orphans for the nightly GC job to pick up." `
        -Mode "single" `
        -Project "backend-api" `
        -Placeholder "Type a custom answer..."

    Capture-Case -Name "05-multi-4-options" `
        -Title "Agent has a question" `
        -Message "Which of these should I run before opening the PR? Pick any." `
        -Options "Re-run the affected snapshot tests,Regenerate the OpenAPI types,Bump the changelog for the public package,Format the diff with the repo prettier config" `
        -Mode "multi" `
        -Placeholder "Or type a custom answer..."

    Capture-Case -Name "06-preview-3-options" `
        -Title "Agent has a question" `
        -Message "Which diff style for the auto-generated changelog?" `
        -Options "Conventional (grouped by type),Per-file unified diff,PR-style narrative" `
        -Previews ($preview1 + "|" + $preview2 + "|" + $preview3) `
        -Mode "preview" `
        -Project "docs-site" `
        -Placeholder "Type a custom answer..."

    Capture-Case -Name "07-plan-approve" `
        -Title "Plan ready" `
        -Message $planMsg `
        -Options "Approve" `
        -Mode "approve" `
        -Project "dashboard-refactor" `
        -Placeholder "Or describe changes to the plan..."

    # ---------- 6 palette swatches ----------
    foreach ($pal in @("slate", "ocean", "amber", "forest", "plum", "default")) {
        Capture-Case -Name ("08-palette-" + $pal) `
            -Title "Agent has a question" `
            -Message "Pulled the spec apart and re-implemented the lease handshake. All tests pass; want me to open the PR?" `
            -Options "Open the PR now,Hold for review,Run integration tests first" `
            -Mode "single" `
            -Palette $pal `
            -Project ($pal + "-project") `
            -Session "a1b2c3d" `
            -Placeholder "Type a custom answer..."
    }

    # ---------- Interaction states ----------
    # Hover lands on the second option (offset is approximate but within the
    # second card given default layout: PAD=14 + first option height + gap).
    Capture-Case -Name "09-opt-hover" `
        -Title "Agent has a question" `
        -Message "Which approach for handling tokens that expire mid-request?" `
        -Options "Refresh on read,Refresh on a schedule,Defer until the next request" `
        -Mode "single" `
        -Project "auth-service" `
        -HoverX 260 -HoverY 240
}
finally {
    $env:LOCALAPPDATA = $origLocalAppData
    Remove-Item -Recurse -Force $sandbox -ErrorAction SilentlyContinue
    Write-Host ""
    Write-Host "Sandbox cleaned. Real LOCALAPPDATA restored."
    Write-Host "All screenshots in: $out"
    Get-ChildItem $out -Filter *.png | Sort-Object Name | Select-Object Name, Length
}
