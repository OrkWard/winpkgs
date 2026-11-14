# Build wpk and package it for release.
#
#   elvish scripts/release.elv             build and package into target/dist/<version>
#   elvish scripts/release.elv --publish   also upload to MinIO and write the
#                                          wpk-bucket manifest (not committed)
#
# MinIO (alias prod, bucket scoop-bucket, public at https://s3.orkward.dev):
#   wpk/<version>/wpk.zip   wpk.exe, wpk-shim.exe, wpk-shimw.exe
#   wpk/latest.json         {version, url, sha256}, read by the install scripts
#   wpk/install.elv, wpk/install.ps1
use os
use path
use str

var publish = (has-value $args --publish)
var root = (path:dir (path:dir (src)[name]))
cd $root

if (and $publish (not-eq (git status --porcelain | slurp) '')) {
  fail 'working tree is dirty; commit first'
}

# `cargo pkgid` prints `path+file:///...#<version>`, or `...#wpk@<version>`.
var version = (cargo pkgid -p wpk | str:split '#' (one) | put [(all)][-1] | str:split '@' (one) | put [(all)][-1])
var public = https://s3.orkward.dev/scoop-bucket/wpk
var target = prod/scoop-bucket/wpk
echo 'version: '$version

selever exec -- cargo build --release --locked -p wpk -p wpk-shim

var out = $root'\target\dist\'$version
os:remove-all $out
os:mkdir-all $out
var zip = $out'\wpk.zip'
var bins = [wpk wpk-shim wpk-shimw]
7z a -tzip -bso0 -bd $zip (each {|b| put $root'\target\release\'$b'.exe' } $bins)

var hash = (certutil -hashfile $zip SHA256 | drop 1 | take 1)
set hash = (str:to-lower (str:replace ' ' '' $hash))
var latest = [&version=$version &url=$public/$version/wpk.zip &sha256=$hash]
put $latest | to-json > $out'\latest.json'
echo (put $latest | to-json)

if (not $publish) {
  echo 'packaged in '$out'; pass --publish to upload'
} else {
  # The zip goes up before latest.json so installers never see a dangling url.
  mc cp --quiet $zip $target/$version/wpk.zip
  mc cp --quiet $root'\scripts\install.elv' $target/install.elv
  mc cp --quiet $root'\scripts\install.ps1' $target/install.ps1
  mc cp --quiet $out'\latest.json' $target/latest.json

  var remote = (curl -fsSL $public/latest.json | from-json)
  if (not-eq $remote[sha256] $hash) {
    fail 'published latest.json does not match: '(put $remote | to-json)
  }

  var manifest = [
    &version=$version
    &description='Portable package manager for Windows, reading Scoop buckets'
    &homepage=https://github.com/OrkWard/winpkgs
    &license=BSD-3-Clause
    &url=$public/$version/wpk.zip
    &hash=$hash
    &bin=wpk.exe
  ]
  var bucket = ~/Repos/wpk-bucket/bucket/wpk.json
  # Indented like Scoop's manifests; jq keeps the key order.
  put $manifest | to-json | jq --indent 4 '{version, description, homepage, license, url, hash, bin}' > $bucket
  echo 'published; commit and push '$bucket
}
