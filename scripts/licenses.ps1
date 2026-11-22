# Writes THIRD-PARTY-LICENSES.html: the licence texts of every crate compiled into Spindle.
# Needs cargo-about (`cargo install cargo-about --locked --features cli`).
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    cargo about generate --workspace --fail -o THIRD-PARTY-LICENSES.html about.hbs
    if ($LASTEXITCODE -ne 0) { throw "cargo about failed ($LASTEXITCODE)" }
} finally { Pop-Location }
