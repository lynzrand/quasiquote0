# Install only Espressif's signed VID_303A/PID_1001/MI_02 WinUSB package.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$DriverDirectory,
    [string]$LogPath = '',
    [switch]$ValidateOnly
)
$ErrorActionPreference = 'Stop'
if (-not $LogPath) { $LogPath = Join-Path $DriverDirectory 'install-jtag-driver.log' }

function Write-Log([string]$Message) {
    $line = '{0} {1}' -f ([DateTimeOffset]::Now.ToString('o')), $Message
    Write-Host $line
    Add-Content -LiteralPath $LogPath -Value $line -Encoding UTF8
}

try {
    $inf = Join-Path $DriverDirectory 'USB_JTAG_debug_unit.inf'
    $catalog = Join-Path $DriverDirectory 'USB_JTAG_debug_unit.cat'
    $text = Get-Content -LiteralPath $inf -Raw
    if ($text -notmatch '(?m)^DeviceID\s*=\s*"VID_303A&PID_1001&MI_02"\s*$' -or
        $text -notmatch '(?m)^DeviceGUID\s*=\s*"\{80869ad8-c37f-476a-a6b4-ae241c30a473\}"\s*$') {
        throw 'Unexpected device ID or interface GUID in driver package.'
    }
    $signature = Get-AuthenticodeSignature -FilePath $catalog
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'Espressif Systems') {
        throw 'Driver catalog must have a valid Espressif signature.'
    }
    Write-Log 'Validated Espressif-signed JTAG-only driver package and its device-interface GUID.'
    if ($ValidateOnly) { exit 0 }

    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'Windows administrator approval is required to install the driver.'
    }
    Write-Log 'Installing JTAG interface MI_02 driver; the serial interface MI_00 is outside this package.'
    # Windows checks the catalog against the INF and driver files as it stages
    # the package. Existing driver packages remain available for rollback.
    $output = & "$env:SystemRoot\System32\pnputil.exe" /add-driver $inf /install 2>&1
    $result = $LASTEXITCODE
    foreach ($line in $output) { Write-Log $line.ToString() }
    if ($result -ne 0 -and $result -ne 3010) { throw "Driver installation failed (exit $result)." }
    Write-Log "DRIVER_STAGED: installation command completed (exit $result); verify binding on the next device appearance."
} catch {
    Write-Log ('ERROR: ' + $_.Exception.Message)
    exit 1
}
