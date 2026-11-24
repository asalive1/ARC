$ErrorActionPreference = "Stop"

$tlsDirectory = Join-Path $PSScriptRoot "tls"
New-Item -ItemType Directory -Path $tlsDirectory -Force | Out-Null
$cert = Join-Path $tlsDirectory "cert.pem"
$key = Join-Path $tlsDirectory "key.pem"

openssl req -x509 -nodes -newkey rsa:2048 `
    -keyout $key `
    -out $cert `
    -days 30 `
    -subj "/CN=ingress" `
    -addext "subjectAltName=IP:127.0.0.1,DNS:localhost,DNS:ingress"

Write-Host "Generated local TLS key and certificate in $tlsDirectory"
