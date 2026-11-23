. "$PSScriptRoot\project-settings.ps1"

Clear-Host

git rev-parse --git-dir *> $null

if ($LASTEXITCODE -ne 0) {
    Write-Host ""
    Write-Host "This folder is not a Git repository."
    Write-Host ""
    Write-Host "Initialize Git before using this task."
    Write-Host ""
    exit
}

Write-Host ""
Write-Host "================================="
Write-Host " GitHub Backup"
Write-Host "================================="
Write-Host ""

$status = git status -sb

if ($status -notmatch "ahead") {
    Write-Host "Repository already synced."
    Write-Host ""
    exit
}

Write-Host "Backing up branch:"
Write-Host "  $GitBranch"
Write-Host ""

git push origin $GitBranch

if ($LASTEXITCODE -ne 0) {
    Write-Host ""
    Write-Host "Backup failed."
    Write-Host ""
    exit
}

git push --tags

Write-Host ""
Write-Host "================================="
Write-Host " Backup Complete"
Write-Host "================================="
Write-Host ""