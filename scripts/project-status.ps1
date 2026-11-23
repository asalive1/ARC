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
Write-Host " Project Status"
Write-Host "================================="
Write-Host ""

Write-Host "Project:"
Write-Host "  $ProjectName"
Write-Host ""

$status = git status -sb

$remote = git remote

Write-Host "Repository State:"

if (-not $remote) {
    Write-Host "  No GitHub Remote Configured"
}
elseif ($status -match "ahead") {
    Write-Host "  Backup Needed"
}
elseif ($status -match "behind") {
    Write-Host "  Local Repository Behind GitHub"
}
elseif ($status -match "M " -or $status -match "\?\?") {
    Write-Host "  Uncommitted Changes Present"
}
else {
    Write-Host "  Synced With GitHub"
}

if ($status -match "ahead") {
    Write-Host "  Backup Needed"
}
elseif ($status -match "behind") {
    Write-Host "  Local Repository Behind GitHub"
}
elseif ($status -match "M " -or $status -match "\?\?") {
    Write-Host "  Uncommitted Changes Present"
}
else {
    Write-Host "  Synced With GitHub"
}

Write-Host ""

Write-Host "Current Branch:"
Write-Host "  $(git branch --show-current)"
Write-Host ""

Write-Host "Repository Status:"
Write-Host "$status"

Write-Host ""

Write-Host "Last Commit:"
git --no-pager log --oneline -1

Write-Host ""

Write-Host "Commits Since Latest Release:"

$latestTag = git tag --sort=-version:refname | Select-Object -First 1

if ($latestTag) {

    $commitCount = git rev-list "$latestTag..HEAD" --count

    Write-Host "  $commitCount"
}
else {
    Write-Host "  No releases created yet."
}

Write-Host ""

Write-Host "Recent History:"
git --no-pager log --oneline -5

Write-Host ""

$tags = git tag --sort=-version:refname | Select-Object -First 5


Write-Host "Latest Release:"

$latestTag = git describe --tags --abbrev=0 2>$null

if ($latestTag) {
    Write-Host "  $latestTag"
}
else {
    Write-Host "  No releases created yet."
}

Write-Host ""

Write-Host "Recent Releases:"

if ($tags) {
    $tags
}
else {
    Write-Host "  No releases created yet."
}
Write-Host ""