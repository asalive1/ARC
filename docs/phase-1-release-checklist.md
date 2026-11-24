# Phase 1 Release Checklist

This checklist is a gate, not a claim that the gate passed.

## Automated gates

- [ ] `cargo test --locked -p arc-domain -p arc-infrastructure -p arc-worker -p arc-api`
- [ ] API PostgreSQL and live loopback ignored tests pass serially
- [ ] UI `npm ci`, `npm test -- --run`, and `npm run build` pass
- [ ] `docker compose config --quiet` passes
- [ ] API, UI and worker Docker images build
- [ ] PostgreSQL dump/restore script passes
- [ ] Rust dependency inventory is generated
- [ ] Nginx WSS configuration passes `nginx -t`

## External acceptance gates

- [ ] Live ALSA hardware produces RTP L16 with no underrun or device errors
- [ ] RTP→SRT and ALSA→RTP evidence is reviewed
- [ ] Worker WSS certificate is issued for and trusted by the worker hostname
- [ ] Certificate/key rotation and revoke procedures are tested
- [ ] Production secret-store adapter is approved and tested
- [ ] Linux AMD64, Linux ARM64 and Windows x64 build/media smoke gates pass
- [ ] GStreamer/plugin/runtime package closure is reviewed
- [ ] GPL/AGPL components are excluded by the approved package policy
- [ ] SBOM, notices and license allowlist are approved
- [ ] Real CPU, memory, network and stream-calibration evidence is recorded

## Repository release gates

- [ ] Traceability matrix links implementation and test evidence
- [ ] Phase 1 acceptance report is reviewed
- [ ] CHANGELOG and deployment documentation are current
- [ ] Approval exists for the repository remote (`origin` currently resolves to `https://github.com/asalive1/ARC.git`)
- [ ] Protected branch and release branch policy are approved
- [ ] Phase 1 commit, CI success, annotated tag and push are authorized
