param(
    [string]$NetworkName = "arc-recovery-validation",
    [string]$PostgresName = "arc-recovery-postgres",
    [string]$DatabaseName = "arc_recovery",
    [string]$Password = "arc-recovery-validation",
    [string]$ApiPort = "18081",
    [string]$OutputDirectory = "artifacts"
)

$ErrorActionPreference = "Stop"

New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$dumpPath = Join-Path $OutputDirectory "arc-postgres-recovery.dump"
$networkCreated = $false
$postgresCreated = $false
$apiCreated = $false

try {
    docker network create $NetworkName | Out-Null
    $networkCreated = $true
    docker run -d --name $PostgresName --network $NetworkName `
        -e POSTGRES_USER=arc `
        -e POSTGRES_PASSWORD=$Password `
        -e POSTGRES_DB=$DatabaseName `
        postgres:16-alpine | Out-Null
    $postgresCreated = $true

    $ready = $false
    for ($attempt = 0; $attempt -lt 40; $attempt++) {
        docker exec $PostgresName pg_isready -U arc -d $DatabaseName *> $null
        if ($LASTEXITCODE -eq 0) {
            $ready = $true
            break
        }
        Start-Sleep -Seconds 1
    }
    if (-not $ready) {
        throw "PostgreSQL did not become ready"
    }

    $databaseUrl = "postgres://arc:${Password}@${PostgresName}:5432/${DatabaseName}"
    $apiImage = "arc-recovery-validation-api"
    docker build -f deploy/Dockerfile.api -t $apiImage . | Out-Null
    docker run -d --name arc-recovery-api --network $NetworkName `
        -e DATABASE_URL=$databaseUrl `
        -e ARC_API_BIND_ADDRESS=0.0.0.0:8080 `
        -p "${ApiPort}:8080" `
        $apiImage | Out-Null
    $apiCreated = $true

    $apiReady = $false
    for ($attempt = 0; $attempt -lt 40; $attempt++) {
        try {
            $response = Invoke-RestMethod "http://127.0.0.1:${ApiPort}/api/v1/health/ready"
            if ($response.status -eq "ready") {
                $apiReady = $true
                break
            }
        } catch {}
        Start-Sleep -Seconds 1
    }
    if (-not $apiReady) {
        throw "API readiness endpoint did not become ready"
    }

    docker exec $PostgresName pg_dump -U arc -d $DatabaseName -Fc -f /tmp/arc-postgres-recovery.dump
    docker cp "${PostgresName}:/tmp/arc-postgres-recovery.dump" $dumpPath

    docker restart arc-recovery-api | Out-Null
    $restartReady = $false
    for ($attempt = 0; $attempt -lt 40; $attempt++) {
        try {
            $response = Invoke-RestMethod "http://127.0.0.1:${ApiPort}/api/v1/health/ready"
            if ($response.status -eq "ready") {
                $restartReady = $true
                break
            }
        } catch {}
        Start-Sleep -Seconds 1
    }
    if (-not $restartReady) {
        throw "API did not become ready after restart"
    }

    docker exec $PostgresName psql -U arc -d $DatabaseName -c "CREATE TABLE IF NOT EXISTS recovery_evidence (id BIGINT PRIMARY KEY, marker TEXT NOT NULL); INSERT INTO recovery_evidence(id, marker) VALUES (1, 'survives-database-dump') ON CONFLICT (id) DO UPDATE SET marker = EXCLUDED.marker;"

    docker exec $PostgresName pg_dump -U arc -d $DatabaseName -Fc -f /tmp/arc-postgres-recovery.dump
    docker cp "${PostgresName}:/tmp/arc-postgres-recovery.dump" $dumpPath

    $restoreDatabaseName = "${DatabaseName}_restored"
    docker exec $PostgresName createdb -U arc $restoreDatabaseName
    docker exec $PostgresName pg_restore -U arc -d $restoreDatabaseName --clean --if-exists /tmp/arc-postgres-recovery.dump
    $marker = docker exec $PostgresName psql -U arc -d $restoreDatabaseName -tAc `
        "SELECT marker FROM recovery_evidence WHERE id = 1"
    if ($marker.Trim() -ne "survives-database-dump") {
        throw "Restored recovery marker did not match"
    }
    Write-Host "Recovery evidence dump: $dumpPath"
    Write-Host "Clean migration, API startup, restart, dump, and restore passed"
}
finally {
    if ($apiCreated) { docker rm -f arc-recovery-api 2>$null | Out-Null }
    if ($postgresCreated) { docker rm -f $PostgresName 2>$null | Out-Null }
    if ($networkCreated) { docker network rm $NetworkName 2>$null | Out-Null }
}
