# Read or synchronize the clock or frame-client firmware over its Yamux USB channel.
[CmdletBinding()]
param(
    [string]$Port = 'COM4',
    [switch]$SyncTime,
    [ValidateRange(0, 120)][int]$WaitForDisplaySeconds = 0
)
$ErrorActionPreference = 'Stop'
$clock = [Diagnostics.Stopwatch]::StartNew()

function Write-Frame([byte]$Kind, [uint16]$Flags, [uint32]$Length, [byte[]]$Payload = @()) {
    [byte[]]$header = @(0, $Kind, ($Flags -shr 8), ($Flags -band 255), 0, 0, 0, 0,
        (($Length -shr 24) -band 255), (($Length -shr 16) -band 255),
        (($Length -shr 8) -band 255), ($Length -band 255))
    $usb.Write($header, 0, $header.Length)
    if ($Payload.Length) { $usb.Write($Payload, 0, $Payload.Length) }
}

function Read-Exact([int]$Count, [double]$Deadline) {
    $bytes = [byte[]]::new($Count)
    $offset = 0
    while ($offset -lt $Count) {
        if ($clock.Elapsed.TotalSeconds -ge $Deadline) { throw 'USB reply deadline exceeded' }
        $n = $usb.Read($bytes, $offset, $Count - $offset)
        if ($n -eq 0) { throw 'USB stream closed' }
        $offset += $n
    }
    return ,$bytes
}

function Invoke-Rpc([int]$Id, [string]$Method, $Params) {
    $request = @{ jsonrpc = '2.0'; id = $Id; method = $Method }
    if ($null -ne $Params) { $request.params = $Params }
    $payload = [Text.Encoding]::UTF8.GetBytes(($request | ConvertTo-Json -Compress -Depth 8) + "`n")
    Write-Frame 0 0 $payload.Length $payload
    $deadline = $clock.Elapsed.TotalSeconds + 5
    $line = [System.Collections.Generic.List[byte]]::new()
    while ($clock.Elapsed.TotalSeconds -lt $deadline) {
        $header = Read-Exact 12 $deadline
        if ($header[0] -ne 0 -or ($header[4..7] | Where-Object { $_ -ne 0 })) {
            throw 'Unexpected Yamux version or stream'
        }
        $flags = ([int]$header[2] -shl 8) -bor $header[3]
        $length = ([uint32]$header[8] -shl 24) -bor ([uint32]$header[9] -shl 16) -bor
            ([uint32]$header[10] -shl 8) -bor $header[11]
        if ($header[1] -eq 1) { continue }
        if ($header[1] -eq 2) {
            if (($flags -band 2) -eq 0) { Write-Frame 2 2 $length }
            continue
        }
        if ($header[1] -ne 0 -or $length -gt 16384) { throw 'Unexpected Yamux frame' }
        $payload = Read-Exact $length $deadline
        foreach ($b in $payload) {
            if ($b -eq 10) {
                $response = [Text.Encoding]::UTF8.GetString($line.ToArray()) | ConvertFrom-Json
                $line.Clear()
                if ($response.id -eq $Id) {
                    if ($null -ne $response.error) { throw $response.error.message }
                    return $response.result
                }
            } else {
                $line.Add($b)
                if ($line.Count -gt 16384) { throw 'JSON-RPC reply too large' }
            }
        }
    }
    throw 'USB reply deadline exceeded'
}

$usb = [IO.Ports.SerialPort]::new($Port, 115200)
$usb.DtrEnable = $false
$usb.RtsEnable = $false
$usb.ReadTimeout = 2000
$usb.WriteTimeout = 2000
try {
    $usb.Open()
    # This application emits only replies. Discard any preceding ROM banner.
    $usb.DiscardInBuffer()
    if ($SyncTime) {
        $result = Invoke-Rpc 1 'clock.set' @{ unix_seconds = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds() }
        if ($result.accepted -ne $true) { throw 'Clock synchronization was not accepted' }
    }
    $deadline = $clock.Elapsed.TotalSeconds + $WaitForDisplaySeconds
    $id = 2
    do {
        $status = Invoke-Rpc $id 'clock.get' $null
        $id++
        if ($status.display.time -eq $status.time -and $null -eq $status.display.error) { break }
        if ($WaitForDisplaySeconds -eq 0 -or $clock.Elapsed.TotalSeconds -ge $deadline) { break }
        Start-Sleep -Milliseconds 250
    } while ($true)
    $status | ConvertTo-Json -Depth 8
    if ($WaitForDisplaySeconds -gt 0 -and
        ($status.display.time -ne $status.time -or $null -ne $status.display.error)) {
        throw 'Display did not acknowledge the current clock face'
    }
} finally {
    if ($usb.IsOpen) { $usb.Close() }
    $usb.Dispose()
}
