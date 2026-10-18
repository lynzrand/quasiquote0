# Flash the clock app over Windows JTAG, keeping the recovered bootloader/layout.
[CmdletBinding()]
param([string]$Port = 'COM4')
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$image = Join-Path $repo 'target\clock-flash\quote0-clock.bin'
if (-not (Test-Path -LiteralPath $image)) { throw 'Build and convert the clock image first.' }
if ((Get-Item -LiteralPath $image).Length -ge 0x1e0000) { throw 'Clock image exceeds ota_0.' }
$stage = Join-Path $env:TEMP "quote0-clock-flash-$PID"
New-Item -ItemType Directory -Path $stage | Out-Null
Copy-Item -LiteralPath "$repo\target\windows-openocd\openocd-esp32" -Destination $stage -Recurse
Copy-Item -LiteralPath $image -Destination "$stage\quote0-clock.bin"
$config = Join-Path $stage 'flash-clock.cfg'
$imageTcl = "$stage\quote0-clock.bin".Replace('\', '/')
@"
gdb port disabled
telnet port disabled
tcl port disabled
source [find board/esp32c3-builtin.cfg]
init
halt 1000
reset halt
program_esp {$imageTcl} 0x10000 verify
echo {CLOCK_FLASH_VERIFIED}
reset run
shutdown
"@ | Set-Content -LiteralPath $config -Encoding ASCII
$openocd = "$stage\openocd-esp32\bin\openocd.exe"
$scripts = "$stage\openocd-esp32\share\openocd\scripts"
$ErrorActionPreference = 'Continue'
$output = & $openocd -s $scripts -f $config 2>&1
$result = $LASTEXITCODE
$ErrorActionPreference = 'Stop'
$transcript = ($output | ForEach-Object { $_.ToString() }) -join "`n"
$transcript | Set-Content -LiteralPath "$repo\target\clock-flash\jtag-flash.log" -Encoding UTF8
if ($result -ne 0 -or -not $transcript.Contains('CLOCK_FLASH_VERIFIED')) {
    Write-Host $transcript
    throw "Clock flash failed (exit $result)."
}
Write-Host 'Clock app flash verified. Synchronizing time and checking the display.'
Start-Sleep -Seconds 1
& "$PSScriptRoot\clock_usb.ps1" -Port $Port -SyncTime -WaitForDisplaySeconds 30
