## Background

Application intended to allow a user to configured Audio and GPI/O inputs from a wide variety of input sources.  These sources include, but not limited to: UDP Streams, AAC or MP3 Icecast Streams, HLS Streams, RTP, AES67, Axia Livewire, Wheatstone, SAS, Lawo, SRT.  Store these sources with friendly names.  Provide routing of these input sources to an equally vast number of output destinations within the automation system.  A single Source should have the ability to route to many Destinations.  A user will have a visual Crosspoint Map that will allow for setting the audio routes.  An API will be available so that a connection with a third party service or automation system will allow the user to fully configured (add sources, add destinations, set or remove audio routes) within the automation software. 

---

## Business Need

Problem This application is looking to solve:

Working with an automation system that is transitioning to a SaaS product operating in AWS has presented limitations with satellite driven programming.
- Issue 1: Current satellite providers do not currently provide audio streams that can be consumed by a customer cloud based system for automation. The current model required the satellite programming be delivered to a satellite receiver located in the customer facility.  The user must use an expensive third party software application to encode the satellite audio and GPIO clousers to an RDP audio stream, and deliver to matching software in AWS to decode this stream.  The AWS end of the software only has the ability to output an RTP/AES67 (Livewire).  The network fabric in AWS does not handle multicast network data in a way that is affordable to the consumer. This requires the Recieving software to hand the audio off to a different service that can convert the audio to an SRT audio stream that can be sent across the AWS network to the automation software. 

>Goal Resolution 1: Provide a software application that will A) Accept a UDP audio Stream that has been embedded with GPIO Relays, and route to a destination via SRT audio stream so that we can eliminate the additional hop to the transport service in AWS. B) provide a solution we own that will not require a customer to purchase an expensive license from a third party vendor. C) Based on conversations with the satellite providers their ability to deliver their content direct to the cloud automation is about a year away.  Setting up our new service to accept a variety of audio streams that are embedded with the GPI/O relays will future proof the solution.

- Issue 2: The current transport methods across the AWS network introduces a delay in audio.  This delay in audio has shifted the GPI relays by 2 seconds so that the audio and GPI no longer line up.

>Goal Resolution: the user will have a per source or per destination configuration option that will allow the user to configure a delay in closure being sent so that the user can ensure the audio an the closure stay in alignment. 

- Issue 3: Our current product offerings do not provide a user to route audio without without expensive third party solutions such as Telos Axia, Wheatstone, or SAS.  For an automation system operating on prem, these third party vendors are the best solution.  Moving to AWS this third party options are not available.  It is common for a single satellite source to be routed to multiple radio stations within a facility.  It is common for audio from one radio station to be routed to another radio station within the facility. There are not currently solutions that can handle this for a cloud based system.

>Goal Resolution: Provide an application to the customer what will allow a single satellite source to be routed to multiple destinations.  Provide the customer with the ability to set the output of one radio station as a Source to the audio router.  Route that audio to other destinations within their network. 

- Issue 4: Some customers still require the use of specialized remote codec when hosting guests, or broadcasting from a remote location.  Many of these codec provide the ability to output to a internet or network based location.  Often this requires specialized equipment or software at the studio location to accept the feed. 

>Goal Resolution: Provide the customer with an application hosted in AWS to directly accept these feeds as sources, so that they can route them the radio station hosted in AWS.

- Issue 5: AWS does not natively offer the ability to host an Audio over IP network.  Their network structure prevents the use of Multicast networking.  Options to add this multicast layer either though AWS services or third party services such as Switch IO are extremely expensive.  Moving a radio broadcast operation to a fully cloud hosted solution will be negatively impacted by this audio routing limitation.  

>Goal Resolution: Develop and deliver an Audio Routing solution to where a variety of audio sources can be collected from either within AWS or externally and route the audio using a network protocal such as SRT that can be transmitted across the AWS network fabric. 

---

## User Story

As a station owner i want to move move my automation system to a Cloud Hosted platform
So that, i can reduce my operational and facility footprint an provide my talent a centrally located platform that can be accessed from the internet. 

As a station engineer i want to ensure that if moving to an AWS hosted platform i will have the ability to provide my stations audio sources from prgramming
So that i can route audio sources between different radio stations without needing to duplicate audio feeds.

As A product manager i want to provide a audio routing resource to route audio across the AWS network fabric 
So that I can
- Offer our customers an affordable solution for audio routing in AWS
- Our customers will pay for our services and not have to allocate money to other third party vendors
- Maintain cost savings in network and compute resources in AWS

---

## Scope

Develop an application for audio routing both on prem and in AWS.

- Audio Inputs (Sources) from external sources via UDP, RTP, SRT, HLS, Icecast
- GPI (Sources) from External sources that are embedded as part of the audio stream and configured through Device Server
- Audio Inputs (Sources) from Internal (AWS or Local) via Livewire, Wheatstone, AES 67, Dante, SRT, RTP, ALSA, Local Audio Devices.
- Audio Output (Destinations) direct to internal (AWS or Local) destinations using SRT, RTP, ALSA, and Local Outputs. 
- Audio Output (Destination) direct to external locations via SRT, HLS, RTP, Icecast
- GPO (Destinations) configured through Device Server
- Browser based UI for configuration of sources and destinations
- API for connection and integration with Automation
- API to allow full configuration as part of integration with Automation System
- API to allow the setting or removing audio routing. 
- Audio Routing so that any source can be routed to one or more destinations
- GPI/O integration with Device Server for the automation of a radio station
- Open Telementry Health reporting of the service



---
## Non Functional Requirements

- ARM Compatible
- Intel Compatible
- Docker Container on Ubuntu Server 24.04
- AWS Deployment 
- Windows Deployment (on Prem)
- Windows Deployment must include adding Service to Windows
- Windows and LinuxDeployment must include option to self host Web UI with service such as Ngnix
- Windows or Linux deployments should provide option to use existing web service to express the Web UI
- Windows Deployments must be fully self contained and NOT require user to install any outside applications as a prerequisite. 
- Poll the system for available network interfaces so these interfaces can be selected as part of the source and destination configuration.
- Self Hosted Web UI for administration and configuration
- Built on Open Source and Open License platforms
- Preferred code based options (though not limited to): REACT, RUST, WebRTC, TrueType, GStreamer.
- Build to secure API and UI with either basic password, API secret, and SSO connectivity using Key Cloak as the background. 
- Provide for a licensing pipeline that can be applied.
	- At this point licening method used is unknown
	- Licensing should provide access to use the application
	- Licensing liminting the number of sources that can be configured
	- Licensing liminting the number of destinations that can be configured. 

___
## Functional Requirements

### Audio Input (Sources) Requirements:
Audio Input sources will be both internal (within AWS) or Local to the server the application is operating on. 
#### External Sources:
- UDP Audio Streams with embedded GPI/O Data
- HLS Audio Streams streams with embedded GPI/O data
- SRT Audio Streams streams with embedded GPI/O data
- Icecast (AAC/MP3/Opus) streams with embedded GPI/O data
#### Internal Sources:
- SRT Audio Stream
- ALSA Audio Feeds
- RTP Audio Feeds
- Local Audio Device (Windows or Linux)
- AES67 (Livewire, Wheatstone, SAS, LAWO, Dante)

#### Configuration
All audio source inputs must be fully configured with in the application.  Each source will offer configuration that is specific to the type of input designated.  This information will be stored within the application and will be exposed via API call from the automation software.  Every source will provide a feed for Friendly name that will be used as a reference for this source via the API and on the routing tables. 
- SRT Internal or External source: Must provide options:
	- Friendly Name
	- Listener or Caller
	- Network interface to be used
	- IP Address or URL for Listener and caller
	- Port Number 
	- Passphrases: make and Optional field
	- Transport Type: MPEG-TS or RTP
	- Stream ID (optional Field)
	- Time Stamp Delivery (Yes, NO drop down)
	- Latency setting (ms)
	- Buffer settings
	- Set encription (yes or no)
	- Embedded GPI/O Data
	- (Other necessary configurable information)
- HLS: Must Provide:
	- Friendly Name
	- URL Connection
	- Embedded (yes or no)
	- Network interface selection
	- (Other necessary configurable information)
- UDP: Must provide
	- Friendly Name
	- Source IP Address
	- Port Number
	- Embedded GPI/O (yes or no)
	- Network interface selection
	- (Other necessary configurable information)
- Icecast (MP3/AAC/Opus)
	- Friendly Name
	- IP and Port Number to accept the Feed
	- Source audio type (MP3/AAC/Opus)
	- Bitrate settings
	- Network Interface
	- Embedded GPI/O (yes or no)
	- (Other necessary configurable information)
- RTP: Must Include
	- Friendly Name
	- Multicast address
	- Port Number
	- Network Interface
	- (Other necessary configurable information)
- AES67: Must Include
	- Friendly Name
	- Multicast address
	- Port Number
	- Network Interface
	- (Other necessary configurable information)
- ALSA: Must Include
	- Friendly Name
	- Channel Name
	- (Other necessary configurable information)
- Local Input: Must Include
	- Friendly Name
	- Drop down of available input channels
	- (Other necessary configurable information)
- GP I/O Options
	- Will be available to device server
	- Should include user ability to configure Friendly name
	- Designiation if it is an Input or Output
	- Based on Source provide a translation between text recieved as part of the embedded data and "Pin Number"
	- Translateion to device server to be a "Pin" that is used to in a way that will control the audiomation system.
	- Should have the ability to produce a single that can be interpreted as a GPI or GPO used to control automation systems
	- Per source configuration of relay delay.
- Now Playing metadata should be passed through to device server to be parsed as needed. 
- Target Automation system WideOrbit Automation for Radio
- Provide a per source option to test and monitor the audio coming in.  The monitor should provide the Web Browser to use default audio device to listen to the feed, and a visual VU meter to show audio coming in. 

### Audio Output (Destination) Requirements:
Audio will be routed from the selected Input source to a destination.  This destination will primarily be internal to the local network (either AWS or On Prem system), the destination should also allow for the sending of the audio to a matching application on Prem from AWS:

- SRT Internal or External destination: Must provide options:
	- Friendly Name
	- Listener or Caller
	- Network interface to be used
	- IP Address or URL for Listener and caller
	- Port Number 
	- Passphrases: make and Optional field
	- Transport Type: MPEG-TS or RTP
	- Stream ID (optional Field)
	- Time Stamp Delivery (Yes, NO drop down)
	- Latency setting (ms)
	- Buffer settings
	- Set encription (yes or no)
	- Embedded GPI/O Data
	- (Other necessary configurable information)
- HLS: Must Provide:
	- Friendly Name
	- URL Connection
	- Embedded (yes or no)
	- Network interface selection
	- (Other necessary configurable information)
- UDP: Must provide
	- Friendly Name
	- Source IP Address
	- Port Number
	- Embedded GPI/O (yes or no)
	- Network interface selection
	- (Other necessary configurable information)
- Icecast (MP3/AAC/Opus)
	- Friendly Name
	- IP and Port Number to accept the Feed
	- Source audio type (MP3/AAC/Opus)
	- Bitrate settings
	- Network Interface
	- Embedded GPI/O (yes or no)
	- (Other necessary configurable information)
- RTP: Must Include
	- Friendly Name
	- Multicast address
	- Port Number
	- Network Interface
	- (Other necessary configurable information)
- AES67: Must Include
	- Friendly Name
	- Multicast address
	- Port Number
	- Network Interface
	- (Other necessary configurable information)
- ALSA: Must Include
	- Friendly Name
	- Channel Name
	- (Other necessary configurable information)
- Local Input: Must Include
	- Friendly Name
	- Drop down of available input channels
	- (Other necessary configurable information)
- GP I/O Options
	- Will be available to device server
	- Should include user ability to configure Friendly name
	- Designiation if it is an Input or Output
	- Based on Source provide a translation between text recieved as part of the embedded data and "Pin Number"
	- Translateion to device server to be a "Pin" that is used to in a way that will control the audiomation system.
	- Should have the ability to produce a single that can be interpreted as a GPI or GPO used to control automation systems
- Now Playing metadata should be passed through to device server to be parsed as needed. 
- Target Automation system WideOrbit Automation for Radio

### Routing Requirements:
Audio routing will provide the user the ability to route audio sources to audio destinations.  This routing ability should allow an audio source be routed to one or many destinations.  Routing will be controlled by API command or Crosspoint Grid (X, Y).
- Route any source to any destination.
- Route any source to many destinations
- ability to control audio routes via API commands
- ability to make audio routes via Crosspoint (X,Y) grid 
- Ability to watch Cross point grid in real time if multiple users are logged in OR if Update is recieved via API. 

### API Requirements:
The API should be the entry point for remote configuration, remote control, and health point monitoring.
- Provide options for secure connectivity from an automation system (WideOrbit Automation for Radio is the target system via Device Server)
- Provide full configuration for Sources, Destinations, GPIO, and Routing from outside application
- Provide Health Monitoring 
- Provide Real time Audio Routing operations via API commands from Automation system.

### Other Requirements:

- GPIO should provide a visual representation of Relays as they are sent or recieved.  Example a circle that will flash green when the GPI or GPO is pulsed.  Or solid green if the relay is latched. 

---
# For POC Only:  Include in Prompt to build

- Security is not a requirement for the POC.  The UI and API should be protected with basic crednetions such as UN: Admin PW: Admin2026!
	- The build of the application should be built in a way that a more secure interface can be used with or without SSO auth using Key Cloak as the backend. 
- Provide and maintain a seperate folder with Plugin ready to deploy to WideOrbit Device Server.
- POC will need to be fully functional

---
## Expected Outcome



---

# Acceptance Criteria



---

## Definition of Done

