# Installs Hoista's hoist and hoistd on Windows from a signed GitHub release:
#
#   .\install.ps1 -Version v0.1.0-beta.3
#
# The release is checked with cosign before anything is copied, so cosign 3
# has to be installed first. -NoService skips starting Hoista at login.
param(
    [Parameter(Mandatory = $true)][string]$Version,
    [string]$Destination = "$env:LOCALAPPDATA\Hoista\bin",
    [switch]$NoService
)
$ErrorActionPreference = 'Stop'

$Repository = 'TheDhejavu/hoista'
$Archive = 'hoista-x86_64-pc-windows-msvc.tar.gz'
$Programs = @('hoist.exe', 'hoistd.exe')
$Notices = @('LICENSE', 'THIRD-PARTY-NOTICES.txt', 'DEPENDENCIES.json')
$ReleaseFiles = $Programs + $Notices

if ($Version -notmatch '^v[0-9][A-Za-z0-9.-]*$') {
    throw 'Give a release version such as v0.1.0-beta.3.'
}
if (-not (Get-Command cosign -ErrorAction SilentlyContinue)) {
    throw 'Install cosign 3 first. Hoista checks every release signature before installing it.'
}

$work = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $work | Out-Null
try {
    $base = "https://github.com/$Repository/releases/download/$Version"
    Invoke-WebRequest "$base/$Archive" -OutFile "$work\$Archive"
    Invoke-WebRequest "$base/$Archive.sigstore.json" -OutFile "$work\signature.json"

    cosign verify-blob `
        --bundle "$work\signature.json" `
        --certificate-identity "https://github.com/$Repository/.github/workflows/release.yml@refs/tags/$Version" `
        --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' `
        "$work\$Archive"
    if ($LASTEXITCODE -ne 0) { throw 'The release signature did not check out. Nothing was installed.' }

    # Only the files a release has, and only ordinary files.
    $entries = tar -tzf "$work\$Archive"
    if ($LASTEXITCODE -ne 0 -or ($entries | Where-Object { $_ -notin $ReleaseFiles })) {
        throw 'The release archive has files it should not. Nothing was installed.'
    }
    tar -xzf "$work\$Archive" -C $work @ReleaseFiles
    if ($LASTEXITCODE -ne 0) { throw 'The release archive could not be unpacked.' }
    foreach ($name in $ReleaseFiles) {
        $file = Get-Item -LiteralPath (Join-Path $work $name)
        if ($file.PSIsContainer -or ($file.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw 'The release archive has a folder or link where a file should be.'
        }
    }

    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
    foreach ($name in $Programs) {
        Copy-Item -LiteralPath (Join-Path $work $name) -Destination $Destination
    }
    $noticeFolder = Join-Path $Destination 'hoista-notices'
    New-Item -ItemType Directory -Force -Path $noticeFolder | Out-Null
    foreach ($name in $Notices) {
        Copy-Item -LiteralPath (Join-Path $work $name) -Destination $noticeFolder
    }
    Write-Host "Installed to $Destination. Add it to PATH."

    if ($NoService) {
        Write-Host "Hoista won't start at login. Run hoist service install when you want it to; hoist commands start it when needed."
    } else {
        & (Join-Path $Destination 'hoist.exe') service install
        if ($LASTEXITCODE -ne 0) {
            throw 'Hoista is installed, but could not be set to start at login. Run hoist service install from your own login to try again, or hoist start for now.'
        }
    }
    Write-Host 'Next: hoist accounts add, then hoist mcp config for your coding tool.'
} finally {
    Remove-Item -Recurse -Force $work
}
