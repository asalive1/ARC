$ErrorActionPreference = "Stop"

$secretsDirectory = Join-Path $PSScriptRoot "..\secrets"
$files = @(
    (Join-Path $secretsDirectory "admin_bootstrap_token.txt"),
    (Join-Path $secretsDirectory "admin_recovery_token.txt")
)

foreach ($file in $files) {
    if (Test-Path $file) {
        throw "Refusing to overwrite existing secret file: $file"
    }
}

New-Item -ItemType Directory -Path $secretsDirectory -Force | Out-Null
$random = [System.Security.Cryptography.RandomNumberGenerator]::Create()
try {
    foreach ($file in $files) {
        $bytes = New-Object byte[] 32
        $random.GetBytes($bytes)
        $token = [System.BitConverter]::ToString($bytes).Replace("-", "").ToLowerInvariant()
        [System.IO.File]::WriteAllText($file, $token, [System.Text.Encoding]::ASCII)
        $identity = [System.Security.Principal.WindowsIdentity]::GetCurrent()
        $acl = New-Object System.Security.AccessControl.FileSecurity
        $acl.SetAccessRuleProtection($true, $false)
        $acl.SetOwner($identity.User)
        $rule = [System.Security.AccessControl.FileSystemAccessRule]::new(
            $identity.Name,
            [System.Security.AccessControl.FileSystemRights]::FullControl,
            [System.Security.AccessControl.AccessControlType]::Allow
        )
        $acl.AddAccessRule($rule)
        Set-Acl -Path $file -AclObject $acl
    }
}
finally {
    $random.Dispose()
}

Write-Output "Created first-run administrator secret files in $secretsDirectory"
