# Copyright The agentgateway Authors.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

<#
.SYNOPSIS
  Installs the agentgateway binary on Windows.

.DESCRIPTION
  Windows counterpart of tools/get-agentgateway. Downloads a release binary
  from GitHub, verifies its SHA256 checksum, installs it into a per-user
  directory (no administrator rights needed) and adds that directory to the
  user PATH.

  Install the latest stable release:

    irm https://agentgateway.dev/install.ps1 | iex

  Piping to iex cannot pass arguments, so every option is also an environment
  variable:

    $env:AGENTGATEWAY_VERSION = 'v1.7.0'; irm https://agentgateway.dev/install.ps1 | iex

  Or pass parameters directly:

    & ([scriptblock]::Create((irm https://agentgateway.dev/install.ps1))) -Version v1.7.0

.PARAMETER Version
  Release tag to install, e.g. v1.7.0. Defaults to the latest stable release.
  Environment variable: AGENTGATEWAY_VERSION.

.PARAMETER InstallDir
  Directory to install agentgateway.exe into.
  Defaults to %LOCALAPPDATA%\Programs\agentgateway.
  Environment variable: AGENTGATEWAY_INSTALL_DIR.

.PARAMETER NoModifyPath
  Do not add the install directory to the user PATH.
  Environment variable: AGENTGATEWAY_NO_MODIFY_PATH=true.

.PARAMETER SkipChecksum
  Do not verify the SHA256 checksum of the download.
  Environment variable: VERIFY_CHECKSUM=false.
#>
param(
  [string]$Version = $env:AGENTGATEWAY_VERSION,
  [string]$InstallDir = $env:AGENTGATEWAY_INSTALL_DIR,
  [switch]$NoModifyPath,
  [switch]$SkipChecksum
)

# Everything runs in a child scope so that `irm | iex` does not leak
# preferences or helper functions into the caller's session. Failures are
# thrown rather than `exit`ed, which would close the caller's shell.
& {
  $ErrorActionPreference = 'Stop'
  # The progress bar makes Invoke-WebRequest very slow on Windows PowerShell 5.1.
  $ProgressPreference = 'SilentlyContinue'

  $binaryName = 'agentgateway'
  $repo = 'agentgateway/agentgateway'

  if ($PSVersionTable.PSVersion.Major -ge 6 -and -not $IsWindows) {
    throw 'This script installs agentgateway on Windows. On Linux and macOS, run: curl -sL https://agentgateway.dev/install | bash'
  }

  # Windows PowerShell 5.1 may not offer TLS 1.2 by default; GitHub requires it.
  [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

  $noModifyPath = $NoModifyPath -or ($env:AGENTGATEWAY_NO_MODIFY_PATH -eq 'true')
  $verifyChecksum = -not ($SkipChecksum -or ($env:VERIFY_CHECKSUM -eq 'false'))
  if (-not $InstallDir) {
    $InstallDir = Join-Path $env:LOCALAPPDATA "Programs\$binaryName"
  }

  # PROCESSOR_ARCHITEW6432 holds the real architecture when running a 32-bit
  # PowerShell on a 64-bit OS.
  $osArch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
  switch ($osArch) {
    'AMD64' { $arch = 'amd64' }
    'ARM64' {
      # No native arm64 build is published; Windows on ARM runs the x64 binary under emulation.
      Write-Host 'No native windows-arm64 binary is published; installing the amd64 binary, which runs under x64 emulation.'
      $arch = 'amd64'
    }
    default {
      throw "No prebuilt binary for windows-$osArch. To build from source, go to https://github.com/$repo"
    }
  }

  function Get-LatestStableTag {
    # Pick the highest stable tag (vX.Y.Z, no suffix), skipping drafts and
    # pre-releases. Use -Version to install a pre-release.
    $url = "https://api.github.com/repos/$repo/releases?per_page=100"
    try {
      $releases = Invoke-RestMethod -Uri $url -UseBasicParsing
    } catch {
      throw "Could not retrieve the latest stable release tag information from ${url}: $($_.Exception.Message)"
    }
    $tag = $releases |
      Where-Object { -not $_.draft -and -not $_.prerelease -and $_.tag_name -match '^v\d+\.\d+\.\d+$' } |
      ForEach-Object { $_.tag_name } |
      Sort-Object { [version]$_.Substring(1) } |
      Select-Object -Last 1
    if (-not $tag) {
      throw "Could not find a stable release in $url"
    }
    $tag
  }

  # Different versions print the version in different formats.
  function Get-InstalledVersion([string]$Exe) {
    $ErrorActionPreference = 'Continue'
    try {
      $output = (& $Exe --version 2>$null | Out-String).Trim()
    } catch {
      return $null
    }
    try {
      return ($output | ConvertFrom-Json).version
    } catch {}
    if ($output -match 'GitTag:"([^"]*)"') {
      return $Matches[1]
    }
    $null
  }

  function Get-File([string]$Url, [string]$Dest) {
    Invoke-WebRequest -Uri $Url -OutFile $Dest -UseBasicParsing
  }

  function Add-ToUserPath([string]$Dir) {
    $trimmed = $Dir.TrimEnd('\')
    $onPath = { param($value) @($value -split ';' | Where-Object { $_.TrimEnd('\') -ieq $trimmed }).Count -gt 0 }

    # Read and write the raw registry value: going through
    # [Environment]::GetEnvironmentVariable would expand %VARIABLES% in the
    # existing PATH and then persist the expanded form.
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
    try {
      $current = [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
      if (-not (& $onPath $current)) {
        $kind = [Microsoft.Win32.RegistryValueKind]::ExpandString
        if ($current) { $kind = $key.GetValueKind('Path') }
        $new = if ($current) { $current.TrimEnd(';') + ';' + $Dir } else { $Dir }
        $key.SetValue('Path', $new, $kind)
        # Writing any user variable through the .NET API broadcasts
        # WM_SETTINGCHANGE, so newly started programs see the updated PATH.
        $dummy = 'AGENTGATEWAY_INSTALL_' + [guid]::NewGuid().ToString('N')
        [Environment]::SetEnvironmentVariable($dummy, '1', 'User')
        [Environment]::SetEnvironmentVariable($dummy, $null, 'User')
        Write-Host "Added $Dir to your user PATH. Open a new terminal for other sessions to pick it up."
      }
    } finally {
      $key.Close()
    }

    # Make the binary usable in the current session too.
    if (-not (& $onPath $env:Path)) {
      $env:Path = $env:Path.TrimEnd(';') + ';' + $Dir
    }
  }

  if ($Version) {
    if (-not $Version.StartsWith('v')) {
      Write-Host "Expected version arg ('$Version') to begin with 'v', fixing..."
      $Version = "v$Version"
    }
    $tag = $Version
  } else {
    $tag = Get-LatestStableTag
  }

  $dist = "$binaryName-windows-$arch.exe"
  $downloadUrl = "https://github.com/$repo/releases/download/$tag/$dist"
  $target = Join-Path $InstallDir "$binaryName.exe"
  $tmpRoot = Join-Path ([IO.Path]::GetTempPath()) ("$binaryName-installer-" + [guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Path $tmpRoot | Out-Null
  try {
    $expected = $null
    if ($verifyChecksum) {
      $sumFile = Join-Path $tmpRoot "$dist.sha256"
      try {
        Get-File "$downloadUrl.sha256" $sumFile
      } catch {
        throw "Failed to download checksum $downloadUrl.sha256: $($_.Exception.Message)`nRelease $tag may not ship a windows-$arch binary. See https://github.com/$repo/releases"
      }
      $expected = ((Get-Content -LiteralPath $sumFile -Raw).Trim() -split '\s+')[0]
      if ($expected -notmatch '^[0-9a-fA-F]{64}$') {
        throw "Checksum file $downloadUrl.sha256 does not contain a SHA256 sum."
      }
    }

    $needsInstall = $true
    if (Test-Path -LiteralPath $target) {
      # Prefer comparing against the release checksum: it identifies the exact
      # binary, whereas the version the binary reports is not always the tag.
      $installed = Get-InstalledVersion $target
      $current = if ($expected) {
        (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash -ieq $expected
      } else {
        $installed -eq $tag
      }
      if ($current) {
        $label = if ($Version) { 'installed' } else { 'the latest version' }
        Write-Host "$binaryName $tag is already $label"
        $needsInstall = $false
      } elseif ($installed -match '^v?\d+\.') {
        Write-Host "Changing $binaryName from version $installed to $tag."
      } else {
        Write-Host "Replacing the existing $binaryName with $tag."
      }
    }

    if ($needsInstall) {
      $tmpFile = Join-Path $tmpRoot $dist
      Write-Host "Downloading $downloadUrl"
      try {
        Get-File $downloadUrl $tmpFile
      } catch {
        throw "Failed to download ${downloadUrl}: $($_.Exception.Message)`nRelease $tag may not ship a windows-$arch binary. See https://github.com/$repo/releases"
      }

      if ($expected) {
        $actual = (Get-FileHash -LiteralPath $tmpFile -Algorithm SHA256).Hash
        if ($actual -ine $expected) {
          throw "SHA256 sum of $dist does not match (expected '$expected', got '$actual'). Aborting."
        }
        Write-Host 'Verifying checksum... Done.'
      }

      New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
      $stale = "$target.old"
      Remove-Item -LiteralPath $stale -Force -ErrorAction SilentlyContinue
      try {
        Copy-Item -LiteralPath $tmpFile -Destination $target -Force
      } catch {
        # A running agentgateway.exe cannot be overwritten, but it can be
        # renamed out of the way.
        if (-not (Test-Path -LiteralPath $target)) { throw }
        Move-Item -LiteralPath $target -Destination $stale -Force
        Copy-Item -LiteralPath $tmpFile -Destination $target -Force
        Write-Host "$binaryName is currently running; restart it to use the new version."
      }
      Write-Host "$binaryName.exe $tag installed into $target"
    }
  } finally {
    Remove-Item -LiteralPath $tmpRoot -Recurse -Force -ErrorAction SilentlyContinue
  }

  if ($noModifyPath) {
    if (-not (Get-Command "$binaryName.exe" -ErrorAction SilentlyContinue)) {
      Write-Host "$binaryName not found on your PATH. Add $InstallDir to your PATH to run it."
    }
  } else {
    Add-ToUserPath $InstallDir
  }
}
