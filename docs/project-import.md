# Import another Smotri project

Use **File → Import Project…** to select a saved `.fcproj`. The ordinary **Import…**
(Cmd+I / Ctrl+I) dialog and dropping a `.fcproj` into the editor also import it.
**Open Project…** still opens a separate document instead.

The import creates a folder named after the source project in the current Project bin.
It copies its sequences, nested sequences, media items, folders, captions, markers,
effects/keyframes, mixer routing, multicam angles, transcripts, source graphics, narration
settings and embedded LUTs. Project notes are appended under the source project name.
Destination sequence/settings, save path and existing edits are retained.

Importing the same saved project into itself or importing it repeatedly creates independent
copies of its sequences and media items. All project IDs and internal references are remapped.
The original files on disk are referenced, not duplicated; paths relative to the imported
project are resolved from that project's folder, including proxies and image sequences.
Keep those media files accessible. Missing/refused files are reported and can be relinked;
they do not prevent the remaining project from importing.

Each project import is one Undo step. Save the destination normally. This imports the
**saved file**, not unsaved edits in another window. Workspace settings, API keys, application
preferences and scratch/ingest configuration from the source are not imported.

Automation: `file.importProject {"path":"/path/source.fcproj","bin":optionalBinId}`.
Returns `bin`, `sequences` and `missingMedia`. `file.import {"paths":[...]}` includes the same
report in `documents`. The native format parser handles schema migrations. Too-new, malformed,
overlarge (256 MB), cyclic, or ID-exhausted projects fail before changing the destination.
