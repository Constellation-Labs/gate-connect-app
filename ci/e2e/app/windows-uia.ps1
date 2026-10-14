# Windows UI Automation driver for the installed Gate Connect bundle (AG-905),
# the counterpart of macos-ax.js.
#
# Why not WebDriver: msedgedriver attaches to WebView2 by handing the runtime
# --remote-debugging-port through WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS, and
# the runtime drops that variable when the app passes its own
# AdditionalBrowserArguments in code, which wry always does. Seen on CI: the
# browser process ran with wry's list and no debugging port, so the session
# failed with "DevToolsActivePort file doesn't exist" (a per-user WebView2
# policy with the same value was dropped too). Getting the port in would take
# a change to the app, which a release build must not carry.
#
# UI Automation needs nothing from the app. WebView2 publishes the page to the
# OS accessibility tree the way WKWebView does on macOS: a <button> is a Button
# named by its text, an <input> an Edit named by its <label>, and text is Text.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File windows-uia.ps1 <command> [args...]
#
#   probe                      can this process see the desktop through UIA?
#   wait-window <secs>         wait for a window of the app to exist
#   dump                       print the app's accessibility tree
#   wait-text <secs> <text>    wait for an element whose name contains <text>
#   click <secs> <text>        Invoke the button (or link) named <text>
#   type <secs> <label> <text> focus the field labelled <label> and type <text>
#
# Exit 0 on success, 1 on a failed step (with the reason on stdout), 2 on
# usage. Matching is exact on a button's own name and substring elsewhere.
param([Parameter(Position = 0)][string]$Command, [Parameter(Position = 1, ValueFromRemainingArguments = $true)][string[]]$Rest)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms

$ProcessName = 'gate-connect-desktop'
$A = [System.Windows.Automation.AutomationElement]
$CT = [System.Windows.Automation.ControlType]
$Scope = [System.Windows.Automation.TreeScope]

function Fail([string]$msg) {
  Write-Output "FAIL: $msg"
  exit 1
}

function AppWindows {
  $ids = @(Get-Process -Name $ProcessName -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
  if ($ids.Count -eq 0) { return @() }
  $out = @()
  foreach ($w in $A::RootElement.FindAll($Scope::Children, [System.Windows.Automation.Condition]::TrueCondition)) {
    try { if ($ids -contains $w.Current.ProcessId) { $out += $w } } catch {}
  }
  return $out
}

# One record per element, with what a match might use. The page lives in a
# child window of the msedgewebview2 process, which a Descendants search from
# the app's top-level window reaches.
function Tree {
  $out = @()
  foreach ($w in AppWindows) {
    $out += [pscustomobject]@{ El = $w; Depth = 0; Type = 'Window'; Name = (Safe { $w.Current.Name }); Help = ''; Enabled = $true; Offscreen = $false }
    foreach ($e in $w.FindAll($Scope::Descendants, [System.Windows.Automation.Condition]::TrueCondition)) {
      $c = $null
      try { $c = $e.Current } catch { continue }
      $out += [pscustomobject]@{
        El = $e; Depth = 1
        Type = ($c.ControlType.ProgrammaticName -replace '^ControlType\.', '')
        Name = [string]$c.Name; Help = [string]$c.HelpText
        Enabled = $c.IsEnabled; Offscreen = $c.IsOffscreen
      }
    }
  }
  return $out
}

function Safe([scriptblock]$f) { try { [string](& $f) } catch { '' } }

function Poll([double]$secs, [scriptblock]$find) {
  $until = (Get-Date).AddSeconds($secs)
  while ($true) {
    $hit = $null
    try { $hit = & $find (Tree) } catch {}
    if ($hit) { return $hit }
    if ((Get-Date) -gt $until) { return $null }
    Start-Sleep -Milliseconds 500
  }
}

function Describe($n) {
  $help = if ($n.Help) { " help=`"$($n.Help)`"" } else { '' }
  $state = if (-not $n.Enabled) { ' (disabled)' } elseif ($n.Offscreen) { ' (offscreen)' } else { '' }
  "$('  ' * $n.Depth)$($n.Type) `"$($n.Name)`"$help$state"
}

switch ($Command) {
  'probe' {
    $root = $A::RootElement
    Write-Output "UIA root: $($root.Current.Name), $(@($root.FindAll($Scope::Children, [System.Windows.Automation.Condition]::TrueCondition)).Count) top-level windows"
    Write-Output "interactive session: $([Environment]::UserInteractive), session $((Get-Process -Id $PID).SessionId)"
  }
  'wait-window' {
    $secs = [double]($Rest[0])
    $until = (Get-Date).AddSeconds($secs)
    while ($true) {
      $ws = @(AppWindows)
      if ($ws.Count -gt 0) {
        Write-Output "windows: $(($ws | ForEach-Object { $_.Current.Name }) -join ', ')"
        exit 0
      }
      if ((Get-Date) -gt $until) { Fail "no Gate Connect window within ${secs}s" }
      Start-Sleep -Milliseconds 500
    }
  }
  'dump' {
    foreach ($n in Tree) { Write-Output (Describe $n) }
  }
  'wait-text' {
    $secs = [double]$Rest[0]; $text = $Rest[1]
    $hit = Poll $secs { param($t) $t | Where-Object { $_.Name.Contains($text) -or $_.Help.Contains($text) } | Select-Object -First 1 }
    if (-not $hit) { Fail "no element reading `"$text`" within ${secs}s" }
    Write-Output "found: $((Describe $hit).Trim())"
  }
  'click' {
    $secs = [double]$Rest[0]; $text = $Rest[1]
    $hit = Poll $secs { param($t) $t | Where-Object { ($_.Type -eq 'Button' -or $_.Type -eq 'Hyperlink') -and $_.Name.Trim() -eq $text -and $_.Enabled -and -not $_.Offscreen } | Select-Object -First 1 }
    if (-not $hit) { Fail "no button `"$text`" within ${secs}s" }
    # InvokePattern is UIA's AXPress: the control's default action, which
    # Chromium dispatches as a click on the element.
    $hit.El.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
    Write-Output "pressed: $((Describe $hit).Trim())"
  }
  'type' {
    $secs = [double]$Rest[0]; $label = $Rest[1]; $text = $Rest[2]
    $hit = Poll $secs { param($t) $t | Where-Object { $_.Type -eq 'Edit' -and ($_.Name.Contains($label) -or $_.Help.Contains($label)) -and $_.Enabled -and -not $_.Offscreen } | Select-Object -First 1 }
    if (-not $hit) { Fail "no text field labelled `"$label`" within ${secs}s" }
    $hit.El.SetFocus()
    Start-Sleep -Milliseconds 300
    # Real key events through the OS input queue, so React sees input events
    # as it does from a person. ValuePattern.SetValue would bypass them.
    # SendKeys reads + ^ % ~ ( ) { } [ ] as commands, so brace each one.
    $escaped = [regex]::Replace($text, '[+^%~(){}\[\]]', { param($m) "{$($m.Value)}" })
    [System.Windows.Forms.SendKeys]::SendWait($escaped)
    Start-Sleep -Milliseconds 300
    $value = $hit.El.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).Current.Value
    if (-not [string]::Equals($value, $text, [StringComparison]::Ordinal)) {
      # Where they part, by position only: the value may be a real secret on
      # some later flow, so no characters are printed.
      $i = 0
      while ($i -lt [Math]::Min($value.Length, $text.Length) -and $value[$i] -eq $text[$i]) { $i++ }
      Fail "field `"$label`" reads $($value.Length) characters after typing $($text.Length), first difference at $i"
    }
    Write-Output "typed $($text.Length) characters into: $((Describe $hit).Trim())"
  }
  default {
    Write-Output 'usage: windows-uia.ps1 probe|wait-window|dump|wait-text|click|type ...'
    exit 2
  }
}
