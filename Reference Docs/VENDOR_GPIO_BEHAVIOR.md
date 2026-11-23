# GPI/O Behavior in the Device Server Vendor Plugins

## How to read this document

This is a summary of the implementations checked into this source tree, not
vendor protocol specifications. A vendor may support more capabilities than
this adapter exposes. The common API is logical bit masks; a `true` bit is
not, by itself, a guarantee about electrical voltage, relay contact position,
or active-high/active-low wiring. Use each plugin's translation code and the
actual device documentation when designing a product-level abstraction.

Nine plugin classes in this tree derive from `AbstractGPIOPlugin`: three Axia
adapters, Broadcast Tools, Clear-Com, Lawo Console, SAS, Wheatstone Console,
and the mock adapter. Other vendor plugin directories do not derive from
this GPIO base in the checked-in code, so their GPIO support is not exposed
through this service GPIO message contract here.

## Common Device Server model

`AbstractGPIOPlugin` maintains separate pending and active bitsets for inputs
and outputs. Subclasses discover or configure channel counts with
`setGPIOCounts()`, translate device events into pending input/output masks,
and call `processGPInputChanges()` or `processGPOutputChanges()` to publish
state changes. Output commands are committed by the vendor subclass.

The common feature response can include input/output counts, relay and
open-collector counts, whether relays appear first, pulse support and
adjustability, one-based numbering, enabled masks, and names. The output mask
is a flat index space even when a plugin also reports relay and collector
sub-counts. Channel masks are encoded/decoded through `MaskHelper`.

The message contract is defined in
[plugin_message_defs.h](../api/ddserver/plugin_message_defs.h), and common
state/feature handling is in
[abstract_gpio_plugin.cc](../api/abstract_gpio_plugin.cc).

## Vendor comparison

| Vendor adapter | How counts are determined | Output and pulse behavior | Readback / notable mapping |
|---|---|---|---|
| Axia Control (LWRP) | Reads `NGPI` / `NGPO` from device version data and scales ports by five pins | Sends port bit strings; true logical output is sent as low (`L`). Supports adjustable pulse and configurable GPIO routing | Parses per-port `GPI` / `GPO` status letters; can configure crossover/routing between GPIO ports |
| Axia LWCP | Parses the `AppControl GpioList` XML; its fader discovery path also sizes GPIO masks from fader counts | Output changes command fader channel on/off and gain; pulse handler currently returns without sending a pulse | Reads a vendor XML list of input/output functions; output feedback/state is tied to fader status |
| Axia Pathfinder | Discovers GPIO router nodes and sums five pins per port; input/output counts are treated as symmetric | Sends per-port trigger commands; pulse state is cleared by a timer; pulse support and adjustable-pulse flags are set | Parses `GPIStat` / `GPOStat`, has GPIO names/enabled state, one-based labels, and route-stat crossover support |
| Broadcast Tools switchers | Model table supplies GPI, relay, collector, ordering, polarity and pulse capability | Changed outputs are sent as latch commands; uses native pulse where the model supports it, otherwise emulates a timed pulse | Parses single/all GPI and output status reports; some models invert GPI bits |
| Clear-Com switchers | Model table supplies GPI and relay/collector counts and ordering | Sends binary output state per card; pulse is emulated by toggling and restoring later | GPI status is parsed from binary card messages; active-low output/input mapping and per-card bit layout matter |
| Lawo Console | Configured `number_gp_inputs` and `number_gp_outputs` | Sends a framed/checksummed binary set command; pulse is emulated with a timer | Receives GPI event frames with channel/state bytes; this implementation does not subdivide outputs into relay/collector categories |
| SAS Router | Device/model table supplies default counts, which configuration can override | Sends on/off/pulse actions; one pulse request with multiple bits currently sends only the first selected output | Parses numbered `Z00` input and `Z01` output status; one-based channel numbers; input high events are temporarily blocked/debounced |
| Wheatstone Console | Configured `number_gp_inputs` / `number_gp_outputs`, defaulting to 64 each if absent | Direct Console mode maps outputs to fader commands; Blade-style mode uses soft-pin commands; pulses use the configured timer interval | Supports multiple console/control modes, uses one-based labels, receives soft-pin/source events; pins may be logical controls rather than physical GPIO |
| Mock GPIO | Fixed at 8 inputs and 32 outputs | In-memory output state; pulse request is a stub | Intended for testing message/state behavior, not physical I/O |

## Adapter details

### Axia Control (LWRP)

The plugin reads device version information including the number of GPI and
GPO ports. It multiplies each count by the five I/O bits per port and
initializes the common masks. Device status arrives as per-port `GPI` / `GPO`
messages containing five `l`/`h` states. In this adapter, low (`l`/`L`) maps
to a set logical bit and high (`h`/`H`) maps to a cleared bit. For output
writes, changed set bits are emitted as `L`, cleared bits as `H`, and
unchanged bits as `x`.

The plugin advertises pulse support and adjustable pulse, and has a
`gpout_pulse_width` setting. It also supports `gpio_routing` entries, including
source/destination and function/pulse configuration, plus per-port crossover
settings. Crossover can make output status feed the logical input mask. Device
protocol port numbers are one-based; the message API itself operates on mask
indices.

References: [axia_control_plugin.cc](../plugins/axia_control_plugin/axia_control_plugin.cc),
[axia_control_plugin.h](../plugins/axia_control_plugin/axia_control_plugin.h).

### Axia LWCP

This is a separate Axia adapter from the LWRP and Pathfinder plugins. It parses
`AppControl GpioList` data, whose XML entries identify input and output
functions, and sizes the GPIO masks from those lists. Its fader-list discovery
path also uses discovered fader/output counts to size GPIO masks.

When an output bit changes, the plugin sends `SET FaCH#... ON_State=...` and
`SET FaCH#... Fader_Gain=...` commands, and updates its tracked state. Its
`doPulseGPOutputs()` currently returns success without emitting a pulse
command. Do not infer native pulse capability from the common message being
accepted.

References: [axia_lwcp_control_plugin.cc](../plugins/axia_lwcp_control_plugin/axia_lwcp_control_plugin.cc),
[axia_lwcp_control_plugin.h](../plugins/axia_lwcp_control_plugin/axia_lwcp_control_plugin.h).

### Axia Pathfinder

The plugin discovers GPIO router nodes among Pathfinder router data. It sums
five I/O pins per port and treats the total GPI and GPO counts as symmetric.
It sets the API channel numbering to one-based and obtains input/output names
and enabled status from GPIO source/destination-name responses.

It parses asynchronous `GPIStat` and `GPOStat` reports into bit masks. For
output changes, it builds a trigger command for the affected GPIO router and
port; if configured to request confirmation, it also requests GPO status.
Pathfinder route statistics can mark a port as crossed, in which case output
commands/status are translated through the corresponding input path. Pulse
requests set selected outputs and use a timer to clear them. The plugin
advertises pulse support and adjustable pulse.

References: [axia_pathfinder_plugin.cc](../plugins/axia_pathfinder_plugin/axia_pathfinder_plugin.cc),
[axia_pathfinder_plugin.h](../plugins/axia_pathfinder_plugin/axia_pathfinder_plugin.h),
[axia_router_data.h](../plugins/axia_pathfinder_plugin/axia_router_data.h).

### Broadcast Tools switchers

The Broadcast Tools plugin selects a model from its device table. The table
provides GPI count, relay count, open-collector count, relay ordering, GPI
polarity, and whether pulse is supported/adjustable. Total output count is
`numRelays + numCollectors`; the common mask carries both output kinds, with
the relay/collector boundary described separately in features.

GPI status can be reported one input at a time or as a complete input mask.
The complete input parser applies the model's `flipGPIbits` setting. Relay and
collector status reports update the corresponding portions of the output
mask. Output state changes are translated to per-channel latch commands.
Pulse is native for models marked pulse-capable; otherwise the plugin turns
the requested outputs on and schedules restoration after the configured
pulse interval. Requests for bits already being pulsed are ignored.

The plugin is configured with a `device_type` and a `device_io_plugin`, which
typically connects it to a serial I/O instance.

References: [bt_switch_control_plugin.cc](../plugins/bt_switch_control_plugin/bt_switch_control_plugin.cc),
[bt_switch_info.h](../plugins/bt_switch_control_plugin/bt_switch_info.h),
[bt_messages.cc](../plugins/bt_switch_control_plugin/bt_messages.cc).

### Clear-Com switchers

The Clear-Com model table provides GPI, relay, and collector counts and
ordering. The plugin communicates with binary messages through an I/O plugin.
Input status is requested from the switcher and also parsed from card status
messages. The implementation uses eight exposed GPI bits per card from a
larger 20-bit message field; its comments document which hardware bits are
discarded and how the remaining bits are indexed across cards.

The code treats GPI and GPO as active-low in its device translation. Output
updates are constructed per card, with output bits inverted before building
the hardware message. Pulse is emulated by toggling the selected output and
restoring it on a later timer tick; do not assume pulse is a native switcher
operation.

References: [cc_switch_control_plugin.cc](../plugins/cc_switch_control_plugin/cc_switch_control_plugin.cc),
[cc_switch_info.h](../plugins/cc_switch_control_plugin/cc_switch_info.h),
[cc_messages.cc](../plugins/cc_switch_control_plugin/cc_messages.cc).

### Lawo Console

The adapter requires a configured I/O plugin and explicit
`number_gp_inputs` / `number_gp_outputs` values. It receives binary GPI event
frames, taking the input channel from byte 3 and the state from byte 4, then
publishes the updated input mask.

Changed output bits are encoded as a framed, checksummed binary command using
the output index and boolean level. Pulse is implemented in software: selected
outputs are set and a timer clears them after the configured pulse interval.
The implementation exposes generic output channels and does not report relay
versus open-collector sub-counts.

References: [lawo_console_control_plugin.cc](../plugins/lawo_console_control_plugin/lawo_console_control_plugin.cc),
[lawo_console_control_plugin.h](../plugins/lawo_console_control_plugin/lawo_console_control_plugin.h).

### SAS Router

The plugin starts with model-derived GPI/GPO counts and allows
`number_gp_inputs` and `number_gp_outputs` to override them. The feature base is
one-based. Input and output reports are parsed as numbered `Z00` / `Z01`
messages, with one-based wire channel numbers converted to zero-based mask
indices.

Output commands support ON, OFF, PULSE, and QUERY actions. A pulse message
containing more than one selected bit currently sends a command for only the
first set bit; this is visible in the explicit `break` in the pulse handler.
The input parser suppresses repeated "on" opto events for 750 ms, and an
output-on action can temporarily block matching opto feedback.

References: [sas_router_control_plugin.cc](../plugins/sas_router_control_plugin/sas_router_control_plugin.cc),
[sas_router_control_plugin.h](../plugins/sas_router_control_plugin/sas_router_control_plugin.h).

### Wheatstone Console

This adapter covers more than one control mode. It defaults to 64 GPI and 64
GPO channels and uses configured `number_gp_inputs` / `number_gp_outputs`
values when positive values are present. It marks API numbering as one-based.
The configured device type distinguishes direct Console mode from Blade,
Bridge, or Router-style configurations.

Output writes in direct Console mode are mapped through a fader table and
produce `<INPUT:...|ON:...>` commands. Other modes write soft-pin
`<SLIO:...|LVL:...>` commands. Input changes can arrive through soft-pin and
source/fader event parsing; output status is also updated from console
messages. The pulse path uses a timer, with a default interval of 500 ms and
configured values constrained by the plugin. Direct mode has special handling
for fader "next" pins; the Blade path treats selected pulse outputs as pulses.

These are logical console/soft-pin controls, not a guarantee that every
configured channel corresponds to a physical GPIO connector. The plugin's
`AbstractGPIOPlugin::init()` call is commented out with a TODO, so validate
GPIO-base initialization and feature/name behavior for the exact mode being
tested.

References: [wheatstone_console_control_plugin.cc](../plugins/wheatstone_console_control_plugin/wheatstone_console_control_plugin.cc),
[wheatstone_console_control_plugin.h](../plugins/wheatstone_console_control_plugin/wheatstone_console_control_plugin.h).

### Mock GPIO

The mock adapter has eight inputs and 32 outputs. It stores state in memory
and immediately publishes changes. It implements setting simulated input
state, but its pulse handler is a stub. This is a useful reference for the
Device Server message contract and a poor stand-in for hardware polarity,
timing, confirmation, or reconnection behavior.

References: [mock_gpio_plugin.cc](../plugins/mock_gpio_plugin/mock_gpio_plugin.cc),
[mock_gpio_plugin.h](../plugins/mock_gpio_plugin/mock_gpio_plugin.h).

## Design takeaways for the new router

1. Keep logical GPI/GPO state separate from electrical polarity. Axia
   LWRP/Pathfinder and Clear-Com perform active-low translation; Broadcast
   Tools can invert GPI state per model.
2. Treat relay and open-collector outputs as types in metadata over a shared
   output index space. Preserve their order and base mapping.
3. Model pulse as an explicit capability with native/emulated behavior,
   duration limits, retrigger policy, and whether completion is confirmed.
   The existing adapters range from native commands to timer emulation to a
   pulse stub.
4. Do not assume every vendor has fixed channel counts or physical-pin
   mapping. Counts may come from device discovery, model tables, or
   configuration; console soft pins and fader controls may be logical GPIO.
5. Preserve source/destination names, enabled channels, and crossover/route
   relationships when the underlying device exposes them.
6. Test the adapter's feedback path separately from command transmission so
   automation sees observed state rather than a stale or optimistic mask.
