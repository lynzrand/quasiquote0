# Flash and synchronize the standalone clock using the recovered board layout.
[CmdletBinding()]
param([string]$Port = 'COM4')
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
& "$PSScriptRoot\flash_app_windows.ps1" -Port $Port -Image "$repo\target\clock-flash\quote0-clock.bin"
