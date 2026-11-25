$ErrorActionPreference = "Stop"

$secretsDirectory = Join-Path $PSScriptRoot "..\secrets"
$credentialsFile = Join-Path $secretsDirectory "worker_credentials.json"
$tokenFile = Join-Path $secretsDirectory "worker_control_token.txt"

foreach ($file in @($credentialsFile, $tokenFile)) {
    if (Test-Path $file) {
        throw "Refusing to overwrite existing secret file: $file"
    }
}

New-Item -ItemType Directory -Path $secretsDirectory -Force | Out-Null
$workerId = [Guid]::NewGuid().ToString().ToLowerInvariant()
$random = [System.Security.Cryptography.RandomNumberGenerator]::Create()
try {
    $bytes = New-Object byte[] 32
    $random.GetBytes($bytes)
    $token = [System.BitConverter]::ToString($bytes).Replace("-", "").ToLowerInvariant()
    $credentials = @{ $workerId = $token } | ConvertTo-Json -Compress
    [System.IO.File]::WriteAllText($credentialsFile, $credentials, [System.Text.Encoding]::ASCII)
    [System.IO.File]::WriteAllText($tokenFile, $token, [System.Text.Encoding]::ASCII)

    $identity = [System.Security.Principal.WindowsIdentity]::GetCurrent()
    foreach ($file in @($credentialsFile, $tokenFile)) {
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

Write-Output "Worker ID: $workerId"
Write-Output "Worker credential files created in $secretsDirectory"
