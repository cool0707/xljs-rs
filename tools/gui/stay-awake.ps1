# 作業中だけ「スリープしない・ロックしない・画面は消す」状態にする。
#   pwsh -File stay-awake.ps1 [-Minutes 60] [-MonitorOff]
# 終了: Ctrl+C、または $env:TEMP\xljs-stay-awake.stop を作る、または -Minutes 経過。
# 同時に 1 つだけ動く（2 つ目は画面を消すだけで終わる）。
# 終了時にスクリーンセーバー（再開時のサインイン＝ロックの原因）をレジストリの設定どおりに戻す。
# SPI はファイルに書かないので、戻し損ねても次回サインインで元に戻る。
param([int]$Minutes = 60, [switch]$MonitorOff)

Add-Type -Namespace X -Name W -MemberDefinition @'
[DllImport("kernel32.dll")] public static extern uint SetThreadExecutionState(uint f);
[DllImport("user32.dll")] public static extern bool SystemParametersInfo(uint a, uint p, System.IntPtr v, uint w);
[DllImport("user32.dll")] public static extern bool PostMessage(System.IntPtr h, uint m, System.IntPtr w, System.IntPtr l);
'@

function Off-Monitor {
    Start-Sleep -Milliseconds 500
    # HWND_BROADCAST, WM_SYSCOMMAND, SC_MONITORPOWER, 2 = off
    [void][X.W]::PostMessage([IntPtr]0xffff, 0x0112, [IntPtr]0xF170, [IntPtr]2)
}

$mutex = New-Object System.Threading.Mutex($false, 'Local\xljs-stay-awake')
if (-not $mutex.WaitOne(0)) {
    'stay-awake: already running'
    if ($MonitorOff) { Off-Monitor }
    exit 0
}

$ES_CONTINUOUS = [uint32]2147483648; $ES_SYSTEM_REQUIRED = [uint32]1
$SPI_SETSCREENSAVEACTIVE = 17
$stop = Join-Path $env:TEMP 'xljs-stay-awake.stop'
Remove-Item $stop -ErrorAction SilentlyContinue

# 元の値は（SPI の現在値ではなく）保存されている設定から取る
$was = [int](Get-ItemProperty 'HKCU:\Control Panel\Desktop').ScreenSaveActive
try {
    [void][X.W]::SetThreadExecutionState($ES_CONTINUOUS -bor $ES_SYSTEM_REQUIRED)
    [void][X.W]::SystemParametersInfo($SPI_SETSCREENSAVEACTIVE, 0, [IntPtr]::Zero, 0)
    "stay-awake: on (screensaver setting $was, until $((Get-Date).AddMinutes($Minutes).ToString('HH:mm')))"
    if ($MonitorOff) { Off-Monitor }
    $end = (Get-Date).AddMinutes($Minutes)
    while ((Get-Date) -lt $end -and -not (Test-Path $stop)) { Start-Sleep -Seconds 2 }
} finally {
    [void][X.W]::SystemParametersInfo($SPI_SETSCREENSAVEACTIVE, [uint32]$was, [IntPtr]::Zero, 0)
    [void][X.W]::SetThreadExecutionState($ES_CONTINUOUS)
    Remove-Item $stop -ErrorAction SilentlyContinue
    $mutex.ReleaseMutex()
    "stay-awake: off (screensaver restored to $was)"
}
