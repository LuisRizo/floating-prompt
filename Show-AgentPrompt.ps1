<#
.SYNOPSIS
  Blocking always-on-top prompt window with global double-Esc dismissal.

.DESCRIPTION
  Shows a topmost modal window and BLOCKS until the user either:
    - answers (clicks an option / types text + Send / presses Enter), OR
    - dismisses with double-Esc pressed anywhere (no focus required).

  Communicates the outcome via EXIT CODE and stdout:
    exit 0 + stdout = the user's answer   -> caller should BLOCK the turn
    exit 10, no stdout = dismissed         -> caller should let the turn END

  The double-Esc listener uses a WH_KEYBOARD_LL low-level hook so it fires
  regardless of which window has focus.

.PARAMETER Title / Message / Options
  Same as before. Options is a comma-separated list rendered as buttons.
#>
param(
    [string]$Title   = "Agent needs you",
    [string]$Message = "",
    [string]$Options = ""
)

Add-Type -AssemblyName PresentationFramework
Add-Type -AssemblyName PresentationCore
Add-Type -AssemblyName WindowsBase

# ----------------------------------------------------------------------------
# Global low-level keyboard hook for focus-independent double-Esc.
# Sets a static flag DoubleEsc=true when Esc is pressed twice within 600ms.
# ----------------------------------------------------------------------------
$kbSource = @"
using System;
using System.Runtime.InteropServices;

public static class EscWatcher {
    public static bool DoubleEsc = false;

    private const int WH_KEYBOARD_LL = 13;
    private const int WM_KEYDOWN = 0x0100;
    private const int VK_ESCAPE = 0x1B;

    private static IntPtr _hook = IntPtr.Zero;
    private static LowLevelKeyboardProc _proc = HookCallback;
    private static DateTime _lastEsc = DateTime.MinValue;

    private delegate IntPtr LowLevelKeyboardProc(int nCode, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll", SetLastError = true)]
    private static extern IntPtr SetWindowsHookEx(int idHook, LowLevelKeyboardProc lpfn, IntPtr hMod, uint dwThreadId);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool UnhookWindowsHookEx(IntPtr hhk);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern IntPtr CallNextHookEx(IntPtr hhk, int nCode, IntPtr wParam, IntPtr lParam);
    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern IntPtr GetModuleHandle(string lpModuleName);

    public static void Start() {
        _hook = SetWindowsHookEx(WH_KEYBOARD_LL, _proc, GetModuleHandle(null), 0);
    }
    public static void Stop() {
        if (_hook != IntPtr.Zero) { UnhookWindowsHookEx(_hook); _hook = IntPtr.Zero; }
    }

    private static IntPtr HookCallback(int nCode, IntPtr wParam, IntPtr lParam) {
        if (nCode >= 0 && (int)wParam == WM_KEYDOWN) {
            int vk = Marshal.ReadInt32(lParam);
            if (vk == VK_ESCAPE) {
                DateTime now = DateTime.Now;
                if ((now - _lastEsc).TotalMilliseconds <= 600) { DoubleEsc = true; }
                _lastEsc = now;
            }
        }
        return CallNextHookEx(_hook, nCode, wParam, lParam);
    }
}
"@
Add-Type -TypeDefinition $kbSource -Language CSharp

# ---- Window ----
$window = New-Object System.Windows.Window
$window.Title = $Title
$window.Width = 440
$window.Height = 250
$window.WindowStartupLocation = "Manual"
$window.Topmost = $true
$window.ResizeMode = "NoResize"
$window.WindowStyle = "ToolWindow"
$window.Background = "#1e1e1e"

$wa = [System.Windows.SystemParameters]::WorkArea
$window.Left = $wa.Right - $window.Width - 16
$window.Top  = $wa.Bottom - $window.Height - 16

$root = New-Object System.Windows.Controls.StackPanel
$root.Margin = "16"

$titleBlock = New-Object System.Windows.Controls.TextBlock
$titleBlock.Text = $Title
$titleBlock.FontSize = 16; $titleBlock.FontWeight = "Bold"
$titleBlock.Foreground = "#ffffff"; $titleBlock.Margin = "0,0,0,8"
$root.AddChild($titleBlock)

$msgBlock = New-Object System.Windows.Controls.TextBlock
$msgBlock.Text = $Message
$msgBlock.FontSize = 13; $msgBlock.Foreground = "#cccccc"
$msgBlock.TextWrapping = "Wrap"; $msgBlock.Margin = "0,0,0,12"
$root.AddChild($msgBlock)

$hint = New-Object System.Windows.Controls.TextBlock
$hint.Text = "Double-press Esc (anywhere) to dismiss"
$hint.FontSize = 11; $hint.Foreground = "#777777"; $hint.Margin = "0,0,0,10"
$root.AddChild($hint)

# Result state
$script:result    = $null     # the answer text, if answered
$script:dismissed = $false

if ($Options -and $Options.Trim().Length -gt 0) {
    $optPanel = New-Object System.Windows.Controls.WrapPanel
    foreach ($opt in ($Options -split ',')) {
        $label = $opt.Trim(); if ($label.Length -eq 0) { continue }
        $btn = New-Object System.Windows.Controls.Button
        $btn.Content = $label; $btn.Margin = "0,0,8,8"; $btn.Padding = "10,4"; $btn.Tag = $label
        $btn.Add_Click({ $script:result = $this.Tag; $window.Close() })
        $optPanel.AddChild($btn)
    }
    $root.AddChild($optPanel)
}

$inputBox = New-Object System.Windows.Controls.TextBox
$inputBox.Margin = "0,0,0,8"; $inputBox.Padding = "6"; $inputBox.FontSize = 13
$root.AddChild($inputBox)

$sendBtn = New-Object System.Windows.Controls.Button
$sendBtn.Content = "Send"; $sendBtn.Padding = "10,4"; $sendBtn.HorizontalAlignment = "Right"
$sendBtn.Add_Click({
    if ($inputBox.Text.Trim().Length -gt 0) { $script:result = $inputBox.Text.Trim(); $window.Close() }
})
$root.AddChild($sendBtn)

$inputBox.Add_KeyDown({
    if ($_.Key -eq "Return" -and $inputBox.Text.Trim().Length -gt 0) {
        $script:result = $inputBox.Text.Trim(); $window.Close()
    }
})

$window.Content = $root

# Poll the global Esc watcher on a UI timer; close if double-Esc seen.
$timer = New-Object System.Windows.Threading.DispatcherTimer
$timer.Interval = [TimeSpan]::FromMilliseconds(80)
$timer.Add_Tick({
    if ([EscWatcher]::DoubleEsc) {
        $script:dismissed = $true
        $timer.Stop()
        $window.Close()
    }
})

$window.Add_ContentRendered({
    $window.Activate() | Out-Null
    $inputBox.Focus() | Out-Null
})

[EscWatcher]::Start()
$timer.Start()
try {
    [void]$window.ShowDialog()    # BLOCKS here until closed
} finally {
    $timer.Stop()
    [EscWatcher]::Stop()
}

# ---- Outcome ----
if ($script:dismissed -or $null -eq $script:result) {
    exit 10           # dismissed -> let the turn end
} else {
    Write-Output $script:result
    exit 0            # answered  -> caller blocks the turn with this text
}
