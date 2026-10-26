# Export with a maximum file size

In Export mode, choose H.264 and enable **Compress to** beside the estimated file size.
Enter decimal megabytes: 1 MB = 1,000,000 bytes. The setting is saved with presets and
queue items and is available to agents as `settings.targetSizeBytes` in
`file.exportMedia`, `export.resolve`, and `export.queue.add`.

The editor budgets video and audio using the selected export range, reserving 4%
plus 32 KiB for container overhead. It selects 32–128 kbps AAC mono/stereo and the
built-in software H.264 encoder in two-pass VBR mode. Frame size and frame rate are
preserved. This does not invoke FFmpeg or require extra software.

The completed MP4/MOV is measured before publication. If it exceeds the limit,
the editor lowers the video bitrate and renders both passes again from the original
sequence, at most three attempts total. It never repeatedly compresses a compressed
intermediate. An impossible budget or exhausted attempts produces an error instead
of delivering an oversized file. Native exports use unique temporary files next to
the destination, removed on error/cancellation. An existing destination is replaced
only by an accepted result; filesystem replacement failures leave it intact.

The limit is a maximum, not an exact size. This is lossy compression, not lossless:
small budgets can visibly reduce detail. No VMAF/SSIM quality guarantee is made.
Limits are 1–100,000 MB for native files, and up to 256 MB for in-memory/web output.
H.265, AV1, image sequences and reserved batch-output handles are not supported by
this mode. Captions exported as a separate sidecar are not included in the video limit.

Validation covers hostile budgets, duration and audio preservation, settings/queue
round-trips, UI controls in Russian and English, temporary-file cleanup and bounded
retries with an intentionally oversized metadata payload. Optional FFprobe is used
only as an external duration/stream test oracle.
