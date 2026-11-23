## **Background**

Cloud-hosted radio automation platforms operating in AWS face significant challenges when handling professional broadcast audio, GPIO signaling, audio routing, and automation integration workflows. Traditional broadcast infrastructure relies heavily on on-premises technologies such as satellite receivers, Audio over IP (AoIP) routing systems, GPIO relay interfaces, and vendor-specific routing platforms that are not easily portable to cloud environments.

Current deployments often require multiple third-party products and protocol conversion layers to ingest satellite programming, transport GPIO events, route audio between stations, and integrate with automation systems. These solutions introduce operational complexity, increased latency, synchronization issues between audio and control signals, additional licensing costs, and architectural limitations caused by AWS networking constraints.

Customers moving radio operations to cloud-hosted automation require a platform-native audio routing solution capable of ingesting, routing, monitoring, transforming, and distributing audio and associated control signaling without relying on external broadcast routing vendors.

The Audio Routing Controller (ARC) will provide a centralized audio routing platform that supports multiple broadcast audio protocols, signal formats, GPIO integration, automation integration, and cloud-native operation while remaining deployable in AWS and traditional on-premises environments. 

---

## **Business Need**

ARC is required to provide a vendor-owned audio routing and control platform that:

- Eliminates dependence on expensive third-party audio routing systems.
- Enables cloud-hosted radio automation deployments.
- Supports both AWS-hosted and on-premises broadcast operations.
- Allows audio sources to be routed to multiple destinations.
- Preserves alignment between audio and associated signaling events.
- Supports automation system integration through WideOrbit Device Server.
- Provides a consistent configuration and monitoring experience across supported protocols.
- Reduces infrastructure complexity by consolidating audio routing, GPIO handling, automation integration, event processing, monitoring, and protocol translation into a single platform.
- Establishes a foundation for future cloud-scale deployment, resilience, and routing capabilities. 
---

## **User Stories**

### Station Owner

**As a station owner,**

I want to move my automation platform to a cloud-hosted environment,

**So that**

I can reduce operational overhead, reduce dependency on specialized hardware, and allow personnel to access systems remotely.

### Station Engineer

**As a station engineer,**

I want to ingest, route, monitor, and distribute audio and control signals across cloud and on-premises environments,

**So that**

I can maintain operational workflows without duplicating programming feeds or deploying additional routing infrastructure.

### Product Manager

**As a Product Manager,**

I want to provide a fully integrated audio routing platform,

**So that**

customers can deploy and operate radio automation systems using WideOrbit-managed solutions rather than relying on third-party routing vendors. 

---

# **Scope**

ARC shall provide a complete audio routing, signaling, GPIO, monitoring, and control platform that includes the following capabilities.

---

## Source Management

Provide configuration and operation of audio sources.

### External Sources

- UDP Audio Streams
- RTP Audio Streams
- SRT Audio Streams
- HLS Streams
- Icecast Streams (AAC, MP3, Opus)

### Internal Sources

- RTP
- SRT
- ALSA
- Windows Local Audio Devices
- AES67
- Axia Livewire
- Wheatstone
- SAS
- Lawo
- Dante

### Source Functions

- Protocol-specific source configuration
- Friendly names
- Network interface selection
- Source monitoring
- Audio preview
- Audio metering
- Audio health monitoring
- Source-specific latency configuration
- Source-specific signaling configuration
- Source-specific GPIO mapping
- Source-specific relay delay configuration
- Source failover and recovery policies
- Metadata ingestion and processing
- Source availability monitoring

---

## Destination Management

Provide configuration and operation of audio destinations.

### Supported Destination Types

- SRT
- RTP
- UDP
- HLS
- Icecast
- ALSA
- Windows Audio Devices
- Null/Test Destinations
- Future-capable AoIP destination models including AES67-class transports

### Destination Functions

- Protocol-specific destination configuration
- Destination lifecycle management
- Output processing
- Output delay
- Transport configuration
- Output monitoring
- Health monitoring
- Recovery policies
- Cue insertion policies
- Metadata policies
- Authentication configuration
- Confirmation and acknowledgment policies
- Destination restart and recovery behavior

---

## Audio Routing

Provide routing between configured sources and destinations.

### Routing Rules

- Any compatible source may route to any compatible destination.
- A source may route to many destinations.
- A destination may have zero or one active source.
- Audio mixing is not supported.
- Independent left/right source routing is not supported.

### Routing Features

- Create routes
- Replace routes
- Remove routes
- Fan-out routing
- Route validation
- Route planning
- Route persistence
- Route recovery
- Route history
- Route rollback
- Route undo
- Atomic routing operations
- Best-effort routing operations
- Route presets (salvos)
- Multi-user route synchronization

---

## Crosspoint Matrix

Provide a real-time browser-based matrix routing interface.

### Matrix Functions

- Crosspoint routing
- Immediate mode operation
- Take mode operation
- Route staging
- Batch routing
- Preset execution
- Route state visualization
- Real-time updates
- Multi-user synchronization
- Search and filtering
- Health visualization
- Compatibility filtering

---

## Signal Processing and Event Handling

Provide ingestion, normalization, translation, generation, and monitoring of signaling information associated with audio streams.

### Supported Signal Types

- SCTE-35
- ASCII-based signaling
- XML-based signaling
- Timed metadata
- Sidecar signaling
- Vendor-specific signaling formats
- Custom parser plugins

### Signal Functions

- Signal extraction
- Signal parsing
- Validation
- Normalization
- Scheduling
- Mapping
- Translation
- Regeneration
- Pass-through
- Suppression
- Auditing
- Monitoring
- Event lifecycle tracking
- Output injection

---

## GPIO Integration

Provide logical GPIO support and automation integration.

### GPIO Functions

- GPI processing
- GPO processing
- Device Server integration
- Logical GPIO banks
- GPIO mapping
- Pulse handling
- State handling
- Debounce handling
- Duplicate suppression
- Retrigger handling
- Real-time status monitoring
- GPIO visualization

---

## WideOrbit Device Server Integration

Provide native integration with WideOrbit Device Server.

### Required Capabilities

- Native Device Server plugin
- Logical GPI integration
- Logical GPO integration
- Audio crosspoint exposure
- Device Server routing control
- State reconciliation
- Capability reporting
- Route synchronization
- Health reporting
- Reconnect handling

---

## API Platform

Provide a versioned API-based control plane.

### Required Interfaces

- REST API
- OpenAPI 3.1 Specification
- Browser UI integration
- WideOrbit integration
- SDK generation
- WebSocket event streaming

### API Functional Areas

- Sources
- Destinations
- Routes
- Route Presets
- GPIO
- Signal Mappings
- Operations
- Licensing
- Audit
- Capabilities
- System Management
- Backup and Restore
- Authentication
- Secrets Management

---

## Monitoring and Observability

Provide operational visibility.

### Required Capabilities

- Real-time health monitoring
- OpenTelemetry metrics
- Structured logging
- Distributed tracing
- Audit history
- Route monitoring
- GPIO monitoring
- Signal monitoring
- Source monitoring
- Destination monitoring
- Operational dashboards

---

## Deployment Platforms

ARC shall support:

### Operating Systems

- Windows
- Linux

### Deployment Models

- On-Premises
- Docker
- AWS

### Processor Architectures

- Intel x86-64
- ARM64

---

## Authentication and Security

ARC shall provide:

### Initial Authentication Model

- Basic Authentication
- API Keys

### Future Authentication Model

The platform shall be designed to allow activation of stronger authentication and authorization mechanisms through configuration without requiring architectural changes.

Supported future authentication models include:

- OpenID Connect
- OAuth
- Keycloak-based authentication
- SSO integrations

---

## Licensing Framework

ARC shall include a licensing framework capable of supporting future licensing enforcement.

### Initial Scope

- Licensing architecture
- Entitlement model
- License validation interfaces
- Usage tracking
- Capacity tracking

### Deferred

- Customer-facing license enforcement rules
- Source limits
- Destination limits
- Feature gating

---

## Architecture Requirements

ARC shall be designed as a stateful platform with separation between:

- Durable Domain State
- Control Plane
- Media Plane
- Infrastructure Adapters

The architecture shall support future evolution toward:

- Multi-worker deployments
- Elastic cloud scaling
- High availability models
- Worker assignment and reconciliation
- Cloud-native deployment patterns

without requiring fundamental redesign of business logic.

---

## **Story Points**

TBD

---

## **Expected Outcome**

Upon completion, ARC provides a complete WideOrbit-owned audio routing platform capable of:

- Ingesting audio from industry-standard broadcast transports.
- Routing audio between cloud-hosted and on-premises environments.
- Managing GPIO and automation signaling.
- Integrating directly with WideOrbit Device Server.
- Providing browser-based routing and administration.
- Supporting automation-driven routing workflows.
- Supporting future licensing models.
- Operating across Windows, Linux, Docker, AWS, Intel, and ARM deployments.
- Eliminating the need for third-party audio routing systems in supported customer deployments.
- Providing a scalable foundation for future cloud-native broadcast operations.

---

## **Out of Scope**

The following items are not included in this Epic and may be delivered through separate Epics, Stories, or Proof of Concept efforts:

- POC-specific implementation shortcuts
- Temporary authentication approaches used solely for prototype validation
- Release-specific protocol subsets
- Final commercial licensing policies
- Production deployment topology decisions
- Future resilience tiers beyond baseline restart recovery
- Customer-specific provider protocol implementations not explicitly defined and tested
- Automation-system scheduling functionality beyond ARC control APIs

---

## **Definition of Done**

- All scoped source, destination, routing, signaling, GPIO, API, and Device Server capabilities are implemented.
- Browser-based administration and routing interfaces are available.
- Platform operates on supported Windows and Linux deployments.
- Platform supports Intel and ARM deployments where dependencies permit.
- APIs are documented and versioned.
- OpenTelemetry-compatible observability is implemented.
- Authentication framework is implemented with Basic Authentication and API Key support.
- Licensing framework is implemented.
- Desired state survives application restart.
- Routing state survives application restart.
- Automated test coverage exists for supported functionality.
- Operational documentation is available.
- Deployment documentation is available.
- Known functional gaps are documented and tracked.
- All acceptance criteria for child stories have passed QA validation.