# Install wpk: fetch the latest release, check its hash and run `wpk init`.
#
#   curl -fsSL https://s3.orkward.dev/scoop-bucket/wpk/install.elv -o install.elv
#   elvish install.elv
use os
use str

var base = https://s3.orkward.dev/scoop-bucket/wpk
var latest = (curl -fsSL $base/latest.json | from-json)
var dir = (os:temp-dir 'wpk-install-*')
try {
  var zip = $dir'\wpk.zip'
  curl -fsSL -o $zip $latest[url]
  # certutil prints the hash on its second line.
  var hash = (certutil -hashfile $zip SHA256 | drop 1 | take 1)
  set hash = (str:to-lower (str:replace ' ' '' $hash))
  if (not-eq $hash $latest[sha256]) {
    fail 'wpk.zip: sha256 is '$hash', expected '$latest[sha256]
  }
  tar -xf $zip -C $dir
  (external $dir'\wpk.exe') init
} finally {
  os:remove-all $dir
}
