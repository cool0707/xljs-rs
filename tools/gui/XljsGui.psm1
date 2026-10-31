# xljs の GUI を実機の Windows で自動操作・撮影するためのヘルパー（pwsh 7）。
# 使い方は docs/gui-testing.md を参照。
#   Import-Module ./tools/gui/XljsGui.psm1
#
# マウスやキーボードの入力（SendInput）は使わない。入力があると消した画面が
# 点いてしまうため、UI Automation のパターンかウィンドウメッセージで操作する。

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms, System.Drawing
Add-Type -Namespace XljsGui -Name Native -MemberDefinition @'
[DllImport("user32.dll")] public static extern System.IntPtr SendMessage(System.IntPtr h, uint m, System.IntPtr w, System.IntPtr l);
[DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "SendMessageW")] public static extern System.IntPtr SendMessageText(System.IntPtr h, uint m, System.IntPtr w, string l);
[DllImport("user32.dll")] public static extern bool PostMessage(System.IntPtr h, uint m, System.IntPtr w, System.IntPtr l);
[DllImport("user32.dll")] public static extern bool GetWindowRect(System.IntPtr h, out RECT r);
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(System.IntPtr h);
[DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(System.IntPtr v);
public struct RECT { public int Left, Top, Right, Bottom; }
'@
# 物理ピクセルで座標を扱う（150% などの拡大率でも撮影範囲がずれないように）
[void][XljsGui.Native]::SetProcessDpiAwarenessContext([IntPtr]-4)

$script:AE = [System.Windows.Automation.AutomationElement]
$script:Cond = [System.Windows.Automation.PropertyCondition]
$script:WM_CLOSE = 0x0010; $script:WM_SETTEXT = 0x000C; $script:BM_CLICK = 0x00F5

function Wait-Until([scriptblock]$Test, [int]$TimeoutSec = 15, [string]$What = 'condition') {
    $end = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $end) {
        $r = & $Test
        if ($r) { return $r }
        Start-Sleep -Milliseconds 250
    }
    throw "timeout ($TimeoutSec s) waiting for $What"
}

<#
.SYNOPSIS 画面全体（全モニター）またはウィンドウを PNG に保存する。画面が消えていても撮れる。
#>
function Save-XljsScreenshot {
    param([Parameter(Mandatory)][string]$Path, [System.Windows.Automation.AutomationElement]$Window)
    if ($Window) {
        $r = New-Object XljsGui.Native+RECT
        [void][XljsGui.Native]::GetWindowRect([IntPtr]$Window.Current.NativeWindowHandle, [ref]$r)
        $rect = [System.Drawing.Rectangle]::FromLTRB($r.Left, $r.Top, $r.Right, $r.Bottom)
    } else {
        $rect = [System.Windows.Forms.SystemInformation]::VirtualScreen
    }
    $bmp = New-Object System.Drawing.Bitmap $rect.Width, $rect.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bmp.Size)
    $dir = Split-Path -Parent $Path
    if ($dir) { New-Item -ItemType Directory -Force $dir | Out-Null }
    $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    Get-Item $Path
}

<#
.SYNOPSIS プロセスのトップレベルウィンドウを待って返す。-Name は部分一致（ワイルドカード可）。
ファイルダイアログやメッセージボックスも xljs と同じプロセスのウィンドウ。オーナーのあるダイアログ
（フォームから開いたファイルダイアログなど）は UIA ではオーナーの子に入るので、そこも探す。
内側（後に開いたもの）を先に返す。
#>
function Get-XljsWindow {
    param([Parameter(Mandatory)][int]$ProcessId, [string]$Name = '*', [int]$TimeoutSec = 15, [switch]$All)
    $c = New-Object $script:Cond($script:AE::ProcessIdProperty, $ProcessId)
    $isWin = New-Object $script:Cond($script:AE::ControlTypeProperty, [System.Windows.Automation.ControlType]::Window)
    function collect($e, $d) {
        if ($d -lt 4) { foreach ($ch in $e.FindAll('Children', $isWin)) { collect $ch ($d + 1) } }
        $e
    }
    Wait-Until -TimeoutSec $TimeoutSec -What "window '$Name' of pid $ProcessId" {
        $ws = @(foreach ($top in $script:AE::RootElement.FindAll('Children', $c)) { collect $top 0 }) | Where-Object { $_.Current.Name -like $Name }
        $ws = @($ws)
        if ($ws.Count) { if ($All) { , $ws } else { $ws[0] } }
    }
}

<#
.SYNOPSIS 要素の UI ツリーを 1 行 1 要素で出す（コントロール種別・クラス・AutomationId・名前・対応パターン）。
#>
function Show-XljsUiTree {
    param([Parameter(Mandatory)][System.Windows.Automation.AutomationElement]$Element, [int]$MaxDepth = 30)
    $walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
    function walk($e, $d) {
        if ($d -gt $MaxDepth) { return }
        $c = $e.Current
        $pats = ($e.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName -replace 'PatternIdentifiers.Pattern', '' }) -join ','
        '{0}{1} class={2} id={3} name="{4}" [{5}]' -f ('  ' * $d), ($c.ControlType.ProgrammaticName -replace 'ControlType.', ''),
            $c.ClassName, $c.AutomationId, ($c.Name -replace "`r?`n", ' / '), $pats
        $ch = $walker.GetFirstChild($e)
        while ($ch) { walk $ch ($d + 1); $ch = $walker.GetNextSibling($ch) }
    }
    walk $Element 0
}

<#
.SYNOPSIS 子孫要素を名前（ワイルドカード可）・AutomationId・コントロール種別で探す。見つかるまで待つ。
#>
function Find-XljsElement {
    param([Parameter(Mandatory)][System.Windows.Automation.AutomationElement]$Root,
          [string]$Name, [string]$AutomationId, [string]$ControlType, [string]$ClassName, [int]$TimeoutSec = 10)
    Wait-Until -TimeoutSec $TimeoutSec -What "element name='$Name' id='$AutomationId' type='$ControlType' class='$ClassName'" {
        $Root.FindAll('Descendants', [System.Windows.Automation.Condition]::TrueCondition) | Where-Object {
            $c = $_.Current
            (-not $Name -or $c.Name -like $Name) -and
            (-not $AutomationId -or $c.AutomationId -eq $AutomationId) -and
            (-not $ClassName -or $c.ClassName -eq $ClassName) -and
            (-not $ControlType -or $c.ControlType.ProgrammaticName -eq "ControlType.$ControlType")
        } | Select-Object -First 1
    }
}

<#
.SYNOPSIS ボタンなどを押す。Invoke → Toggle → SelectionItem → ネイティブボタンへの BM_CLICK の順に試す。
xljs の起動画面のボタンは UIA では Pane に見えてパターンがないので BM_CLICK になる。
#>
function Invoke-XljsElement {
    param([Parameter(Mandatory, ValueFromPipeline)][System.Windows.Automation.AutomationElement]$Element)
    process {
        $p = $null
        if ($Element.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$p)) { $p.Invoke(); return }
        if ($Element.TryGetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern, [ref]$p)) { $p.Toggle(); return }
        if ($Element.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$p)) { $p.Select(); return }
        $h = $Element.Current.NativeWindowHandle
        # Button（標準）、CCPushButton（IProgressDialog のキャンセル）など
        if ($h -and $Element.Current.ClassName -like '*Button') {
            # SendMessage だと押した先でモーダルループに入ったまま戻らないので Post
            [void][XljsGui.Native]::PostMessage([IntPtr]$h, $script:BM_CLICK, [IntPtr]0, [IntPtr]0); return
        }
        throw "cannot invoke '$($Element.Current.Name)' ($($Element.Current.ControlType.ProgrammaticName))"
    }
}

<#
.SYNOPSIS テキスト入力欄に値を設定する（ValuePattern、なければ WM_SETTEXT）。
#>
function Set-XljsText {
    param([Parameter(Mandatory)][System.Windows.Automation.AutomationElement]$Element, [Parameter(Mandatory)][AllowEmptyString()][string]$Value)
    $p = $null
    if ($Element.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$p)) { $p.SetValue($Value); return }
    $h = $Element.Current.NativeWindowHandle
    if ($h) { [void][XljsGui.Native]::SendMessageText([IntPtr]$h, $script:WM_SETTEXT, [IntPtr]0, $Value); return }
    throw "cannot set text of '$($Element.Current.Name)'"
}

<#
.SYNOPSIS 要素のテキストを取る（ValuePattern の値、なければ Name）。
#>
function Get-XljsText {
    param([Parameter(Mandatory, ValueFromPipeline)][System.Windows.Automation.AutomationElement]$Element)
    process {
        $p = $null
        if ($Element.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$p)) { return $p.Current.Value }
        $Element.Current.Name
    }
}

<#
.SYNOPSIS ウィンドウを閉じる（WindowPattern.Close、なければ WM_CLOSE）。
#>
function Close-XljsWindow {
    param([Parameter(Mandatory, ValueFromPipeline)][System.Windows.Automation.AutomationElement]$Window)
    process {
        $p = $null
        if ($Window.TryGetCurrentPattern([System.Windows.Automation.WindowPattern]::Pattern, [ref]$p)) { $p.Close(); return }
        [void][XljsGui.Native]::PostMessage([IntPtr]$Window.Current.NativeWindowHandle, $script:WM_CLOSE, [IntPtr]0, [IntPtr]0)
    }
}

<#
.SYNOPSIS 共通ファイルダイアログ（IFileDialog）に パスを入れて確定する。開く・保存・フォルダー選択のどれでも使える。
ファイル名欄（クラス Edit。開く・保存では AutomationId 1001、フォルダー選択では 1152）に入れ、既定ボタン（AutomationId 1）を押す。
#>
function Submit-XljsFileDialog {
    param([Parameter(Mandatory)][System.Windows.Automation.AutomationElement]$Dialog, [Parameter(Mandatory)][string]$Path)
    # 標準コントロールは UIA で Pane に見えることがあるので ClassName で探す
    $edit = $Dialog.FindAll('Descendants', [System.Windows.Automation.Condition]::TrueCondition) | Where-Object {
        $_.Current.ClassName -eq 'Edit' -and $_.Current.AutomationId -in '1152', '1001'
    } | Select-Object -First 1
    if (-not $edit) { throw 'file name box not found' }
    # ダイアログは / 区切りを受け付けない（「フォルダー名は有効ではありません」）
    Set-XljsText $edit ([System.IO.Path]::GetFullPath($Path))
    Find-XljsElement -Root $Dialog -AutomationId '1' -ClassName Button | Invoke-XljsElement
}

<#
.SYNOPSIS 作業中だけスリープ・スクリーンセーバー（＝ロック）を止める。終わったら Stop-XljsAwake。
-MonitorOff で画面だけ消す。設定はファイルに書かないので、戻し損ねても次回サインインで元に戻る。
#>
function Start-XljsAwake {
    param([int]$Minutes = 60, [switch]$MonitorOff)
    $argv = @('-NoProfile', '-File', (Join-Path $PSScriptRoot 'stay-awake.ps1'), '-Minutes', $Minutes)
    if ($MonitorOff) { $argv += '-MonitorOff' }
    $script:AwakeJob = Start-Process pwsh -WindowStyle Hidden -PassThru -ArgumentList $argv
    $script:AwakeJob
}
function Stop-XljsAwake {
    New-Item -ItemType File -Force (Join-Path $env:TEMP 'xljs-stay-awake.stop') | Out-Null
    if ($script:AwakeJob) { $script:AwakeJob.WaitForExit(10000) | Out-Null; $script:AwakeJob = $null }
}

Export-ModuleMember -Function Wait-Until, Save-XljsScreenshot, Get-XljsWindow, Show-XljsUiTree, Find-XljsElement, Invoke-XljsElement,
    Set-XljsText, Get-XljsText, Close-XljsWindow, Submit-XljsFileDialog, Start-XljsAwake, Stop-XljsAwake
