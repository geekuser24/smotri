# Installed audio plugins

Smotri hosts installed **VST 2.4 / VST3 effects** on desktop, and **Audio Units
(aufx/aumf)** on macOS. The helper is included in desktop archives; no separate
host installation is needed. Plugins themselves must be installed and activated
by their vendors, with the same CPU architecture as Smotri.

## Use

1. Open **Settings → Plugins** (Настройки → Плагины).
2. Add optional VST/VST3 folders, one per line, and click **Scan audio plugins**.
   Standard system/user folders and registered macOS Audio Units are included.
3. In **Effects → Audio Effects**, find the installed effect by name or its
   vendor/category group. Apply it to an audio clip, or choose it in an Audio
   Track Mixer insert slot (including submix and master strips).
4. Edit its exposed parameters in Effect Controls or the mixer effect editor.
   Parameters support normal project keyframes. Undo/redo and project saves
   preserve the descriptor, initial opaque state and edited parameter values.

The initial release uses **Smotri's parameter editor**, not the vendor's custom
plugin window. MIDI/instruments, external sidechains, tempo/transport-synced
processing, VST2 shell containers, AUv3 extension hosting and surround plugin
buses are not implemented. Mono/stereo audio effects are supported. AU components
must be installed in macOS component locations; extra folder scanning is for VST.
Plugin names and parameter labels come from their vendor, not UI translations.
VST controls currently show normalized values from 0 to 1; AU controls use native ranges.

## Reliability and limits

Every native instance runs in a disposable process. Scans and initialization have
20-second request deadlines; processing has a 2-second deadline. This contains
crashes/hangs but is not a security sandbox. Install plugins only from sources
you trust. Scanning does not load a plugin into the editor process.

A missing, incompatible or failed enabled effect produces an explicit error;
playback stops and export fails, rather than silently exporting unprocessed audio.
Disable/remove the failed effect, or use **Restart audio plugins** after resolving
the issue. Rescan after installing/updating plugins; restart Smotri after a plugin
update changes its parameter layout. Project-supplied library paths cannot bypass
the local scanned catalog.

Discovery is bounded to 512 modules, 512 registered effects, 512 parameters per
registered effect, 32 MiB of catalog data and 32 simultaneously active workers.
Limits are reported, not treated as successful support. A plugin reporting a
latency change during processing is rejected to avoid incorrect compensation.
Fixed latency is compensated in clip/mixer rendering. Effects retain their state
across contiguous buffers; seeking rebuilds their processing state and preroll.
Reverb/delay outside the sequence/export range is not appended automatically.

## Implementation and verification

- `crates/project/src/audio_plugin.rs`: serializable identity/state and validation.
- `crates/audio-plugin-client`: safe authenticated localhost IPC, catalog, failure latch.
- `apps/audio-plugin-host`: isolated VST2/VST3/AU adapters; the only native host boundary.
- `crates/render`: identical processors for clip effects, mixer inserts and export.
- Commands: `audioPlugins.scan`, `audioPlugins.list`, `audioPlugins.reset`.

Local compatibility checks use separately installed OTT (VST2/VST3), Valhalla
Supermassive (VST3) and Apple AUDelay (AU). Their binaries and scan data are never
committed or distributed. These checks do not establish compatibility with every
plugin. Automated tests cover malformed IPC, state bounds, project round trips,
missing plugins, and export failure propagation.
