# OpenScreen projects

File → Import or File → Open accepts OpenScreen v2 `.openscreen` projects. Open
creates an unsaved native document after validating the recording; Save As writes a
new `.fcproj`. Import is one undoable edit that adds a project bin and retains
existing sequences. Dropping a project onto the timeline inserts its main sequence
as a nested clip, using available tracks; an empty timeline gets a new sequence.
Dropping a native `.fcproj` uses its saved active sequence. The original document
is never rewritten. macOS Open With and command-line opening also accept `.openscreen`.

The main sequence has full-length video tracks for Picture in picture, Dual screen,
Desktop and Webcam (camera-dependent variants are omitted without a camera). Only
Picture in picture is enabled initially; use track eye toggles to choose a layout,
or cut/enable the nested clips to switch layouts over time. Audio plays once from
one shared audio track. The original authored layout cuts remain in `Original edit`.

Each layout opens directly to native desktop, camera, background and cursor clips,
with separate recorded audio streams. The first authored geometry of each layout is
used across that angle; the original sequence preserves all composition animation.
Transform and mask keys are editable Smotri parameters sampled at the recording's
frame rate. Camera offset and end-frame holds are preserved.

Select the **Picture in picture** clip in the main sequence and open **Effect
Controls → OpenScreen Zoom**. The effect is automatically added during import,
including the recorded zoom intervals. **Zoom Active** is an animatable checkbox:
a true key starts zooming in, a false key starts zooming out. Before the first key
zoom is off. Move/delete these keys to edit intervals; Scale and In/Out Duration
control magnification and smoothness. Follow recorded cursor follows the imported
cursor path. Disable or delete the effect to return to the unzoomed composition.

Zoom is evaluated at render time, not baked into Motion keys. It transforms desktop
and cursor layers of this PiP clip only; camera, audio and other layout angles stay
unchanged. The original authored sequence retains its legacy animation. The old
Clip/context-menu zoom dialog has been removed. For automation,
`openscreen.addZoom {start?, duration, scale, follow}` now adds visible effect keys
to a selected PiP clip. Times are main-sequence seconds; scale is percent.

Projects imported by the earlier baking implementation retain their existing
Motion keys. Re-import the original `.openscreen` into a new project to get the
editable effect and clean layout variants; no existing edits are silently removed.

Supported data:

- Picture in picture, dual frame, vertical stack and hidden camera layouts.
- Same-layout interpolation and cuts between layout presets, crop, mirror and masks.
- Zoom regions, automatic cursor-follow focus, connected zoom pans and spring motion.
- Captured cursor positions, visibility, bitmap changes, smoothing and click bounce.
- Trimmed ranges and speed regions, applied to linked nested video and audio.
- Embedded PNG/JPEG/WebP wallpaper and cursor images, extracted next to the document
  into `<name>.openscreen-media/` without overwriting existing files.
- Built-in `/wallpapers/...` images resolved from an installed OpenScreen or a
  `wallpapers` folder beside the project, then copied into that media folder.
- Sparse cursor visibility switches: only the currently visible cursor receives
  motion keys, so recordings with many cursor shapes do not multiply animation data.

Keep the original screen/camera files and `<screen filename>.cursor.json` beside the
project (or at their stored paths). Save the imported result as `.fcproj`; it can
then be edited, reopened and exported without OpenScreen. The extracted media and
recording files must remain available, just like other linked project media.

## Limits

This is an editable reconstruction, not a pixel-identical OpenScreen renderer.
OpenScreen replacement cursor themes, motion blur, shadows and 3D perspective are
not reproduced; the command's `report` lists these differences. Cursor graphics come
from the recording, not from bundled system artwork. Annotations are rejected with
an actionable error instead of silently losing them. Non-file wallpapers other
than black are currently unsupported. Only version 2 documents are accepted.

Inputs are capped: project 128 MiB, cursor JSON 32 MiB, recording 2 hours, sampled
motion 120,000 frames, animation budget 1,000,000 layer-frames. Missing media,
invalid geometry/times, excessive data and unsupported versions fail before the
project edit. An unsuccessful import can leave extracted, unreferenced images but
never overwrites an existing file or mutates the open project.

## Provenance and validation

Motion/layout semantics are adapted from the MIT-licensed OpenScreen implementation
(`compositionKeyframes`, `compositeLayout`, `zoomRegionUtils`, `zoomSpring`,
`cursorFollowUtils`, `cursorPathSmoothing`). See [the MIT notice](../THIRD-PARTY-OPENSCREEN.txt).
No recorded media or cursor artwork is distributed with Smotri.

`cargo test -p filmcraft-engine openscreen --lib` exercises malformed input, layout
interpolation/cuts, camera offset, separate cursor tracks, nested rendering,
save/reopen, single Undo and non-overwriting asset extraction using generated media.

OpenScreen/Chromium H.264 camera recordings may use Annex-B headers in a Matroska
`.webm` file, or omit CodecPrivate and store SPS/PPS in the first packet. Smotri
normalizes those headers and NAL packets for its existing
software/hardware AVC decoders; no conversion, external encoder or extra download
is required. Import verifies that both screen and camera can decode a frame before
adding the sequence.
