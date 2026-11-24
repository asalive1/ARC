param(
    [string]$OutputDirectory = "artifacts"
)

$ErrorActionPreference = "Stop"

New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$jsonPath = Join-Path $OutputDirectory "rust-dependency-inventory.json"
$textPath = Join-Path $OutputDirectory "rust-dependency-inventory.txt"

$workspace = (Resolve-Path ".").Path
if ($workspace.StartsWith("\\\\")) {
    throw "Run this script from a local mapped or staging workspace path; UNC Docker bind paths are unsupported"
}
$metadataJson = docker run --rm `
    -v "${workspace}:/workspace" `
    -w /workspace `
    rust:1-slim `
    cargo metadata --locked --format-version 1 --all-features

$metadata = $metadataJson | ConvertFrom-Json
$packages = $metadata.packages |
    Sort-Object name, version |
    ForEach-Object {
        [PSCustomObject]@{
            name = $_.name
            version = $_.version
            license = if ([string]::IsNullOrWhiteSpace($_.license)) { "UNKNOWN" } else { $_.license }
            source = if ([string]::IsNullOrWhiteSpace($_.source)) { "workspace" } else { $_.source }
        }
    }

$packages | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $jsonPath -Encoding UTF8
$packages |
    ForEach-Object { "{0}`t{1}`t{2}`t{3}" -f $_.name, $_.version, $_.license, $_.source } |
    Set-Content -LiteralPath $textPath -Encoding UTF8

Write-Host "Rust dependency inventory: $jsonPath"
Write-Host "Rust dependency table: $textPath"
