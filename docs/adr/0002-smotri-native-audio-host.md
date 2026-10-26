# ADR 0002: isolated audio-plugin hosting and Finder open events

Status: native boundaries approved by the Smotri owner on 2026-10-26.
First audio-effect host implemented for the 20261026.4 preview. See ../audio-plugins.md for supported scope.

## Audio plugins

Installed VST2, VST3 and Audio Units require a native host. No plugin library may be
loaded into the editor process. A separately packaged Rust helper owns all
native instances and is the only additional target allowed to use unsafe FFI.
The editor, project, render engine and IPC client retain `unsafe_code = forbid`.
All FFI must have SAFETY comments and checked boundaries. A helper is process
isolation, not an operating-system security sandbox.

Discovery probes candidates in disposable helpers with deadlines. A failed,
incompatible or hung plugin is reported and quarantined until explicitly
rescanned. System locations and user-selected locations are scanned; directory
recursion, candidate count, metadata sizes and IPC messages are bounded. The
catalog identifies plugins by format and stable native identifier, not name.
Architecture mismatch is reported rather than attempting to load incompatible
binaries. Legacy VST2 uses the MIT vst-rs adapter and is probed before registration;
finding a `.vst` file alone does not mean it is usable.

The same processing path must serve clip effects, mixer inserts and export.
Projects preserve identity, parameter values and opaque plugin state, including
when a plugin is missing. Realtime failures mute the processor and stop playback explicitly; exports
must fail visibly rather than silently produce a mix without an enabled plugin.
Latency, tails, seeks, rate changes, mono/stereo layouts, automation and state
restore require tests before release. UI lists categories and manufacturers;
plugin parameters and supported native editors must operate on the same state.

The helper ships with each supported desktop build. Plugin binaries, licenses,
activation data and local scan catalogs are never published with the product.
WASM does not host native plugins.

## Finder documents

The owner separately approved a small platform FFI boundary to receive Open
Documents Apple events. A bounded queue passes file paths to the UI. The Cocoa
callback never reads a project, mutates the session, or replaces the winit
application delegate. The UI asks about unsaved changes and uses the existing
project reader. Failed reads retain the existing project.

Keep `.fcproj` compatibility; no conversion is needed for Finder opening. Bundle
metadata declares the Smotri document icon explicitly.

## Sources

- https://www.steinberg.net/developers/vstsdk/ — current VST3 MIT terms.
- https://developer.apple.com/documentation/audiotoolbox/auaudiounit — AU hosting.
- ADR 0001 remains applicable to the existing OS codec boundary.
