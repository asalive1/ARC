. "$PSScriptRoot\project-settings.ps1"

Clear-Host

Write-Host ""
Write-Host "================================="
Write-Host " Save Checkpoint"
Write-Host "================================="
Write-Host ""

git rev-parse --git-dir *> $null

if ($LASTEXITCODE -ne 0) {
    Write-Host "This folder is not a Git repository."
    Write-Host ""
    Write-Host "Initialize Git before using this task."
    Write-Host ""
    exit
}

$changes = git status --porcelain

if (-not $changes) {
    Write-Host "No changes detected."
    Write-Host ""
    Write-Host "Checkpoint not required."
    Write-Host ""
    exit
}

$message = Read-Host "Checkpoint Description"

if ([string]::IsNullOrWhiteSpace($message)) {
    Write-Host ""
    Write-Host "Commit cancelled."
    Write-Host ""
    exit
}

git add .

git commit -m "$message"

if ($LASTEXITCODE -eq 0) {
    Write-Host ""
    Write-Host "================================="
    Write-Host " Checkpoint Saved"
    Write-Host "================================="
    Write-Host ""

    git log --oneline -1
}
else {
    Write-Host ""
    Write-Host "Checkpoint failed."
    Write-Host ""
}