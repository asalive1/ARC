Clear-Host

Write-Host ""
Write-Host "================================="
Write-Host " Create Project"
Write-Host "================================="
Write-Host ""

$ProjectName = Read-Host "Project Name"

if ([string]::IsNullOrWhiteSpace($ProjectName)) {
    Write-Host ""
    Write-Host "Project creation cancelled."
    Write-Host ""
    exit
}

Write-Host ""
Write-Host "Project Name:"
Write-Host "  $ProjectName"
Write-Host ""