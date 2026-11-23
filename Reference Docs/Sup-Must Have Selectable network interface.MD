ARC Network Interface and Container Networking Requirements
Design Principle

ARC is intended for professional broadcast, streaming, automation, and AoIP environments where hosts frequently contain multiple network interfaces, bonded interfaces, VLAN interfaces, redundant networks, and physically separate media networks.

Network interfaces shall be treated as first-class configurable resources within ARC.

ARC shall never rely upon operating system interface selection or container-network defaults for media transport.

Every network-based source and destination must explicitly identify the host network interface that will be used for receiving or transmitting media.

Failure to explicitly select interfaces has previously caused deployment and operational issues in similar systems and is therefore considered an architectural defect.

Network Interface Management
Host Interface Discovery

ARC Worker shall discover and maintain a real-time inventory of available host network interfaces.

Discovered interfaces shall include:

Interface Name
Operational State
Administrative State
IPv4 Addresses
IPv6 Addresses
MAC Address
MTU
Multicast Capability
Link Speed (when available)

Example:

JSON
{
"name": "eth0",
"ipv4": "10.1.1.100",
"up": true,
"multicast": true,
"mtu": 9000
}

Network interface discovery shall be provided through the API and exposed in the administration UI.

Network Interface API

Implement:

Plain Text
GET /api/v1/network/interfaces

Returns:

JSON
[
{
"name": "eth0",
"ipv4": "10.10.1.100",
"up": true,
"multicast": true
},
{
"name": "eth1",
"ipv4": "172.16.10.100",
"up": true,
"multicast": true
}
]

This endpoint shall be used by the UI to populate interface selection controls.

Network Interface Selection
Mandatory User Selection

Every network-based source and destination must expose a network interface selection field.

A dropdown list shall be presented in the UI.

Users shall be able to explicitly bind each source and each destination to a specific interface.

Examples:

RTP Input
JSON
{
"interface": "eth0",
"group": "239.1.1.1",
"port": 5000
}
RTP Output
JSON
{
"interface": "eth1",
"destination": "239.1.2.1",
"port": 5000
}
SRT Input
JSON
{
"interface": "bond0",
"mode": "listener",
"port": 9000
}
SRT Output
JSON
{
"interface": "bond1",
"mode": "caller",
"remote": "10.15.1.20:9000"
}
Supported Sources Requiring Interface Selection

At minimum:

RTP
SRT
UDP MPEG-TS
HLS
Icecast
Future AoIP Protocols
Future Network Sources
Supported Destinations Requiring Interface Selection

At minimum:

RTP
SRT
UDP MPEG-TS
HLS
Icecast
Future AoIP Protocols
Future Network Destinations
Persistence Requirement

Selected interfaces shall be persisted.

Configuration records must include interface identifiers.

Example:

JSON
{
"name": "Studio RTP Input",
"interface": "eth0",
"address": "239.100.1.1",
"port": 5000
}

ARC shall restore network bindings following:

Worker restart
Service restart
Host reboot
Container recreation

without requiring user intervention.

Media Plane Networking Requirements
Host Network Awareness

The Media Plane shall operate against host networking resources.

The Media Plane must be capable of:

Binding to specific interfaces
Joining multicast groups on specific interfaces
Listening for external RTP traffic
Listening for external SRT traffic
Sending RTP traffic to external networks
Sending SRT traffic to external networks
Accessing interface statistics
Receiving interface state changes

The Media Plane shall not assume all media remains inside a Docker network.

Audio Routing Scope

Audio entering ARC is expected to originate from:

External RTP encoders
Broadcast devices
Audio processors
STL systems
SRT gateways
Network audio equipment
Future AoIP infrastructure

Audio leaving ARC is expected to terminate at:

External RTP destinations
SRT gateways
Broadcast processors
Studio equipment
Monitoring systems
Automation systems
Future AoIP infrastructure

ARC shall be designed with the assumption that media routinely crosses both host and container boundaries.

Linux Container Networking Requirements
Hard Requirement

For Linux deployments, ARC Worker containers must have direct access to host networking.

This is a mandatory architectural requirement.

Direct access is required to support:

RTP
SRT
Multicast
UDP MPEG-TS
Future AoIP protocols
Low latency transport
Broadcast deployment scenarios
Container Networking Mode

Preferred deployment:

YAML
network_mode: host

for all Media Plane worker containers.

ARC documentation shall treat host networking as the standard Linux deployment model.

Host Interface Visibility

The ARC Worker must have visibility into:

Host interfaces
Interface status changes
Link state changes
Network configuration changes

The Worker must not rely solely upon virtual Docker network adapters.

Windows Networking Requirements

Windows deployments shall expose the same interface-selection capabilities.

Windows Audio destinations and network destinations shall support explicit interface binding whenever the underlying operating system supports interface selection.

The platform abstraction shall prevent Linux and Windows networking behavior from diverging at the API level.

Worker Architecture Requirements

The ARC Worker owns all media networking responsibilities.

Responsibilities include:

Interface discovery
Interface monitoring
Socket binding
RTP reception
RTP transmission
SRT reception
SRT transmission
Multicast management
Network health monitoring

The API layer shall never directly manage media sockets.

The UI layer shall never directly manage media sockets.

User Interface Requirements
Source Configuration Screens

Each network source configuration page shall provide:

Interface Selector
Address
Port
Protocol Options
Validation
Connection Status
Destination Configuration Screens

Each destination configuration page shall provide:

Interface Selector
Destination Address
Port
Protocol Options
Validation
Connection Status
Interface Health Display

The administration UI shall display:

Interface Name
IP Address
Link Status
Health State
Active Routes Using Interface
Bandwidth Statistics (future phase)
Future AoIP Compatibility

The networking architecture shall support future implementation of:

AES67
SMPTE ST 2110-30
ST 2022-7
Dante Adapter Integrations
Livewire Integrations
WheatNet Integrations
Other IP Audio Transport Protocols

without requiring redesign of source, destination, routing, or interface-selection models.

Additional Architecture Directive

Add the following directive to the Architecture Requirements section:

Network interfaces are first-class managed resources within ARC. Every network-based source and destination shall be explicitly associated with a host network interface. The Media Plane shall operate using host networking resources rather than relying on container-network abstractions. Linux deployments must support direct host-interface access from ARC Worker containers. Audio ingress and egress are assumed to originate from and terminate on systems external to both the container and the host machine.

This addition closes a significant architectural gap in the existing prompt and ensures future implementations cannot accidentally reintroduce container-isolated media assumptions.