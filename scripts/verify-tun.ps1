# TriProxy — a modern multi-core proxy client.
# Copyright (C) 2026 luoqingci
# SPDX-License-Identifier: GPL-3.0-or-later
#
# This program is free software: you can redistribute it and/or modify it under
# the terms of the GNU General Public License as published by the Free Software
# Foundation, either version 3 of the License, or (at your option) any later
# version. It is distributed in the hope that it will be useful, but WITHOUT ANY
# WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
# PARTICULAR PURPOSE. See LICENSE for the full text.
#
# Additional terms under GNU GPL v3 section 7 apply: see ADDITIONAL-TERMS.md.

<#
.SYNOPSIS
  Verifies that TUN mode creates its adapter and routes, and that a hard kill
  leaves neither behind.

.DESCRIPTION
  This is the check that turns Phase 7 from "the core accepts the config" into
  "TUN actually works". It must run elevated: creating a wintun adapter requires
  administrator rights.

  The script runs the real user flow (`triproxy start --tun`), records the
  adapter and route table while TUN is up, kills the core process outright to
  simulate a crash, and then reports what survived.

  Expect a few seconds of broken connectivity while TUN is up: the core's
  upstreams are whatever your profile contains, and TUN routes all traffic into
  it. That is inherent to the feature, not a bug in the script.

.PARAMETER Exe
  Path to triproxy.exe. Defaults to target\debug\triproxy.exe under the repo.

.PARAMETER Core
  Core to test. Defaults to sing-box, because it pins interface_name to TriProxy
  and is therefore the one whose adapter can be identified by name.

.PARAMETER InterfaceName
  Adapter name to look for. Defaults to TriProxy.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\verify-tun.ps1
#>

param(
  [string]$Exe = '',
  [string]$Core = 'sing-box',
  [string]$InterfaceName = 'TriProxy'
)

$ErrorActionPreference = 'Continue'

if (-not $Exe) {
  $Exe = Join-Path $PSScriptRoot '..\target\debug\triproxy.exe'
}
$Exe = (Resolve-Path $Exe -ErrorAction SilentlyContinue)
if (-not $Exe) {
  Write-Output "triproxy.exe not found. Build it first (cargo build) or pass -Exe."
  exit 1
}

function Log($msg) { Write-Output $msg }

$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
Log "elevated=$admin"
if (-not $admin) {
  Log 'This script must run elevated: right-click PowerShell -> Run as administrator, then re-run.'
  exit 1
}

# --- baseline ----------------------------------------------------------------
Log ''
Log '=== baseline ==='
$baseRouteCount = (Get-NetRoute -AddressFamily IPv4 | Measure-Object).Count
Log "ipv4 routes: $baseRouteCount"
$pre = Get-NetAdapter -Name $InterfaceName -ErrorAction SilentlyContinue
if ($pre) { Log "WARNING: '$InterfaceName' already exists before the test (status=$($pre.Status))" }

# --- start with TUN ----------------------------------------------------------
Log ''
Log '=== triproxy start --tun ==='
& $Exe start $Core --tun 2>&1 | ForEach-Object { Log "  $_" }
Start-Sleep -Seconds 6

# --- while TUN is up ---------------------------------------------------------
Log ''
Log '=== while TUN is up ==='
$adapter = Get-NetAdapter -Name $InterfaceName -ErrorAction SilentlyContinue
if ($adapter) {
  Log "adapter: found, status=$($adapter.Status), ifIndex=$($adapter.ifIndex)"
  Log "driver:  $($adapter.InterfaceDescription)"
  $routes = Get-NetRoute -InterfaceIndex $adapter.ifIndex -ErrorAction SilentlyContinue
  Log "routes on the adapter: $(($routes | Measure-Object).Count)"
  $routes | Select-Object -First 6 | ForEach-Object { Log "  $($_.DestinationPrefix) -> $($_.NextHop)" }
} else {
  Log "adapter: NOT FOUND — TUN did not come up (check the core log)"
}

# --- hard kill ---------------------------------------------------------------
Log ''
Log '=== killing the core (simulated crash) ==='
$statePath = Join-Path $env:APPDATA 'TriProxy\state.json'
$corePid = $null
if (Test-Path $statePath) {
  $corePid = (Get-Content $statePath -Raw | ConvertFrom-Json).pid
}
if ($corePid) {
  Log "core pid=$corePid"
  Stop-Process -Id $corePid -Force -ErrorAction SilentlyContinue
  Log 'killed without letting it clean up'
} else {
  Log 'no pid recorded; killing any sing-box process instead'
  Get-Process -Name 'sing-box' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
}
Start-Sleep -Seconds 5

# --- leftovers ---------------------------------------------------------------
Log ''
Log '=== after the kill ==='
$left = Get-NetAdapter -Name $InterfaceName -ErrorAction SilentlyContinue
if ($left) {
  Log "RESULT: adapter LEFT BEHIND (status=$($left.Status))"
  $leftRoutes = Get-NetRoute -InterfaceIndex $left.ifIndex -ErrorAction SilentlyContinue
  Log "RESULT: routes left behind: $(($leftRoutes | Measure-Object).Count)"
  $leftRoutes | Select-Object -First 6 | ForEach-Object { Log "  $($_.DestinationPrefix)" }
} else {
  Log 'RESULT: adapter gone'
}
$afterRouteCount = (Get-NetRoute -AddressFamily IPv4 | Measure-Object).Count
Log "RESULT: ipv4 routes now $afterRouteCount (baseline $baseRouteCount)"
$defaults = Get-NetRoute -DestinationPrefix '0.0.0.0/0' -ErrorAction SilentlyContinue
Log "RESULT: default routes via $(($defaults | ForEach-Object { $_.InterfaceAlias }) -join ', ')"
& $Exe tun status 2>&1 | ForEach-Object { Log "  $_" }

# --- cleanup -----------------------------------------------------------------
Log ''
Log '=== cleanup ==='
& $Exe stop 2>&1 | ForEach-Object { Log "  $_" }
Get-Process -Name 'sing-box' -ErrorAction SilentlyContinue | ForEach-Object {
  Log "stopping leftover core pid $($_.Id)"
  Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
}
Start-Sleep -Seconds 2
if (Get-NetAdapter -Name $InterfaceName -ErrorAction SilentlyContinue) {
  Log "WARNING: '$InterfaceName' still present. Remove it with: pnputil /remove-device <instance-id>"
  Log '         (see Device Manager -> Network adapters)'
} else {
  Log "clean: no '$InterfaceName' adapter"
}
Log ''
Log '=== done ==='
