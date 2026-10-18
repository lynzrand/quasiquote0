# Flash and synchronize the standalone clock using the recovered board layout.
[CmdletBinding()]
param([string]$Port = 'COM4', [ValidateSet('auto','ota_0','ota_1')][string]$Slot = 'auto')
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
& "$PSScriptRoot\flash_app_windows.ps1" -LegacyClock -Slot $Slot -Port $Port -Image "$repo\target\clock-flash\quote0-clock.bin"
