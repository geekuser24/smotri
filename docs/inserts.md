# Inserts / Вставки

Window → Inserts opens a normal dockable FilmCraft panel. It follows the application's theme,
can be moved/floated like the other panels, and is included next to Project in the default editing
layout. Existing saved workspaces can open it from Window. Russian UI title: «Вставки».

Settings → Inserts stores the GIPHY API key in macOS Keychain (service
`FilmCraft.Inserts.GIPHY`, account `api-key`). The masked draft is cleared on Cancel and is never
serialized in project, preference or inspection JSON. Other desktop platforms support
`GIPHY_API_KEY` at startup and in-memory key changes. No key is bundled in the app or repository.

Search sends the user's exact query (1–50 characters) directly to GIPHY's Search API. It requests
24 GIFs per page, uses `pg-13` rating, and selects Russian/English language hints from the query.
The Sources checkbox includes/excludes GIPHY. No Tenor connection is advertised as functional.
The panel displays static thumbnails, result titles, source links, and Powered By GIPHY attribution.
Network requests and downloads run on guarded workers with finite timeouts/response sizes.

Import downloads the original animated GIF (maximum 25 MiB), adds it through `file.import`, and
opens it in the Source monitor. Drag the imported project item onto the timeline. Files are stored
in an `Inserts` directory beside a saved project, or in FilmCraft's application data directory for
an unsaved project. Undo removes the project import; it does not erase downloaded files. Changing
the project during download rejects automatic import into the changed project.

Control/automation:
- `window.panel.Inserts`, `app.settings.inserts`
- `inserts.search {query, offset?}`
- `inserts.import {id}` (only IDs in the current search results)
- `inserts.inspect {}` (no credentials or credential-bearing API URLs)
- `inserts.query`, `inserts.search`, `inserts.source.giphy`, `inserts.settings`,
  `inserts.import.<id>`, `inserts.previous`, `inserts.next` widget IDs.

GIPHY API documentation: https://developers.giphy.com/docs/api/
Downloaded user media and QA screenshots stay outside version control; no GIPHY assets are
bundled in the distribution.

Validation (2026-10-24): real GIPHY search returned 24 results and the next page; disabling the
source rejected search. An original GIF was downloaded, imported as animated media, opened in Source,
placed on a new timeline track and rendered at a later frame. Undo reversed placement and import.
Settings, results and timeline were visually inspected. Native URL/response validation, UI invalid
input/thumbnail tests, 22 Jarvis regressions, docking and settings tests passed; scoped Clippy,
formatting and diff checks passed. Installed in `~/Applications/FilmCraft.app` and reopened the user's
saved project with Inserts visible. API key was configured in Keychain, not committed.

### Automatic timeline placement

Settings → Inserts → Add to timeline after import is on by default and saved in
`inserts.addToTimeline`. With it off, imports remain in the Project panel only.
With it on, Import remembers the active sequence and playhead before downloading,
then places the complete GIF on an unlocked, enabled video track above any overlapping
footage. It creates a track when needed. Placement (including a new track) is one Undo
step; a second Undo removes the project import. If there is no active sequence, the
GIF imports into Project only; if the sequence changes while downloading, placement
is skipped with a message. No existing clips are overwritten.

Validated the default-on setting and both import modes against a real GIPHY GIF,
including moving the playhead during download, placement at the captured frame, and Undo.
The synthetic placement test covers new-track creation and restoring the original project.
