# Audio synchronization and multicam

Select timeline clips, then right-click → **Synchronize… / Синхронизировать…**.
Choose Audio, an analysis channel (all channels mixed, or source channel 1–32),
and optionally a reference clip or track. The source channel choice applies to
every selected recording. Missing channels, silence and ambiguous audio matches
return an error without changing the project. In/Out, timecode and marker alignment
remain available.

Linked video/audio move together. Clips can initially share a track. The operation
keeps their original tracks when the destination interval is free; otherwise it
uses another unlocked track of the same type or creates one. It never overwrites
unselected clips. Undo restores both placement and newly created tracks.

The cross-correlation compares the first 30 minutes of each selected source range
against the reference. Recordings need common audio; independent scenes, repeated
music or noisy microphones may not produce a reliable match. Select a cleaner
channel/reference or use markers/timecode. This aligns recording offsets, not
clock drift. Synchronize before speed changes, reversing or frame holds. Analysis
is local and currently runs synchronously, so long recordings can take time.

To create a multicam source without manual track preparation, select the camera
clips and right-click → **Create Multi-Camera Source Sequence…**. Audio synchronization
and channel selection are available in that dialog too. This uses original source
items (not the trimmed timeline ranges), putting each camera on its own track in
a separate sequence; originals remain intact. Insert that source into an editing
sequence and enable the Program monitor's Multi-Camera view to switch angles.

Validation includes generated stereo PCM with a known 12,345-sample offset,
phase-cancelled mixes, invalid/missing channels, same-track collisions, unrelated
occupied tracks, linked video/audio, locked tracks, Undo/Redo and UI channel selection.
