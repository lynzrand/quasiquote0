# Small USB bootstrap for browser firmware. Wi-Fi keys stay in this process.
[CmdletBinding()]
param(
    [string]$Port = 'COM4',
    [string]$WifiProfile,
    [switch]$ClearWifi,
    [ValidateRange(0, 120)][int]$WaitForNetworkSeconds = 0,
    [switch]$OpenBrowser,
    [switch]$ShowAccess,
    # For local callers only: the returned object includes the device access code.
    [switch]$PassThru
)
$ErrorActionPreference = 'Stop'
if ($ClearWifi -and $WifiProfile) { throw 'Choose a Wi-Fi profile or clear Wi-Fi.' }
$usb = [IO.Ports.SerialPort]::new($Port, 115200)
$usb.DtrEnable = $false
$usb.RtsEnable = $false
$usb.Encoding = [Text.UTF8Encoding]::new($false)
$usb.NewLine = "`n"
$usb.ReadTimeout = 5000
$usb.WriteTimeout = 3000
function Invoke-Setup($Command) {
    $line = $Command | ConvertTo-Json -Compress -Depth 4
    if ([Text.Encoding]::UTF8.GetByteCount($line) -gt 1024) { throw 'USB command too long.' }
    $usb.WriteLine($line)
    $deadline = [DateTime]::UtcNow.AddSeconds(8)
    do {
        $text = $usb.ReadLine()
        # A hardware reset can emit a ROM banner before application replies.
        if ($text.TrimStart().StartsWith('{')) { break }
    } while ([DateTime]::UtcNow -lt $deadline)
    $response = $text | ConvertFrom-Json
    if (-not $response.ok) { throw $response.error }
    return $response.result
}
try {
    $usb.Open()
    $usb.DiscardInBuffer()
    # Flush a partial command left by a disconnected client.
    $usb.WriteLine('')
    if ($WifiProfile) {
        $privateDir = Join-Path $env:TEMP ([Guid]::NewGuid().ToString())
        New-Item -ItemType Directory -Path $privateDir | Out-Null
        try {
            # Never send netsh output or the exported key to the transcript.
            $null = & netsh wlan export profile "name=$WifiProfile" key=clear "folder=$privateDir" 2>&1
            if ($LASTEXITCODE -ne 0) { throw 'Cannot read the selected saved Wi-Fi profile.' }
            $files = @(Get-ChildItem -LiteralPath $privateDir -Filter '*.xml')
            if ($files.Count -ne 1) { throw 'Expected exactly one exported Wi-Fi profile.' }
            [xml]$profile = Get-Content -LiteralPath $files[0].FullName -Raw -Encoding UTF8
            $ssid = [string]$profile.WLANProfile.SSIDConfig.SSID.name
            $security = $profile.WLANProfile.MSM.security
            if ($security.authEncryption.authentication -notin @('WPA2PSK', 'WPAPSK', 'open')) {
                throw 'Only personal-password or open Wi-Fi profiles are supported.'
            }
            $password = [string]$security.sharedKey.keyMaterial
            if ($security.sharedKey.protected -eq 'true') { throw 'Windows did not release the profile key.' }
            $null = Invoke-Setup @{cmd='wifi'; ssid=$ssid; password=$password}
        } finally {
            $password = $null
            $profile = $null
            Remove-Item -LiteralPath $privateDir -Recurse -Force
        }
    } elseif ($ClearWifi) { $null = Invoke-Setup @{cmd='clear_wifi'} }
    $deadline = [DateTime]::UtcNow.AddSeconds($WaitForNetworkSeconds)
    do {
        $status = (Invoke-Setup @{cmd='status'}).status
        $expectedMode = if ($WifiProfile) { 'station' } elseif ($ClearWifi) { 'setup' } else { $null }
        if (($status.network.connected -and (-not $expectedMode -or $status.network.mode -eq $expectedMode)) -or [DateTime]::UtcNow -ge $deadline) { break }
        Start-Sleep -Milliseconds 300
    } while ($true)
    if ($OpenBrowser) {
        if (-not $status.network.url) { throw 'No browser address yet; inspect the network error or retry.' }
        Start-Process ($status.network.url + '#token=' + $status.access_token)
    }
    if ($PassThru) { return $status }
    $safe = [ordered]@{device=$status.device; network=$status.network; bluetooth=$status.bluetooth; clock=$status.clock;
        power=$status.power; pull=$status.pull; operation_active=$status.operation_active; memory=$status.memory; display=$status.display; ota=$status.ota; recovery_error=$status.recovery_error}
    if ($ShowAccess) { $safe.access_token=$status.access_token; $safe.setup=$status.setup }
    $safe | ConvertTo-Json -Depth 8
} finally { $usb.Dispose() }
