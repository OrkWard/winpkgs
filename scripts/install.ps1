# Install wpk: fetch the latest release, check its hash and run `wpk init`.
#
#   irm https://s3.orkward.dev/scoop-bucket/wpk/install.ps1 | iex
$ErrorActionPreference = 'Stop'
$base = 'https://s3.orkward.dev/scoop-bucket/wpk'
$latest = Invoke-RestMethod "$base/latest.json"
$dir = Join-Path ([IO.Path]::GetTempPath()) "wpk-install-$([guid]::NewGuid())"
New-Item -ItemType Directory $dir | Out-Null
try {
    $zip = Join-Path $dir 'wpk.zip'
    Invoke-WebRequest $latest.url -OutFile $zip -UseBasicParsing
    $hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
    if ($hash -ne $latest.sha256) {
        throw "wpk.zip: sha256 is $hash, expected $($latest.sha256)"
    }
    Expand-Archive $zip -DestinationPath $dir
    & (Join-Path $dir 'wpk.exe') init
    if ($LASTEXITCODE -ne 0) { throw 'wpk init failed' }
} finally {
    Remove-Item -Recurse -Force $dir
}
