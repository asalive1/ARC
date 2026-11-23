# Local Compose Deployment

This Compose stack is a Phase 0 development deployment of the runnable
scaffold: PostgreSQL, the API, and the static UI. The API applies SQLx
migrations before opening its HTTP listener. PostgreSQL data persists in the
`postgres_data` named volume.

## Start

Copy `.env.example` to `.env`, replace the local database password with a
URL-safe value, then run:

```powershell
Copy-Item .env.example .env
docker compose up --build --wait
```

Open the UI at `http://localhost:3000`; the API is also available at
`http://localhost:8080/api/v1/health/live`. Both host ports can be changed with
`ARC_UI_PORT` and `ARC_API_PORT` in `.env`. Published ports bind only to
loopback. The database has no published host port.

`docker compose down` stops and removes the containers but preserves database
data. `docker compose down --volumes` also permanently removes the local
database volume.

## Scope and limitations

- The API, UI, and PostgreSQL are the only runnable Phase 0 services. A worker
  executable and OpenTelemetry export are not implemented; Compose does not
  claim to provide either.
- The API currently exposes scaffold health/capability routes. It does not yet
  implement persistence-backed routing or media operations.
- Compose builds for the host's Docker platform. The Dockerfiles use
  multi-platform base images and avoid platform-specific media packages, but
  Linux AMD64/ARM64 release builds and Windows x86-64 builds have not yet been
  validated.
- This is a local development stack, not a production deployment. API
  authentication, TLS termination, worker networking/device access, backups,
  and production secret management remain out of scope.
- Keep `.env` local; it is ignored by Git. The example password is a
  placeholder, not a credential.
