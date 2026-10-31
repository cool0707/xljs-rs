# 実機の Windows で xljs の GUI を一通り自動操作するスモークテスト（pwsh 7）。
#   cargo build
#   pwsh -File tools/gui/smoke.ps1 [-Exe target/debug/xljs.exe] [-OutDir $env:TEMP\xljs-gui-smoke] [-MonitorOff]
# 各ケースの PASS/FAIL を出し、スクリーンショットを <OutDir>/shots に保存する。失敗があれば終了コード 1。
# 詳しくは docs/gui-testing.md。
param(
    [string]$Exe = (Join-Path $PSScriptRoot '../../target/debug/xljs.exe'),
    [string]$OutDir = (Join-Path $env:TEMP 'xljs-gui-smoke'),
    [switch]$MonitorOff
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'XljsGui.psm1') -Force
$Exe = (Resolve-Path $Exe).Path
$examples = (Resolve-Path (Join-Path $PSScriptRoot '../../examples')).Path
Remove-Item $OutDir -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$OutDir/data/sub", "$OutDir/shots" | Out-Null
$shots = "$OutDir/shots"
$data = [System.IO.Path]::GetFullPath("$OutDir/data")
$js = $OutDir.Replace('\', '/')  # スクリプトに埋め込むパス

# スクリプトをモック UI で実行する（ダイアログなし）。終了コードを返す。
function Invoke-Mock([string]$Name, [string]$Source) {
    $f = "$OutDir/$Name.js"
    Set-Content $f $Source -Encoding utf8
    $p = Start-Process $Exe -ArgumentList '--ui=mock', '--lang=ja', $f -PassThru -Wait -WindowStyle Hidden
    $p.ExitCode
}
function Start-Xljs([string[]]$ArgList) {
    $p = Start-Process $Exe -ArgumentList (@('--lang=ja') + $ArgList) -PassThru
    $script:started += $p
    $p
}
function Assert($cond, [string]$msg) { if (-not $cond) { throw "assert: $msg" } }
function Wait-Exit($p, [int]$sec = 10) { Assert ($p.WaitForExit($sec * 1000)) "xljs (pid $($p.Id)) did not exit" }
function Get-AllText($el) {
    ($el.FindAll('Descendants', [System.Windows.Automation.Condition]::TrueCondition) | ForEach-Object { $_.Current.Name } | Where-Object { $_ }) -join ' | '
}

$script:started = @()
$results = [ordered]@{}
function Test-Case([string]$Name, [scriptblock]$Body) {
    try { & $Body; $results[$Name] = 'PASS' }
    catch {
        $results[$Name] = "FAIL: $($_.Exception.Message)"
        try { Save-XljsScreenshot "$shots/FAIL-$Name.png" | Out-Null } catch {}
    }
    foreach ($p in $script:started) { if (-not $p.HasExited) { $p.Kill() } }
    $script:started = @()
    Write-Host ("{0,-20} {1}" -f $Name, $results[$Name])
}

# テスト用のブック（明細シート）
$rc = Invoke-Mock gen @"
const mk = (p, rows) => { const b = workbooks.create(); b.addSheet("明細").write(rows); b.save(p); };
mk("$js/data/a.xlsx", [{顧客名:"山田商事",金額:1200},{顧客名:"佐藤工業",金額:800}]);
mk("$js/data/sub/b.xlsx", [{顧客名:"山田商事",金額:300},{顧客名:"鈴木物産",金額:5000}]);
"@
if ($rc -ne 0) { throw "test data generation failed (exit $rc)" }

if ($MonitorOff) { Start-XljsAwake -Minutes 30 -MonitorOff | Out-Null }
try {
    # 引数なし → 開始画面（Win32 のボタン。UIA では Pane に見えるので BM_CLICK で押す）
    Test-Case launch-screen {
        $p = Start-Xljs @()
        $w = Get-XljsWindow -ProcessId $p.Id -Name 'xljs'
        Save-XljsScreenshot "$shots/launch.png" -Window $w | Out-Null
        foreach ($n in 'スクリプトを実行', 'スクリプトをデバッグ', 'exe を作成', 'VBA コードを取り出す') {
            Find-XljsElement -Root $w -Name $n -TimeoutSec 3 | Out-Null
        }
        Find-XljsElement -Root $w -Name 'キャンセル' -ClassName Button | Invoke-XljsElement
        Wait-Exit $p
    }

    # フォルダー選択 → 進捗 → 保存ダイアログ → alert → エクスプローラー表示
    Test-Case dialog-aggregate {
        $p = Start-Xljs @("$examples/dialog-aggregate.js")
        $d = Get-XljsWindow -ProcessId $p.Id -Name '集計するフォルダを選択'
        Save-XljsScreenshot "$shots/dialog-folder.png" -Window $d | Out-Null
        Submit-XljsFileDialog $d $data
        $d = Get-XljsWindow -ProcessId $p.Id -Name '名前を付けて保存' -TimeoutSec 30
        Save-XljsScreenshot "$shots/dialog-save.png" -Window $d | Out-Null
        Submit-XljsFileDialog $d "$OutDir/out1.xlsx"
        $m = Get-XljsWindow -ProcessId $p.Id -Name 'dialog-aggregate'
        Save-XljsScreenshot "$shots/dialog-alert.png" -Window $m | Out-Null
        $t = Get-AllText $m
        Assert ($t -like '*2 ファイル、4 件を集計しました*') "alert text: $t"
        Find-XljsElement -Root $m -Name 'OK' -ClassName Button | Invoke-XljsElement
        Wait-Exit $p
        # shell.showItemInFolder で開いたエクスプローラーを閉じる
        Start-Sleep 2
        $script:AE = [System.Windows.Automation.AutomationElement]
        $script:AE::RootElement.FindAll('Children', [System.Windows.Automation.Condition]::TrueCondition) |
            Where-Object { $_.Current.ClassName -eq 'CabinetWClass' -and $_.Current.Name -like '*xljs-gui-smoke*' } | Close-XljsWindow
        $rc = Invoke-Mock check1 @"
const b = workbooks.open("$js/out1.xlsx");
const s = JSON.stringify(b.sheets["集計"].toObjects());
if (s !== '[{"顧客名":"山田商事","件数":2,"合計":1500},{"顧客名":"佐藤工業","件数":1,"合計":800},{"顧客名":"鈴木物産","件数":1,"合計":5000}]') exit(2);
if (b.sheets["明細"].toObjects().length !== 4) exit(3);
"@
        Assert ($rc -eq 0) "output check failed (exit $rc)"
    }

    # WebView2 のフォーム。HTML 要素は UIA で id（AutomationId）付きで見え、Invoke/Value/Toggle が使える
    Test-Case form-aggregate {
        $p = Start-Xljs @("$examples/form-aggregate.js")
        $f = Get-XljsWindow -ProcessId $p.Id -Name '売上集計'
        $browse = Wait-Until -What '参照 buttons' {
            $b = @($f.FindAll('Descendants', [System.Windows.Automation.Condition]::TrueCondition) | Where-Object { $_.Current.Name -eq '参照…' })
            if ($b.Count -eq 2) { , $b }
        }
        $browse[0] | Invoke-XljsElement
        Submit-XljsFileDialog (Get-XljsWindow -ProcessId $p.Id -Name 'フォルダーの選択') $data
        $browse[1] | Invoke-XljsElement
        Submit-XljsFileDialog (Get-XljsWindow -ProcessId $p.Id -Name '名前を付けて保存') "$OutDir/out2.xlsx"
        Find-XljsElement -Root $f -Name '商品名' -ControlType RadioButton | Invoke-XljsElement
        # remember: true で前回値が残るので、状態を見てからそろえる
        $cb = Find-XljsElement -Root $f -AutomationId f4
        if ($cb.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern).Current.ToggleState -eq 'On') { $cb | Invoke-XljsElement }
        Wait-Until -What 'f0 value' { (Find-XljsElement -Root $f -AutomationId f0 | Get-XljsText) -eq $data } | Out-Null
        Save-XljsScreenshot "$shots/form-filled.png" -Window $f | Out-Null
        Find-XljsElement -Root $f -AutomationId submit | Invoke-XljsElement
        Find-XljsElement -Root $f -Name '完了しました' -TimeoutSec 30 | Out-Null
        Save-XljsScreenshot "$shots/form-done.png" -Window $f | Out-Null
        $t = Get-AllText $f
        Assert ($t -like '*2 ファイルが見つかりました*' -and $t -like '*4 件を集計して保存しました*') "form log: $t"
        Find-XljsElement -Root $f -AutomationId cancel | Invoke-XljsElement   # 「閉じる」
        Wait-Exit $p
        $rc = Invoke-Mock check2 @"
const b = workbooks.open("$js/out2.xlsx");
if (b.sheets["明細"]) exit(2);
if (JSON.stringify(b.sheets["集計"].toObjects()) !== '[{"商品名":"(未設定)","件数":4,"合計":7300}]') exit(3);
"@
        Assert ($rc -eq 0) "output check failed (exit $rc)"
    }

    # IProgressDialog のキャンセル（ボタンは CCPushButton）
    Test-Case progress-cancel {
        $p = Start-Xljs @("$examples/progress-cancel.js")
        $d = Get-XljsWindow -ProcessId $p.Id -Name 'デモ'
        Start-Sleep 1
        Save-XljsScreenshot "$shots/progress.png" -Window $d | Out-Null
        Find-XljsElement -Root $d -Name 'キャンセル' | Invoke-XljsElement
        $m = Get-XljsWindow -ProcessId $p.Id -Name 'progress-cancel'
        $t = Get-AllText $m
        Assert ($t -like '*キャンセルされました*') "alert text: $t"
        Find-XljsElement -Root $m -Name 'OK' -ClassName Button | Invoke-XljsElement
        Wait-Exit $p
    }
} finally {
    if ($MonitorOff) { Stop-XljsAwake }
}

$failed = @($results.Values | Where-Object { $_ -ne 'PASS' }).Count
Write-Host "`n$($results.Count - $failed) passed, $failed failed. screenshots: $shots"
exit ([int]($failed -gt 0))
