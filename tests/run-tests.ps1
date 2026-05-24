<#
  tests/run-tests.ps1 - integration test harness for floating-prompt v0.2.

  Covers REQUIREMENTS.md R2 (multi-session queue) and R3 (position persistence)
  to the extent automated tests can - interactive steps are prompted; assertions
  on file state, exit codes, and stdout content run automatically.

  Usage (from project root):
    powershell -ExecutionPolicy Bypass -File tests/run-tests.ps1
    powershell -ExecutionPolicy Bypass -File tests/run-tests.ps1 -OnlyTest 2
#>
param(
    [int]$OnlyTest = 0
)

$ErrorActionPreference = "Stop"
$script:RootDir   = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$script:Exe       = Join-Path $script:RootDir "target\release\floating-prompt.exe"
$script:DataDir   = Join-Path $env:LOCALAPPDATA "floating-prompt"
$script:StatePath = Join-Path $script:DataDir   "state.json"
$script:QueueDir  = Join-Path $script:DataDir   "queue"
$script:TmpDir    = Join-Path $env:TEMP         "fp-tests"

if (-not (Test-Path $script:Exe)) {
    throw "floating-prompt.exe not found at $($script:Exe). Build first: cargo build --release"
}

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
function Reset-Environment {
    Get-Process floating-prompt -ErrorAction SilentlyContinue | ForEach-Object {
        Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
    }
    Remove-Item -Recurse -Force $script:QueueDir -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $script:TmpDir   -ErrorAction SilentlyContinue
    New-Item    -ItemType Directory -Force $script:TmpDir | Out-Null
    # Don't reset state.json - its persistence is what test 4 exercises.
}

function Start-FpAsync {
    param($Title, $Message, $Options, $StdoutFile)
    $argList = @("--event", "Question", "--title", $Title, "--message", $Message)
    if ($Options) { $argList += @("--options", $Options) }
    # NOTE: do NOT pass -WindowStyle Hidden - it suppresses the floating
    # window itself for GUI-subsystem child processes.
    $p = Start-Process -FilePath $script:Exe `
        -ArgumentList $argList `
        -RedirectStandardOutput $StdoutFile `
        -RedirectStandardError  ($StdoutFile + ".err") `
        -PassThru
    # Without touching .Handle here, the Process wrapper never opens an OS
    # handle with PROCESS_QUERY_INFORMATION rights, and $p.ExitCode reads as
    # $null after WaitForExit. This one access forces the handle open.
    $null = $p.Handle
    $p
}

function Assert {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) {
        Write-Host "FAIL: $Message" -ForegroundColor Red
        $script:FailCount++
    } else {
        Write-Host "PASS: $Message" -ForegroundColor Green
        $script:PassCount++
    }
}

function Section {
    param([string]$Name)
    Write-Host ""
    Write-Host "================================================================" -ForegroundColor Cyan
    Write-Host "  $Name" -ForegroundColor Cyan
    Write-Host "================================================================" -ForegroundColor Cyan
}

function Prompt-User {
    param([string]$Text)
    Write-Host ""
    Write-Host ">>> $Text" -ForegroundColor Yellow
    Read-Host "    (press Enter when done)" | Out-Null
}

function Read-StdoutTrimmed {
    param([string]$Path)
    $raw = Get-Content $Path -Raw -ErrorAction SilentlyContinue
    if ($null -eq $raw) { return "" }
    return $raw.Trim()
}

$script:PassCount = 0
$script:FailCount = 0

# ---------------------------------------------------------------------------
# Test 1: Single window basic - exit codes + stdout for option vs dismiss
# ---------------------------------------------------------------------------
function Test-SingleWindowBasic {
    Section "TEST 1: Single window - option click and dismiss contract"
    Reset-Environment

    $stdout = Join-Path $script:TmpDir "t1-allow.out"
    $p = Start-FpAsync "Test 1a: click Allow" "Click the Allow button" "Allow,Deny" $stdout
    Prompt-User "Click the 'Allow' button on the window that appeared."
    $p.WaitForExit(60000) | Out-Null
    $ec  = $p.ExitCode
    $out = Read-StdoutTrimmed $stdout
    Assert ($ec -eq 0)      "1a exit code == 0 (got $ec)"
    Assert ($out -eq "Allow") "1a stdout == 'Allow' (got '$out')"

    Reset-Environment
    $stdout = Join-Path $script:TmpDir "t1-dismiss.out"
    $p = Start-FpAsync "Test 1b: double-Esc" "Press Esc twice quickly" "Allow,Deny" $stdout
    Prompt-User "Press Esc twice quickly (within 0.6s) to dismiss the window."
    $p.WaitForExit(60000) | Out-Null
    $ec  = $p.ExitCode
    $out = Read-StdoutTrimmed $stdout
    Assert ($ec -eq 10) "1b exit code == 10 (got $ec)"
    Assert ($out -eq "") "1b stdout is empty (got '$out')"
}

# ---------------------------------------------------------------------------
# Test 2: Multi-session queue (R2)
# ---------------------------------------------------------------------------
function Test-MultiSessionQueue {
    Section "TEST 2: R2 - multi-session queue (3 concurrent sessions)"
    Reset-Environment

    $stdoutA = Join-Path $script:TmpDir "t2-A.out"
    $stdoutB = Join-Path $script:TmpDir "t2-B.out"
    $stdoutC = Join-Path $script:TmpDir "t2-C.out"

    Write-Host "Spawning 3 .exe instances..." -ForegroundColor Yellow
    $pA = Start-FpAsync "Session A" "Session A request - click Alpha"   "Alpha,Beta"            $stdoutA
    Start-Sleep -Milliseconds 50
    $pB = Start-FpAsync "Session B" "Session B request - click Bravo"   "Alpha,Bravo"           $stdoutB
    Start-Sleep -Milliseconds 50
    $pC = Start-FpAsync "Session C" "Session C request - click Charlie" "Alpha,Bravo,Charlie"   $stdoutC

    Start-Sleep -Milliseconds 500
    $procCount = @(Get-Process floating-prompt -ErrorAction SilentlyContinue).Count
    Assert ($procCount -eq 3) "all 3 .exe processes are running (got $procCount)"

    Prompt-User "ONLY ONE window should be visible (Session A) with '1 of 3' shown. Verify visually, then click 'Alpha' on it."

    $pA.WaitForExit(60000) | Out-Null
    $ecA  = $pA.ExitCode
    $outA = Read-StdoutTrimmed $stdoutA
    Assert ($ecA  -eq 0)        "Session A exit == 0 (got $ecA)"
    Assert ($outA -eq "Alpha")  "Session A stdout == 'Alpha' (got '$outA')"

    Start-Sleep -Milliseconds 800
    Prompt-User "Session B's window should now be visible with '1 of 2'. Click 'Bravo'."

    $pB.WaitForExit(60000) | Out-Null
    $ecB  = $pB.ExitCode
    $outB = Read-StdoutTrimmed $stdoutB
    Assert ($ecB  -eq 0)       "Session B exit == 0 (got $ecB)"
    Assert ($outB -eq "Bravo") "Session B stdout == 'Bravo' (got '$outB')"

    Start-Sleep -Milliseconds 800
    Prompt-User "Session C's window should now be visible (no counter, since alone). Click 'Charlie'."

    $pC.WaitForExit(60000) | Out-Null
    $ecC  = $pC.ExitCode
    $outC = Read-StdoutTrimmed $stdoutC
    Assert ($ecC  -eq 0)        "Session C exit == 0 (got $ecC)"
    Assert ($outC -eq "Charlie") "Session C stdout == 'Charlie' (got '$outC')"

    Start-Sleep -Milliseconds 300
    $remaining = @(Get-ChildItem $script:QueueDir -ErrorAction SilentlyContinue).Count
    Assert ($remaining -eq 0) "queue dir empty after all sessions answered (got $remaining files)"
}

# ---------------------------------------------------------------------------
# Test 3: Stale request cleanup
# ---------------------------------------------------------------------------
function Test-StaleCleanup {
    Section "TEST 3: Stale request cleanup - killed .exe doesn't wedge queue"
    Reset-Environment

    $stdoutDead = Join-Path $script:TmpDir "t3-dead.out"
    $pDead = Start-FpAsync "Stale: will be killed" "About to be killed" "" $stdoutDead
    Start-Sleep -Milliseconds 400
    Stop-Process -Id $pDead.Id -Force
    $deadCount = @(Get-ChildItem $script:QueueDir -ErrorAction SilentlyContinue).Count
    Assert ($deadCount -eq 1) "1 stale request file remains after kill (got $deadCount)"

    $stdoutLive = Join-Path $script:TmpDir "t3-live.out"
    $pLive = Start-FpAsync "Live after stale" "Click OK - the dead one should have been cleaned" "OK" $stdoutLive
    Prompt-User "A window should appear within ~1 second (showing 'Live after stale'). Click 'OK'."
    $pLive.WaitForExit(60000) | Out-Null
    $ecL  = $pLive.ExitCode
    $outL = Read-StdoutTrimmed $stdoutLive
    Assert ($ecL  -eq 0)    "live session exit == 0 (got $ecL)"
    Assert ($outL -eq "OK") "live session stdout == 'OK' (got '$outL')"
}

# ---------------------------------------------------------------------------
# Test 4: Position persistence (R3)
# ---------------------------------------------------------------------------
function Test-PositionPersistence {
    Section "TEST 4: R3 - window position is remembered"
    Reset-Environment
    Remove-Item -Force $script:StatePath -ErrorAction SilentlyContinue

    # 4a: drag the window anywhere; verify state.json got written with valid numbers.
    $stdout4a = Join-Path $script:TmpDir "t4a.out"
    $p4a = Start-FpAsync "4a: drag me anywhere" "Drag the window (by its top portion) to ANY position you like, then click OK" "OK" $stdout4a
    Prompt-User "Drag the window by its TOP AREA (above the buttons) to ANY position you want, then click 'OK'."
    $p4a.WaitForExit(120000) | Out-Null
    $ec4a = $p4a.ExitCode
    Assert ($ec4a -eq 0) "4a exit == 0 (got $ec4a)"
    $stateExists = Test-Path $script:StatePath
    Assert $stateExists "4a state.json exists at $script:StatePath"
    $script:DraggedX = $null; $script:DraggedY = $null
    if ($stateExists) {
        $state = Get-Content $script:StatePath -Raw | ConvertFrom-Json
        $script:DraggedX = [int]$state.x
        $script:DraggedY = [int]$state.y
        Write-Host "    state.json contents: x=$script:DraggedX, y=$script:DraggedY" -ForegroundColor Gray
        Assert ($script:DraggedX -ne 0 -or $script:DraggedY -ne 0) "4a state.json holds non-default coords"
    }

    # 4b: new spawn opens at the saved position.
    $stdout4b = Join-Path $script:TmpDir "t4b.out"
    $p4b = Start-FpAsync "4b: where you dragged" "If this opened at the same spot as 4a, click YES." "YES,NO" $stdout4b
    Prompt-User "The new window should open AT THE POSITION you dragged 4a to. Click 'YES' if it did, 'NO' if not."
    $p4b.WaitForExit(60000) | Out-Null
    $out4b = Read-StdoutTrimmed $stdout4b
    Assert ($out4b -eq "YES") "4b window opened at remembered position (your answer: '$out4b')"

    # 4c: position survives MULTIPLE consecutive spawns (no overwrite).
    # state.json is unchanged from 4a - 4b's spawn didn't write anything new
    # because the user didn't drag again. Same coords should be used.
    $stateBefore = Get-Content $script:StatePath -Raw
    $stdout4c = Join-Path $script:TmpDir "t4c.out"
    $p4c = Start-FpAsync "4c: still where you dragged" "Same as 4b - if window is at the same dragged position, click YES." "YES,NO" $stdout4c
    Prompt-User "Third spawn in a row WITHOUT you dragging again. It should still be at the position you set in 4a. Click 'YES' if it is."
    $p4c.WaitForExit(60000) | Out-Null
    $out4c = Read-StdoutTrimmed $stdout4c
    Assert ($out4c -eq "YES") "4c position persists across multiple spawns (your answer: '$out4c')"
    $stateAfter = Get-Content $script:StatePath -Raw
    Assert ($stateBefore -eq $stateAfter) "4c state.json unchanged when user did not drag"

    # 4d: write bogus off-screen coords; window should be CLAMPED to the
    # nearest visible work area, which lands it near (but not exactly at) the
    # bottom-right corner. This is per R3.4 - clamping, not defaulting.
    Remove-Item -Force $script:StatePath -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force $script:DataDir | Out-Null
    Set-Content -Path $script:StatePath -Value '{"x":99999,"y":99999}' -Encoding utf8
    $stdout4d = Join-Path $script:TmpDir "t4d.out"
    $p4d = Start-FpAsync "4d: clamp test" "state.json was deliberately corrupted with off-screen coords (99999, 99999). Per spec R3.4 the window should appear at the bottom-right corner of your work area (clamped). NOT a position you previously dragged to." "OK" $stdout4d
    Prompt-User "Window should appear at the bottom-right CORNER of your screen (clamped from off-screen). Click 'OK' if it appeared anywhere visible."
    $p4d.WaitForExit(60000) | Out-Null
    $ec4d = $p4d.ExitCode
    Assert ($ec4d -eq 0) "4d off-screen saved position was clamped to visible (exit $ec4d)"
}

# ---------------------------------------------------------------------------
# Runner
# ---------------------------------------------------------------------------
$tests = @(
    @{ N = 1; Fn = { Test-SingleWindowBasic } },
    @{ N = 2; Fn = { Test-MultiSessionQueue } },
    @{ N = 3; Fn = { Test-StaleCleanup } },
    @{ N = 4; Fn = { Test-PositionPersistence } }
)

foreach ($t in $tests) {
    if ($OnlyTest -ne 0 -and $t.N -ne $OnlyTest) { continue }
    & $t.Fn
}

Section "RESULTS"
Write-Host "PASS: $script:PassCount" -ForegroundColor Green
$failColor = "Green"
if ($script:FailCount -gt 0) { $failColor = "Red" }
Write-Host "FAIL: $script:FailCount" -ForegroundColor $failColor
if ($script:FailCount -gt 0) { exit 1 }
