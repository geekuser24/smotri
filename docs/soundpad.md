# SoundPad

Window → SoundPad opens a dockable MyInstants sound browser. Search by name, listen without adding a project item, then import. Results are numbered for voice control and paginated. Stop ends preview playback; previews are capped at 60 seconds. Original MP3 files are imported in full.

Settings → SoundPad defaults to adding imported sounds at the captured playhead on an unlocked, enabled audio track with no overlap. If needed, one new audio track is created. Turn this off to import into Project only. Downloads are saved in `SoundPad` beside a saved project, or in the application data directory for an unsaved project. Import and timeline placement use normal undoable editor commands. If the project changes during download, the file is retained but no automatic edit is performed.

MyInstants API v1 works anonymously (documented limit: 10 requests/minute). Searches are throttled to one per 7 seconds; HTTP 429 is reported without automatic retry. Optional API tokens use the Authorization header and macOS Keychain (`Smotri.SoundPad.MyInstants`), never project/preferences JSON. On other desktop platforms tokens are session-only, or supplied using `MYINSTANTS_API_TOKEN`. API access and availability depend on MyInstants. User-imported sounds are not bundled or redistributed with the editor; source links stay visible.

Transport accepts only HTTPS MP3 URLs under `www.myinstants.com/media/sounds/`, follows no redirects, caps JSON at 1 MB, sound downloads at 20 MB and results at 50/page. Workers catch panics and report failure rather than leaving the UI busy. Files are written with exclusive creation and removed after failed writes. Preview uses the existing Rust decoder and audio output, without creating project items. During preview and a three-second tail, Jarvis accepts only stop commands so a sound saying a wake word cannot edit the project.

## Commands

- `soundpad.search {query, page?:1..1000}`
- `soundpad.preview {index:1-based}` or `{id:slug}`
- `soundpad.stop {}`
- `soundpad.import {index:1-based, timeline?:bool}` or `{id:slug, timeline?:bool}`
- `soundpad.inspect {}`

## Jarvis examples

- «Открой саундпад» / “open soundpad”.
- «Найди звук удар» / “search sounds impact”.
- «Послушай звук второй» / “preview sound two”.
- «Добавь звук 2 на таймлайн» / “import sound 2 into project”.
- «Следующие звуки» / “next sounds”.
- «Останови звук» / “stop sound”.

New workflow grammar also covers opening Inserts, Downloader, Silence Removal, Duplicate Takes and Vertical Videos; GIF search/import by number; downloader probe/download/audio-only/quality/cancel/import; silence analysis/apply; duplicate analysis and explicit take/group preview/keep; shorts source selection, 30/60/90-second analysis, numbered preview/selection, select-all, creation, framing/caption options, 1–5 variants, folder picker and batch export. All model-assisted analysis still uses configured providers and their billing. Unknown or negated editing clauses are rejected; this is a bounded bilingual grammar, not unrestricted language understanding.

Examples: «Проанализируй шортсы на 60 секунд», «Выбери шортс второй», «Покажи шортс 1», «Шортсы с размытым фоном», «Субтитры шортсов снизу», «Создай шортсы», «Выбери папку для шортсов», «Экспортируй шортсы», «Прослушай дубль 2 в группе 1», «Оставь дубль 2 в группе 1».

## Verification (macOS ARM, 2026-10-25)

234 UI/library tests passed, including voice routing, bounded hostile inputs, free audio track allocation and Undo; 30 settings tests passed serially (the hardware decoder flag is process-global); two native transport tests passed, including a real anonymous API search and MP3 decode. Clippy on affected crates/all targets, formatting, layer checks and asset attribution checks passed. The release app was driven over its control channel: search, audio preview, timeline import, unchanged video tracks, Undo, project-only import and settings screenshot. Microphone recognition itself was not re-tested in this pass; voice-command dispatch was tested with transcript input. No claim of unrestricted natural-language understanding or new cross-platform release binaries.
