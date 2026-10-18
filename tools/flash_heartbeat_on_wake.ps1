# Flash the USB-only diagnostic on the next COM4 wake. Run on Windows.
[CmdletBinding()]
param(
    [string]$Port = 'COM4',
    [string]$Espflash = "$env:USERPROFILE\.cargo\bin\espflash.exe",
    [ValidateRange(1, 604800)][int]$TimeoutSeconds = 86400,
    [string]$LogPath = ''
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
$build = Join-Path $repoRoot 'target\usb-heartbeat'
$image = Join-Path $build 'quote0_usb_heartbeat.elf'
$bootloader = Join-Path $build 'bootloader\bootloader.bin'
$partitions = Join-Path $repoRoot 'partitions.csv'
foreach ($file in @($Espflash, $image, $bootloader, $partitions)) {
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "Missing $file" }
}

function Write-Log([string]$Message) {
    $line = '{0} {1}' -f ([DateTimeOffset]::Now.ToString('o')), $Message
    Write-Host $line
    if ($LogPath) { Add-Content -LiteralPath $LogPath -Value $line -Encoding UTF8 }
}

function Enter-DownloadMode {
    # Espressif's USB Serial/JTAG reset sequence, applied before starting the
    # external flasher so process startup cannot consume the wake window.
    $usb = New-Object IO.Ports.SerialPort $Port,115200
    $usb.DtrEnable = $false
    $usb.RtsEnable = $false
    try {
        $usb.Open()
        $usb.RtsEnable = $false
        $usb.DtrEnable = $false
        Start-Sleep -Milliseconds 100
        $usb.DtrEnable = $true
        $usb.RtsEnable = $false
        Start-Sleep -Milliseconds 100
        $usb.RtsEnable = $true
        $usb.DtrEnable = $false
        $usb.RtsEnable = $true
        Start-Sleep -Milliseconds 100
        $usb.DtrEnable = $false
        $usb.RtsEnable = $false
    } finally {
        if ($usb.IsOpen) { $usb.Close() }
        $usb.Dispose()
    }
}

$clock = [Diagnostics.Stopwatch]::StartNew()
$lastReport = -60.0
Write-Log "Waiting for $Port to flash USB-only heartbeat; existing config and frames will be retained."
while ($clock.Elapsed.TotalSeconds -lt $TimeoutSeconds) {
    if ([IO.Ports.SerialPort]::GetPortNames() -contains $Port) {
        Write-Log "Detected $Port; entering ROM download mode and flashing."
        try {
            Enter-DownloadMode
        } catch {
            Write-Log ('Could not enter download mode: ' + $_.Exception.Message)
            Start-Sleep -Milliseconds 50
            continue
        }
        # OTA metadata is reset so ota_0 is selected. NVS and LittleFS stay intact.
        $flashArgs = @('flash', '--port', $Port, '--chip', 'esp32c3',
            '--before', 'no-reset', '--after', 'hard-reset',
            '--flash-size', '4mb', '--flash-mode', 'dio', '--flash-freq', '40mhz',
            '--bootloader', $bootloader, '--partition-table', $partitions,
            '--target-app-partition', 'ota_0', '--erase-parts', 'otadata', $image)
        $ErrorActionPreference = 'Continue'
        $output = & $Espflash @flashArgs 2>&1
        $result = $LASTEXITCODE
        $ErrorActionPreference = 'Stop'
        foreach ($line in $output) { Write-Log $line.ToString() }
        if ($result -eq 0) {
            Write-Log 'Flash succeeded and was verified by espflash; checking USB heartbeat.'
            $deadline = $clock.Elapsed.TotalSeconds + 15
            while ($clock.Elapsed.TotalSeconds -lt $deadline) {
                $usb = New-Object IO.Ports.SerialPort $Port,115200
                $usb.DtrEnable = $false
                $usb.RtsEnable = $false
                try {
                    $usb.Open()
                    $received = ''
                    while ($clock.Elapsed.TotalSeconds -lt $deadline) {
                        $received += $usb.ReadExisting()
                        if ([regex]::Matches($received, 'alive\r?\n').Count -ge 3) {
                            Write-Log 'SUCCESS: received three USB alive heartbeats after flashing.'
                            exit 0
                        }
                        Start-Sleep -Milliseconds 100
                    }
                } catch {
                    Write-Log ('Waiting for heartbeat: ' + $_.Exception.Message)
                } finally {
                    if ($usb.IsOpen) { $usb.Close() }
                    $usb.Dispose()
                }
                Start-Sleep -Milliseconds 100
            }
            Write-Log 'Firmware flashed, but USB heartbeat was not confirmed.'
            exit 2
        }
        Write-Log "Flash attempt failed (exit $result); waiting for another connection."
        Start-Sleep -Seconds 1
    } elseif ($clock.Elapsed.TotalSeconds - $lastReport -ge 60) {
        Write-Log "Waiting for $Port."
        $lastReport = $clock.Elapsed.TotalSeconds
    }
    Start-Sleep -Milliseconds 50
}
Write-Log 'Timed out before flashing succeeded.'
exit 1
