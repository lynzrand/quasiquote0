# Repeatedly connect and halt over native Windows USB JTAG, then flash.
[CmdletBinding()]
param(
    [string]$Port = 'COM4',
    [ValidateRange(1, 604800)][int]$TimeoutSeconds = 86400,
    [ValidateRange(0, 1000)][int]$RetryDelayMilliseconds = 0,
    [ValidateRange(1, 3600)][int]$ReportIntervalSeconds = 60,
    [string]$LogPath = ''
)
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent

function Write-Log([string]$Message) {
    $line = '{0} {1}' -f ([DateTimeOffset]::Now.ToString('o')), $Message
    Write-Host $line
    if ($LogPath) { Add-Content -LiteralPath $LogPath -Value $line -Encoding UTF8 }
}

# Stage on the Windows disk so loading OpenOCD/DLLs does not use the WSL
# network share during the brief window. All files stay in a unique temp dir.
$stage = Join-Path $env:TEMP "quote0-jtag-$PID"
New-Item -ItemType Directory -Path $stage | Out-Null
Copy-Item -LiteralPath (Join-Path $repoRoot 'target\windows-openocd\openocd-esp32') -Destination $stage -Recurse
$firmware = Join-Path $stage 'firmware'
New-Item -ItemType Directory -Path $firmware | Out-Null
foreach ($relative in @('bootloader\bootloader.bin', 'partition_table\partition-table.bin',
    'quote0_usb_heartbeat.bin', 'ota_data_initial.bin', 'flasher_args.json')) {
    $destination = Join-Path $firmware $relative
    New-Item -ItemType Directory -Path (Split-Path $destination -Parent) -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path "$repoRoot\target\usb-heartbeat" $relative) -Destination $destination
}
$openocd = Join-Path $stage 'openocd-esp32\bin\openocd.exe'
$scripts = Join-Path $stage 'openocd-esp32\share\openocd\scripts'
$config = Join-Path $stage 'catch-and-flash.cfg'
$firmwareTcl = $firmware.Replace('\', '/')
@"
gdb port disabled
telnet port disabled
tcl port disabled
source [find board/esp32c3-builtin.cfg]
init
halt 1000
echo {CAUGHT: CPU halted over JTAG}
reset halt
if {[program_esp_bins {$firmwareTcl} flasher_args.json verify reset] != 0} {
    shutdown error
}
echo {FLASH_VERIFIED}
shutdown
"@ | Set-Content -LiteralPath $config -Encoding ASCII

$clock = [Diagnostics.Stopwatch]::StartNew()
$attempts = 0
$lastReport = 0.0
$lastReportAttempts = 0
$startInfo = [Diagnostics.ProcessStartInfo]::new()
$startInfo.FileName = $openocd
$startInfo.Arguments = '-s "' + $scripts + '" -f "' + $config + '"'
$startInfo.UseShellExecute = $false
$startInfo.CreateNoWindow = $true
$startInfo.RedirectStandardError = $true
$startInfo.RedirectStandardOutput = $true
Write-Log "ARMED: continuously trying JTAG init/halt, then flash; retry delay ${RetryDelayMilliseconds}ms; staged in $stage."
while ($clock.Elapsed.TotalSeconds -lt $TimeoutSeconds) {
    $attempts++
    # init cannot be retried safely in the same OpenOCD process after a
    # missing adapter, so create a fresh process for each attempt.
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    $output = [System.Collections.Generic.List[string]]::new()
    try {
        $null = $process.Start()
        $stdout = $process.StandardOutput.ReadToEndAsync()
        while (-not $process.StandardError.EndOfStream) {
            $line = $process.StandardError.ReadLine()
            $output.Add($line)
            # Avoid OpenOCD's slow target cleanup when the USB adapter is
            # absent. Never interrupt a connected target or an active flash.
            if ($line.Contains('esp_usb_jtag: could not find or open device!')) {
                # It can exit between emitting this line and our Kill call.
                try { $process.Kill() } catch [System.ComponentModel.Win32Exception] {
                } catch [System.InvalidOperationException] {
                } catch [System.Management.Automation.MethodInvocationException] {
                    if (-not $process.HasExited) { throw }
                }
                break
            }
            if ($line.Contains('CAUGHT:')) { Write-Log $line }
        }
        $process.WaitForExit()
        $result = $process.ExitCode
        $output.Add($stdout.GetAwaiter().GetResult())
    } finally {
        $process.Dispose()
    }
    $transcript = $output -join "`n"
    if ($transcript.Contains('CAUGHT:')) {
        foreach ($line in $output) { Write-Log $line.ToString() }
    }
    if ($result -eq 0 -and $transcript.Contains('FLASH_VERIFIED')) {
        Write-Log 'JTAG flash verified; checking for three USB heartbeats.'
        $deadline = $clock.Elapsed.TotalSeconds + 20
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
                        Write-Log 'SUCCESS: JTAG flash verified and three alive heartbeats received.'
                        exit 0
                    }
                    Start-Sleep -Milliseconds 100
                }
            } catch {
                Start-Sleep -Milliseconds 100
            } finally {
                if ($usb.IsOpen) { $usb.Close() }
                $usb.Dispose()
            }
        }
        Write-Log 'Flash verified, but USB heartbeat was not confirmed.'
        exit 2
    }
    $now = $clock.Elapsed.TotalSeconds
    if ($now - $lastReport -ge $ReportIntervalSeconds) {
        $rate = ($attempts - $lastReportAttempts) / ($now - $lastReport)
        Write-Log ("Waiting for JTAG ({0} attempts; {1:F1}/second over {2:F1}s). Last exit: {3}" -f
            $attempts, $rate, ($now - $lastReport), $result)
        Set-Content -LiteralPath (Join-Path $stage 'last-attempt.log') -Value $transcript -Encoding UTF8
        $lastReport = $now
        $lastReportAttempts = $attempts
    }
    if ($RetryDelayMilliseconds -gt 0) {
        Start-Sleep -Milliseconds $RetryDelayMilliseconds
    }
}
Write-Log 'Timed out waiting for JTAG.'
exit 1
