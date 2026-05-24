<#
  launch.ps1 - BLOCKING hook glue (v0.3 - LEGACY).

  As of floating-prompt v0.2, the recommended install bypasses this script
  entirely: settings.json invokes floating-prompt.exe directly with --hook,
  and the .exe handles stdin parsing + decision JSON emission natively.
  This file is preserved for the PowerShell-WPF fallback install path.

  Runs SYNCHRONOUSLY (no async in the config), so while the window is open the
  Claude Code turn HANGS until the user answers or double-Esc dismisses.

  All gating flows through events that actually honor decisions:
    - Stop          -> {"decision":"block","reason":"<answer>"} continues the turn
    - PreToolUse     -> permissionDecision allow/deny/ask  (this WORKS for gating;
                        PermissionRequest deny is currently ignored by Claude Code)

  -Event values:
    Stop      : agent finished; answer is injected as the next instruction
    Question  : PreToolUse on AskUserQuestion/ExitPlanMode
    Gate      : PreToolUse on a consequential tool (Bash, Write, Edit, ...) used
                as the permission prompt; Allow/Deny/free-text -> permissionDecision

  Looping: the window appears on every Stop (including ones that follow a
  blocked Stop). The human is the terminator - double-Esc ends the turn. We
  never auto-inject text, so there is no runaway risk.
#>
param(
    [ValidateSet("Stop", "Question", "Gate")]
    [string]$Event = "Stop"
)
 
$ErrorActionPreference = "Stop"
 
$raw = [Console]::In.ReadToEnd()
$payload = $null
if ($raw -and $raw.Trim().Length -gt 0) {
    try { $payload = $raw | ConvertFrom-Json } catch { $payload = $null }
}
 
# NOTE on looping: stop_hook_active is TRUE on every Stop that follows a blocked
# Stop. That is exactly our normal conversational case, so we must NOT bail here
# or the window only ever shows once. The loop terminator is the human: blocking
# only happens when the user types an answer; double-Esc ends the turn. We do not
# auto-inject text, so there is no runaway risk.
 
# --- Derive title / message / options ---
$title = "Agent needs you"; $message = ""; $options = ""
 
switch ($Event) {
    "Stop" {
        $title = "Agent finished - reply or dismiss"
        $message = "Claude finished this turn. Type a reply to keep going, or double-Esc to let it stop."
        if ($payload -and $payload.transcript_path -and (Test-Path $payload.transcript_path)) {
            try {
                foreach ($line in (Get-Content -LiteralPath $payload.transcript_path -Tail 50)) {
                    $obj = $null; try { $obj = $line | ConvertFrom-Json } catch { continue }
                    if ($obj.message.content) {
                        foreach ($b in $obj.message.content) {
                            if ($b.type -eq "text" -and $b.text) { $message = $b.text }
                        }
                    }
                }
            } catch { }
        }
    }
    "Question" {
        $title = "Agent has a question"
        $message = "Claude needs your input."
        if ($payload -and $payload.tool_input.questions) {
            $q = $payload.tool_input.questions[0]
            if ($q.question) { $message = $q.question }
            if ($q.options) {
                $labels = @(); foreach ($o in $q.options) { if ($o.label) { $labels += $o.label } }
                $options = ($labels -join ",")
            }
        } elseif ($payload -and $payload.tool_name -eq "ExitPlanMode") {
            $title = "Plan ready"; $message = "Approve the plan, or type changes."
            $options = "Approve"
        }
    }
    "Gate" {
        $title = "Permission needed"
        if ($payload -and $payload.tool_input.command) { $message = "Run: " + $payload.tool_input.command }
        elseif ($payload -and $payload.tool_name)      { $message = ("Allow " + $payload.tool_name + "?") }
        else { $message = "Claude wants to run a tool." }
        $options = "Allow,Deny"
    }
}
 
if ($message.Length -gt 400) { $message = $message.Substring(0, 397) + "..." }
 
# --- Show the window. THIS BLOCKS the hook (and thus the turn). ---
$windowScript = Join-Path $PSScriptRoot "Show-AgentPrompt.ps1"
if (-not (Test-Path $windowScript)) { exit 0 }   # fail open: never wedge the agent
 
$answer = & $windowScript -Title $title -Message $message -Options $options
$code = $LASTEXITCODE   # 0 = answered (answer on stdout), 10 = dismissed
 
function Emit-Json($obj) {
    $obj | ConvertTo-Json -Compress -Depth 6 | Write-Output
}
 
if ($code -ne 0 -or [string]::IsNullOrWhiteSpace($answer)) {
    exit 0   # dismissed -> proceed normally
}
 
$answer = $answer.Trim()
 
switch ($Event) {
    "Stop" {
        # Block the stop and inject the user's reply as Claude's next instruction.
        Emit-Json @{ decision = "block"; reason = $answer }
        exit 0
    }
    "Gate" {
        # PreToolUse permission control - these decisions ARE honored.
        if ($answer -eq "Allow") {
            Emit-Json @{ hookSpecificOutput = @{ hookEventName = "PreToolUse"; permissionDecision = "allow"; permissionDecisionReason = "Approved by user via floating prompt." } }
        } elseif ($answer -eq "Deny") {
            Emit-Json @{ hookSpecificOutput = @{ hookEventName = "PreToolUse"; permissionDecision = "deny"; permissionDecisionReason = "Denied by user via floating prompt." } }
        } else {
            # Free-text -> deny and hand the instruction back to Claude.
            Emit-Json @{ hookSpecificOutput = @{ hookEventName = "PreToolUse"; permissionDecision = "deny"; permissionDecisionReason = $answer } }
        }
        exit 0
    }
    "Question" {
        # Deny the tool and hand Claude the user's instruction to act on.
        Emit-Json @{ hookSpecificOutput = @{ hookEventName = "PreToolUse"; permissionDecision = "deny"; permissionDecisionReason = $answer } }
        exit 0
    }
}
 
exit 0
