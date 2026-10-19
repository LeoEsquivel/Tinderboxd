# Genera en dist/ los paquetes de Windows:
#   Tinderboxd_<versión>_x64-setup.exe     instalador (por usuario, sin administrador)
#   Tinderboxd_<versión>_x64-portable.zip  versión portable (descomprimir y usar)
#
#   powershell -ExecutionPolicy Bypass -File scripts/empaquetar-windows.ps1
#
# Requisitos: Rust y `cargo install cargo-packager --locked` (descarga NSIS solo).

$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..")
Push-Location $root
try {
    $version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
    $dist = Join-Path $root "dist"
    New-Item -ItemType Directory -Force $dist | Out-Null

    # 1. Instalador (cargo-packager compila en release antes de empaquetar).
    cargo packager --release
    if ($LASTEXITCODE -ne 0) { throw "cargo packager falló" }
    $setup = Join-Path $dist "Tinderboxd_${version}_x64-setup.exe"
    Move-Item -Force (Join-Path $dist "tinderboxd-gui_${version}_x64-setup.exe") $setup

    # 2. Portable: los dos .exe + portable.txt (datos en datos\ junto a ellos).
    $staging = Join-Path $dist "Tinderboxd-portable"
    if (Test-Path $staging) { Remove-Item -Recurse -Force $staging }
    New-Item -ItemType Directory $staging | Out-Null
    Copy-Item target\release\tinderboxd-gui.exe, target\release\tinderboxd.exe $staging
    # Los textos viven en scripts/portable/ (UTF-8): Windows PowerShell 5 leería
    # mal los acentos si estuvieran escritos dentro de este script.
    $templates = Join-Path $PSScriptRoot "portable"
    $utf8 = New-Object System.Text.UTF8Encoding $true   # con BOM: el Bloc de notas lo detecta
    foreach ($name in @("portable.txt", "LEEME.txt")) {
        $text = [System.IO.File]::ReadAllText((Join-Path $templates $name), [System.Text.Encoding]::UTF8)
        $text = $text.Replace("{{VERSION}}", $version).Replace("`r`n", "`n").Replace("`n", "`r`n")
        [System.IO.File]::WriteAllText((Join-Path $staging $name), $text, $utf8)
    }

    $zip = Join-Path $dist "Tinderboxd_${version}_x64-portable.zip"
    if (Test-Path $zip) { Remove-Item -Force $zip }
    Compress-Archive -Path $staging -DestinationPath $zip
    Remove-Item -Recurse -Force $staging

    ""
    "Listo:"
    Get-Item $setup, $zip | ForEach-Object { "  {0}  ({1:N1} MB)" -f $_.Name, ($_.Length / 1MB) }
}
finally {
    Pop-Location
}
