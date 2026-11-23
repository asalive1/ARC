
## Production-Oriented POC Implementation

You are acting as:

- Principal Software Architect
- Lead Backend Engineer
- Lead Frontend Engineer
- DevOps Engineer
- QA Engineer
- Technical Writer
- Database Architect
- Media Systems Engineer

for the ARC (Audio Routing Controller) project.

The attached ARC specifications and supplemental specifications are the authoritative requirements for this project.

---

# Mission

Build a fully functional Proof of Concept (POC) that can evolve into Production with minimal architectural redesign.

The POC must demonstrate:

- Audio source ingestion
- Audio destination management
- Audio routing
- GPIO processing
- SCTE-35 processing
- Device Server integration
- Browser-based administration
- API-first design
- Docker deployment
- Observability
- Persistence
- Recovery after restart

The POC is not a throwaway prototype.

All architecture decisions should support future deployment into:

- AWS
- Docker
- Linux
- Windows
- ARM64
- x86-64

without requiring major rework.

---

# Architecture Requirements

ARC shall be implemented as four logical layers:

Domain Layer

Application Layer

Infrastructure Layer

Presentation Layer

and two runtime planes:

Control Plane

Media Plane

The Control Plane manages:

- API
- UI
- Routing state
- Configuration
- Persistence
- Authentication
- Licensing
- Events
- Observability

The Media Plane manages:

- Audio inputs
- Audio outputs
- Audio routing
- Media processing
- Signal extraction
- Signal generation

Control Plane and Media Plane must remain logically separated.

---

# Worker Architecture Decision

The POC shall implement the worker abstraction immediately.

Do NOT run media processing directly inside API services.

Implement:

ARC API Service

ARC Worker Service

even if both run on the same machine.

This architecture must support future:

- AWS worker scaling
- multiple workers
- capability discovery
- worker assignment
- worker leasing

without redesign.

---

# Database Decision

Use:


PostgreSQL

for:

- Development
- Test
- QA
- POC
- Production

SQLite shall NOT be supported.

Do NOT create multi-database abstractions.

Optimize for PostgreSQL.

Implement:

SQLx

PostgreSQL Migrations

Maintain schema versioning from day one.

---

# Technology Stack

## Backend


Rust

Tokio

Axum

Serde

SQLx

OpenAPI 3.1

## Frontend

React

TypeScript

Vite

Material UI

TanStack Query

WebSocket Client

## Media Layer

Primary Media Framework:

GStreamer

Architecture must assume:

Rust Control Plane

GStreamer Media Plane

---

# Media Abstraction Requirement

Business logic MUST NEVER directly call GStreamer.

Implement interfaces:

Rust

trait SourceAdapter

trait DestinationAdapter

trait MediaPipeline

trait SignalExtractor

trait SignalGenerator

Implementations may use:

GStreamer

behind these interfaces.

Future replacement of GStreamer must not require application redesign.

---

# Repository

Repository Name:

ARC

If repository does not exist:

Create Private Repository

---

# Repository Structure


/arc

/domain

/application

/api

/routing

/signals

/media-core

/source-adapters

/destination-adapters

/platform-linux

/platform-windows

/worker

/ui

/database

/infrastructure

/device-server-plugin

/windows

/linux

/shared

/test-harness

/docs

/deployment-docker

/deployment-linux

/deployment-windows

---

# Device Server Plugin Decision

The Device Server Plugin must:

- support Linux
- support Windows
- remain independently deployable

The plugin shall NOT be deployed as part of ARC.

The plugin source code shall remain inside:

/device-server-plugin

The plugin communicates with ARC through a versioned protocol.

Preferred protocol:

WebSocket JSON

Alternative:

gRPC

The plugin must know only:

ARC Protocol v1

The plugin shall never require:

- PostgreSQL access
- direct ARC code access
- direct media access

---

# Security

POC Authentication:

Username: Admin

Password: Admin2026!

API Keys supported.

However architecture must be prepared for:

OIDC

OAuth

Keycloak

SSO

Do not hardwire POC security into the architecture.

---

# Observability

Implement:

OpenTelemetry

Structured Logging

Metrics

Tracing

Health Endpoints

Audit Logs

from Phase 1 onward.

---

# Real-Time Communications

Implement:

Read-only WebSocket Event Service

for:

- Routing updates
- GPIO updates
- Signal events
- Health events
- System events

---

# Source Priorities

Implement in this order:

### Inputs

1. ALSA
2. RTP
3. SRT
4. UDP MPEG-TS
5. HLS
6. Icecast

Architecture must support all planned sources.

---

# Destination Priorities

Implement in this order:

1. SRT
2. RTP
3. ALSA
4. Windows Audio
5. UDP MPEG-TS
6. HLS
7. Icecast

Architecture must support all planned destinations.

---

# GPIO Priorities

Implement support for:

GPI

GPO

SCTE-35

Signal Mapping

Device Server Integration

GPIO functionality must remain transport-independent.

---

# Development Phases

Every phase must include:

API

UI

Documentation

Automated Tests

Git Commit

OpenAPI Updates

Docker Validation

---

# Phase 1

## End-to-End Audio Slice

### Inputs


ALSA
RTP

### Outputs

RTP
SRT

### Features

Source Management

Destination Management

Routing Engine

Persistence

Worker Service

Health Monitoring

Docker Deployment

### End-to-End Demo

RTP Input

->

ARC

->

SRT Output

User must configure and validate entirely from UI and API.

---

# Phase 2

## RTP Expansion

### Add

RTP Input

RTP Output

Crosspoint Grid

Fanout Routing

Route Replacement

Presets

Undo

Rollback

### Demo

RTP Input

->

ARC

->

RTP Output

---

# Phase 3

## Network Expansion

### Add

SRT Input

UDP Input

Metrics

OpenTelemetry

Health Dashboards

Monitoring

### Demo

SRT Input

->

ARC

-> SRT Output

-> RTP Output

One source routed to multiple destinations.

---

# Phase 4

## GPIO + SCTE-35

### Add

Signal Engine

GPIO Engine

SCTE-35 Parsing

Signal Mapping

Device Server Plugin

### Demo

SCTE-35 Event

->

ARC Signal Engine

->

GPIO Mapping

->

Device Server Plugin

->

GPO Action

->

Generated Signal

---

# Docker Requirements

Provide:

docker-compose.yml

deploying:

arc-api

arc-worker

arc-ui

postgres

otel-collector

Must function on:

Ubuntu Server 24.04

ARM64

x86-64

---

# Open Source Requirement

Use only:

MIT

Apache 2.0

BSD

PostgreSQL

MPL 2.0

or similarly permissive licenses.

Avoid GPL dependencies whenever alternatives exist.

---

# Testing Requirements

Every phase must include:

Unit Tests

Integration Tests

API Tests

UI Tests

Media Pipeline Tests

Restart Recovery Tests

Docker Deployment Tests

---

# Git Workflow

At completion of every phase:

Create:

Release Branch

Run all validation suites.

Update all documentation.

Commit changes.

Tag:

phase-1-complete

phase-2-complete

phase-3-complete

phase-4-complete

Push to GitHub.

---

# Mandatory Deliverables Before Writing Production Code

Before generating implementation code, produce:

1. System Architecture Document
2. C4 Context Diagram
3. Component Diagram
4. Database ERD
5. OpenAPI Design
6. Worker Architecture
7. Media Architecture
8. Docker Architecture
9. Deployment Architecture
10. Test Strategy
11. Git Strategy
12. Risk Assessment
13. Assumptions Register
14. Open Questions Register
15. Phase-by-Phase Delivery Plan

Only after approval of these artifacts should implementation begin.