# Device Server Plugin Guide for a New Audio/GPIO Router

## Scope

This guide describes the plugin interface implemented by this Device Server
source tree. It is based on the C++ API and existing plugins, not on a
separate, language-neutral plugin protocol. The new router can run as an
external application, but the adapter loaded by Device Server is a native
shared library that translates Device Server messages to and from that
application.

The relevant API definitions are in
[device_plugin.h](../api/ddserver/device_plugin.h),
[abstract_hardware_plugin.h](../api/ddserver/abstract_hardware_plugin.h),
[abstract_control_hardware_plugin.h](../api/ddserver/abstract_control_hardware_plugin.h),
[abstract_gpio_plugin.h](../api/ddserver/abstract_gpio_plugin.h), and
[abstract_switch_plugin.h](../api/ddserver/abstract_switch_plugin.h).

## Recommended shape of the adapter

Choose the base classes for the capabilities the adapter will expose:

| Adapter responsibility | Base class pattern | What it provides |
|---|---|---|
| Device identity, health, reset/lock messages | `AbstractHardwarePlugin` | Device lifecycle, status, ping timer, and standard device messages |
| Communicate through an existing serial/TCP I/O plugin | `AbstractControlHardwarePlugin` | `device_io_plugin` configuration, I/O open/close notifications, incoming data callback, and `postMessageToDevice()` |
| Expose GP inputs/outputs to automation | `AbstractGPIOPlugin` | GPIO messages, state masks, feature reporting, change notifications, and input debounce support |
| Expose audio crosspoints | `AbstractSwitchPlugin` | Audio input/output counts, crosspoint messages, and change notifications |

For a router that supports both audio routing and GPIO, follow the composition
used by the Axia control plugin:
`AbstractControlHardwarePlugin`, `AbstractSwitchPlugin`, and
`AbstractGPIOPlugin`. For a minimal GPIO-only/test adapter, see
[mock_gpio_plugin.h](../plugins/mock_gpio_plugin/mock_gpio_plugin.h) and
[mock_gpio_plugin.cc](../plugins/mock_gpio_plugin/mock_gpio_plugin.cc).

`AbstractControlHardwarePlugin` already derives from
`AbstractHardwarePlugin`; do not add `AbstractHardwarePlugin` a second time.
If the new application owns its transport and does not use a Device Server I/O
plugin, use the hardware base and implement/manage that transport directly.

## Required implementation work

### 1. Implement the native plugin class

The plugin class must derive from `ddserver::Plugin` through the selected base
classes. The abstract requirements visible in this API include:

- `main()` for the plugin message loop. `AbstractHardwarePlugin::main()` is
  empty, so hardware plugins that dispatch messages must provide their own
  loop; see the mock and vendor plugins.
- `pingDevice()` for `AbstractHardwarePlugin`, unless the plugin has a
  deliberate alternative health strategy.
- `onReceiveMessageFromDevice()` for
  `AbstractControlHardwarePlugin`.
- `commitPendingGPIOs()` and `doPulseGPOutputs()` for
  `AbstractGPIOPlugin`.
- `commitPendingAudioXpoints()` for `AbstractSwitchPlugin`.

Initialize GPIO counts with `setGPIOCounts(inputs, outputs)` and audio
crosspoint dimensions with `setInputOutputCounts(inputs, outputs)`. The GPIO
base represents each input and output as a bit in a `Bitset`; its documented
range is 0-64 channels for each side.

The subclass must translate hardware/application state into the base class's
pending masks and call `processGPInputChanges()` or
`processGPOutputChanges()` when state changes. For outputs, implement
`commitPendingGPIOs()` to apply changed pending bits to the router. For audio,
commit the pending crosspoint masks and publish changes with the switcher base
helpers. Do not treat successful command transmission as confirmed physical
state unless that is the intended behavior; existing plugins differ on whether
they notify optimistically or after receiving status.

### 2. Use the Device Server GPIO message contract

The message names are defined in
[plugin_message_defs.h](../api/ddserver/plugin_message_defs.h). The main
requests and responses are:

| Request | Response/event | Purpose |
|---|---|---|
| `gpio-get-features` | `gpio-features` | Counts, pulse support, index base, and available channel metadata |
| `gpio-get-inputs` | `gpio-inputs` | Read current input mask |
| `gpio-get-outputs` | `gpio-outputs` | Read current output mask |
| `gpio-set-inputs` | `gpio-inputs-changed` | Used by test/simulation plugins to set input state |
| `gpio-set-outputs` | `gpio-outputs-changed` | Set output state |
| `gpio-update-outputs` | `gpio-outputs-changed` | Apply on/off masks to output state |
| `gpio-pulse-outputs` | `gpio-outputs-changed` when applicable | Pulse selected outputs |
| `gpio-set-input-debounce` | `gpio-input-debounce` | Configure per-input debounce |

Masks are `DSVariant` values. Use `MaskHelper::decodeMask()` and
`MaskHelper::encodeMask()` rather than inventing a second bit ordering or
serialization. The mask keys are `gpinputs_mask`, `gpoutputs_mask`,
`gpinputs_on_mask`, `gpinputs_off_mask`, `gpoutputs_on_mask`, and
`gpoutputs_off_mask`.

The base class builds `gpio-features` with `num_gpinputs`,
`num_gpoutputs`, `num_relays`, `num_collectors`, `relays_first`,
`pulse_support`, `pulse_adjustable`, and `one_based`. It can also report
enabled-channel masks and channel names. Populate relay/collector counts and
ordering only when those categories are meaningful for the new device; they
share the output mask but describe different physical output types. Keep the
reported base consistent with the bit indices and the external channel labels.

Treat GPIO state as logical on/off state at the Device Server boundary.
Electrical polarity and the relationship between logical "on" and voltage or
contact state belong in the adapter's protocol translation. Define and test
that mapping explicitly.

### 3. Connect to the new application

If the router exposes a TCP or serial protocol, the usual integration is:

1. Configure an existing I/O plugin (for example `tcp_client` or
   `serial_port`).
2. Set the adapter's `device_io_plugin` setting to that I/O plugin instance.
3. Use `onIoChannelOpened()` to initialize/subscribe/query the router.
4. Parse received device data in `onReceiveMessageFromDevice()`.
5. Send commands through `postMessageToDevice()`.

This is the contract of `AbstractControlHardwarePlugin`; it handles
communication with the I/O plugin, not the new application's routing protocol.
The adapter still owns message parsing, channel mapping, acknowledgements,
timeouts, and reconnect/resynchronization behavior.

If the new application already has a Device Server-compatible message
endpoint, the adapter can translate messages to that endpoint instead. It
still needs to export the standard native plugin factory and metadata below.

### 4. Export metadata and build a shared library

Every loadable module needs one `DDSERVER_EXPORT_PLUGIN` invocation. The macro
exports the factory and the plugin's name, version, author, class name, and
build date. See
[plugin.h](../api/ddserver/plugin.h) and the existing export in
[axia_control_plugin.cc](../plugins/axia_control_plugin/axia_control_plugin.cc).

Add a CMake shared-library target, compile the plugin sources, and link against
`ddserverapi`, following
[mock_gpio_plugin/CMakeLists.txt](../plugins/mock_gpio_plugin/CMakeLists.txt)
or
[axia_control_plugin/CMakeLists.txt](../plugins/axia_control_plugin/CMakeLists.txt).
For a source-integrated build, add the target to `LIST_OF_PLUGINS` in the
root [CMakeLists.txt](../CMakeLists.txt); the root project adds each listed
plugin directory and installs its runtime artifact under the plugin output
directory.

Build against the same Device Server API, platform, compiler/runtime, and
configuration as the service. This is an in-process native module, so ABI or
dependency mismatches can prevent loading or cause runtime failures.

## Drop-in test workflow

1. Build the module for the service's platform: `.dll` on Windows or `.so` on
   Linux. Use a unique exported plugin name and version.
2. Copy the module into the plugin directory scanned by this service build
   (the CMake build outputs plugins in its `plugins` output directory). The
   manager scans that directory for native modules, loads them, and reads
   their exported name and version. The service-managed installation path
   uses `<name>-<version>` in the library filename; using that convention for
   a manual drop is recommended. The scanner skips symbolic links.
3. Add a plugin instance to the service configuration. Follow the
   `plugin_type` / `autostart` / `settings` structure in
   [ddserver.w32.cfg](../src/ddserver.w32.cfg) or
   [ddserver.cfg](../scripts/ddserver.cfg). `plugin_type` must match the exported
   plugin name. For a live `PLUGIN_ADD_REQ`, the requested version must also
   match the exported version.
4. For a transport-backed adapter, start/configure its I/O plugin as well and
   set `device_io_plugin` to that instance name.
5. Start or restart the service so it discovers the module. Confirm the
   module appears in the plugin list and that the instance starts without
   `plugin load`, configuration, or connection errors.
6. Exercise `gpio-get-features`, both state queries, output set/update and
   pulse requests, and input/output change events. Test channel zero and the
   last channel, one-based labeling, empty and invalid masks, device
   disconnect/reconnect, and status reconciliation.
7. If audio routing is exposed, test source/destination enumeration,
   crosspoint set/update, status feedback, and reconnect recovery separately
   from GPIO.

The existing test patterns are in
[abstract_gpio_plugin_test.cc](../device_server_test/abstract_gpio_plugin_test.cc)
and the plugin-specific tests listed by
[device_server_test/CMakeLists.txt](../device_server_test/CMakeLists.txt).
The mock plugin is useful for API-level smoke testing, but it does not prove
the new application's protocol, polarity, timing, or hardware feedback.

## Suggested acceptance checklist

- [ ] Plugin metadata name/version match the instance configuration.
- [ ] The library loads with the production service build and all dependencies
      resolve.
- [ ] The plugin initializes counts before responding to feature/state
      requests.
- [ ] The feature response describes channel counts, names, base numbering,
      relay/collector layout, and pulse capabilities accurately.
- [ ] Every state transition emits a correctly encoded change event.
- [ ] Output commands are serialized safely; returned/observed state is
      reconciled with hardware or the application.
- [ ] Pulse duration, retrigger behavior, and whether pulse is native or
      emulated are tested.
- [ ] Disconnect, restart, and topology/count changes do not leave stale
      masks or subscriptions.
- [ ] Logs identify the plugin instance and report connection/protocol errors.
