Clear-Host

Write-Host ""
Write-Host "================================="
Write-Host " Project Initialization"
Write-Host "================================="
Write-Host ""

$ProjectName = Read-Host "Project Name"

if ([string]::IsNullOrWhiteSpace($ProjectName)) {
    Write-Host ""
    Write-Host "Initialization cancelled."
    Write-Host ""
    exit
}

#
# Update project-settings.ps1
#

$settingsFile = Join-Path $PSScriptRoot "project-settings.ps1"

$settingsContent = Get-Content $settingsFile -Raw

$settingsContent = $settingsContent.Replace(
    '$ProjectName = "CHANGE_ME"',
    "`$ProjectName = `"$ProjectName`""
)

Set-Content -Path $settingsFile -Value $settingsContent

#
# Update README.md
#

$readmeFile = Join-Path (Split-Path $PSScriptRoot -Parent) "README.md"

if (Test-Path $readmeFile) {

    $readmeContent = Get-Content $readmeFile -Raw

    $readmeContent = $readmeContent.Replace(
        'PROJECT_NAME',
        $ProjectName
    )

    Set-Content -Path $readmeFile -Value $readmeContent
}

#
# Initialize Git Repository
#

git rev-parse --git-dir *> $null

if ($LASTEXITCODE -ne 0) {

    git init

    git branch -M main
}


Write-Host ""
Write-Host "Project initialized:"
Write-Host "  $ProjectName"
Write-Host ""

Write-Host "Git repository initialized."
Write-Host ""
Write-Host "If this project is stored on the NAS, run:"
Write-Host ""
Write-Host "git config --global --add safe.directory `"$((Get-Location).Path)`""
Write-Host ""