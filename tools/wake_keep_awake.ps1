# Catch a Quote/0 USB wake and replace its config with USB-always-awake defaults.
# Uses Windows' built-in serial API; no Python or USB forwarding is needed.
[CmdletBinding()]
param(
    [string]$Port = 'COM4',
    [ValidateRange(1, 604800)][int]$TimeoutSeconds = 86400,
    [string]$LogPath = '',
    [switch]$SelfTest
)

$ErrorActionPreference = 'Stop'

function Write-Log([string]$Message) {
    $line = '{0} {1}' -f ([DateTimeOffset]::Now.ToString('o')), $Message
    Write-Host $line
    if ($LogPath) { Add-Content -LiteralPath $LogPath -Value $line -Encoding UTF8 }
}

function Write-Frame($Stream, [byte]$Kind, [uint16]$Flags, [uint32]$Length, [byte[]]$Payload = @()) {
    # Yamux v0, stream 0, with network byte order for all multi-byte fields.
    [byte[]]$header = @(0, $Kind, ($Flags -shr 8), ($Flags -band 255), 0, 0, 0, 0,
        (($Length -shr 24) -band 255), (($Length -shr 16) -band 255),
        (($Length -shr 8) -band 255), ($Length -band 255))
    $Stream.Write($header, 0, $header.Length)
    if ($Payload.Length) { $Stream.Write($Payload, 0, $Payload.Length) }
}

function Read-Exact($Stream, [int]$Count, [double]$Deadline) {
    $bytes = New-Object byte[] $Count
    $offset = 0
    while ($offset -lt $Count) {
        if ($clock.Elapsed.TotalSeconds -ge $Deadline) { throw 'RPC deadline exceeded' }
        $read = $Stream.Read($bytes, $offset, $Count - $offset)
        if ($read -eq 0) { throw 'USB stream closed' }
        $offset += $read
    }
    return ,$bytes
}

function Send-Rpc($Stream, [int]$Id, [string]$Method, $Params) {
    $request = @{ jsonrpc = '2.0'; id = $Id; method = $Method }
    if ($null -ne $Params) { $request.params = $Params }
    $payload = [Text.Encoding]::UTF8.GetBytes(($request | ConvertTo-Json -Depth 8 -Compress) + "`n")
    Write-Frame $Stream 0 0 $payload.Length $payload
}

function Receive-Rpc($Stream, [int]$Id) {
    $deadline = $clock.Elapsed.TotalSeconds + 2
    $line = New-Object 'System.Collections.Generic.List[byte]'
    while ($clock.Elapsed.TotalSeconds -lt $deadline) {
        $header = Read-Exact $Stream 12 $deadline
        if ($header[0] -ne 0) { throw 'Invalid Yamux version' }
        $flags = ([int]$header[2] -shl 8) -bor $header[3]
        $streamId = ([uint32]$header[4] -shl 24) -bor ([uint32]$header[5] -shl 16) -bor
            ([uint32]$header[6] -shl 8) -bor $header[7]
        $length = ([uint32]$header[8] -shl 24) -bor ([uint32]$header[9] -shl 16) -bor
            ([uint32]$header[10] -shl 8) -bor $header[11]
        if ($header[1] -eq 2 -and $streamId -eq 0) {
            if (($flags -band 2) -eq 0) { Write-Frame $Stream 2 2 $length }
            continue
        }
        if ($header[1] -eq 1) { continue } # Window updates have no payload.
        if ($header[1] -ne 0 -or $streamId -ne 0 -or $length -gt 16384) {
            throw 'Unexpected Yamux frame'
        }
        $payload = Read-Exact $Stream $length $deadline
        foreach ($b in $payload) {
            if ($b -eq 10) {
                $response = [Text.Encoding]::UTF8.GetString($line.ToArray()) | ConvertFrom-Json
                $line.Clear()
                if ($response.id -eq $Id) {
                    if ($null -ne $response.error) { throw ('RPC error: ' + $response.error.message) }
                    return $response.result
                }
            } else {
                $line.Add($b)
                if ($line.Count -gt 16384) { throw 'JSON-RPC line too long' }
            }
        }
    }
    throw 'RPC deadline exceeded'
}

$clock = [Diagnostics.Stopwatch]::StartNew()

if ($SelfTest) {
    # Exercise wire encoding and split JSON replies without opening a COM port.
    $stream = New-Object IO.MemoryStream
    Send-Rpc $stream 1 'config.set' @{ config = @{ power = @{ stay_awake_on_usb = $true } } }
    $encoded = $stream.ToArray()
    $json = [Text.Encoding]::UTF8.GetString($encoded, 12, $encoded.Length - 12) | ConvertFrom-Json
    if ($encoded[0] -ne 0 -or $encoded[1] -ne 0 -or $encoded[11] -ne ($encoded.Length - 12) -or
        $json.method -ne 'config.set' -or $json.params.config.power.stay_awake_on_usb -ne $true) {
        throw 'Request encoding failed'
    }
    $stream.Dispose()
    $stream = New-Object IO.MemoryStream
    $payload = [Text.Encoding]::UTF8.GetBytes('{"jsonrpc":"2.0","id":2,"result":{"config":{"power":{"stay_awake_on_usb":true}}}}' + "`n")
    Write-Frame $stream 1 0 262144
    Write-Frame $stream 0 0 13 $payload[0..12]
    Write-Frame $stream 0 0 ($payload.Length - 13) $payload[13..($payload.Length - 1)]
    $stream.Position = 0
    $result = Receive-Rpc $stream 2
    if ($result.config.power.stay_awake_on_usb -ne $true) { throw 'Reply decoding failed' }
    $stream.Dispose()
    Write-Host 'Self-test passed: config request, window update, and fragmented reply.'
    exit 0
}

Write-Log "Watching $Port for up to $TimeoutSeconds seconds; minimal config will reset Wi-Fi/pull settings."
$attempts = 0
$lastReport = -60.0
while ($clock.Elapsed.TotalSeconds -lt $TimeoutSeconds) {
    $usb = New-Object IO.Ports.SerialPort $Port,115200
    $usb.ReadTimeout = 300
    $usb.WriteTimeout = 300
    $usb.DtrEnable = $true
    $usb.RtsEnable = $true
    $opened = $false
    try {
        $usb.Open()
        $opened = $true
        $attempts++
        Write-Log "Opened $Port (wake attempt $attempts); sending config immediately."
        Send-Rpc $usb 1 'config.set' @{ config = @{ power = @{ stay_awake_on_usb = $true } } }
        $accepted = Receive-Rpc $usb 1
        if ($accepted.accepted -ne $true) { throw 'Configuration was not accepted' }
        Send-Rpc $usb 2 'config.get' $null
        $saved = Receive-Rpc $usb 2
        if ($saved.config.power.stay_awake_on_usb -ne $true) { throw 'USB awake setting did not persist' }
        # The scheduler runs once a second. Check again after it has had time
        # to observe the setting, and confirm VBUS is actually detected.
        Start-Sleep -Seconds 3
        Send-Rpc $usb 3 'device.status' $null
        $status = Receive-Rpc $usb 3
        if ($status.battery.usb_present -ne $true) { throw 'Device does not detect USB power' }
        Write-Log "SUCCESS: saved stay_awake_on_usb=true; device still responds and detects USB power on $Port."
        exit 0
    } catch {
        if ($opened) {
            Write-Log ('Wake attempt failed: ' + $_.Exception.Message)
        } elseif ($clock.Elapsed.TotalSeconds - $lastReport -ge 60) {
            Write-Log "Waiting for $Port to become available."
            $lastReport = $clock.Elapsed.TotalSeconds
        }
    } finally {
        if ($usb.IsOpen) { $usb.Close() }
        $usb.Dispose()
    }
    Start-Sleep -Milliseconds 50
}
Write-Log 'Timed out without confirming an awake device.'
exit 1
