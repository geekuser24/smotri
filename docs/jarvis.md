# Jarvis voice control (desktop)

Click **Jarvis** in the top-right header (replacing the Discord shortcut) to toggle voice
control. Configure the key in **Settings → Jarvis**, alongside Appearance and Audio Hardware.
The header button only toggles voice mode; it has no settings popup. If no key is configured,
it displays a hint directing you to Settings.
The existing Discord Help menu link remains available.

On macOS the ElevenLabs API key is stored in Keychain under `Smotri.Jarvis.ElevenLabs` (with automatic fallback to the old FilmCraft entry),
account `api-key`. Enter a replacement in Settings → Jarvis and click OK to save it;
Cancel discards the draft. The page shows whether a key is already configured without
revealing it. On other desktop platforms it is session-only; `ELEVENLABS_API_KEY` is
also accepted. Keys are never serialized into projects, preferences or control-channel
inspection. Voice mode is off at every launch. Stopping closes the socket, cancels the
microphone worker and discards pending transcripts. A failed connection stops the mode;
reconnection requires another click.

While enabled, microphone audio is streamed to ElevenLabs using the authenticated
[Scribe v2 Realtime WebSocket API](https://elevenlabs.io/docs/api-reference/speech-to-text/v-1-speech-to-text-realtime),
`scribe_v2_realtime`, mono 16-bit PCM at the device's supported rate. Normal ElevenLabs
usage charges and data handling apply. Server VAD commits at 0.5 seconds of silence;
language is auto-detected with Russian and English as secondary languages, independently
of the interface language. Partial, edited and timestamp transcripts never execute commands.
No recordings or full conversation transcripts are saved locally.

Say **Джарвис** or **Jarvis** followed by one command. A standalone wake word arms only the
next final segment for four seconds. Supported wake variants include джервис / жарвис.
Commands accept polite fillers, common synonyms and one transcription typo in a long word;
ambiguous phrases, negatives and compound commands are ignored. Final segments delayed
more than three seconds in the UI queue are discarded.

The Settings → Jarvis page contains a searchable command reference (`jarvis.commandSearch`).
Say «Джарвис, команды» / “Jarvis, voice commands” to open it; «закрой диалог» / “close dialog”
discards and closes Preferences or a clip settings dialog. Other dialogs use their own Cancel button.

118 curated actions cover editing, selection, transport, markers, tools, panels, workspaces,
clip settings, tracks, saving and export. Examples (always preceded by the wake word):

- «выдели клип под курсором», «следующий клип», «выдели все», «сними выделение»;
  “select clip at playhead”, “select next clip”, “select all”, “deselect all”.
- «разрежь», «разрежь все дорожки», «скопируй», «вырежи», «вставь со сдвигом»,
  «удали со сдвигом», «отмени»; “split”, “split all tracks”, “copy”, “cut”, “paste insert”,
  “ripple delete”, “undo”. Cut means clipboard Cut; split means Add Edit.
- «в начало», «в конец», «следующая склейка», «следующий кадр», «приблизь»;
  “go to start”, “go to end”, “next edit”, “next frame”, “zoom in”.
- «поставь вход», «поставь выход», «отметь клип», «поставь маркер», «следующий маркер»;
  “mark in”, “mark out”, “mark clip”, “add marker”, “next marker”.
- «добавь переход», «сгруппируй», «настрой скорость», «громче», «выбери лезвие»;
  “add video transition”, “group”, “speed duration”, “louder”, “razor tool”.
- «сохрани проект», «импорт медиа», «экспорт видео», «экспорт кадра»;
  “save project”, “import media”, “export video”, “export frame”.
- «играй», «пауза», «выключись»; “play”, “pause”, “stop listening”.

Parameterized commands (integer values, no inferred units):

- «вперед на 10 кадров» / “forward 10 frames”, «назад пять кадров» / “back five frames”
  (1–10,000 frames).
- «перейди на 12 секунду» / “go to 12 seconds” (absolute position, 0–86,400 seconds).
- «скорость 50 процентов» / “speed 50 percent” (1–1,000%; Scribe's `50%` spelling also works).
- «громче на 3 дб» / “louder 3 db”, «тише на три децибела» / “quieter three db” (1–24 dB).
- «заблокируй дорожку видео 1» / “lock track video one”; «разблокируй дорожку видео 1» /
  “unlock track video one”; «заглуши дорожку аудио 1» / “mute track audio one”;
  «включи дорожку аудио 1» / “unmute track audio one”. Tracks must already exist (1–99).

Both digits and spoken zero–ten, twenty, fifty, hundred are supported. Other spelled-out numbers,
decimals and dictated timecodes are not yet supported. Parameter commands never use fuzzy matching.

The catalog also exposes 444 registered menu/shortcut actions by their existing English and Russian
labels (Russian falls back to English where the editor lacks a translation), independent of the
chosen interface language. Use “command” / «команда» followed by the full menu path when names collide,
e.g. “Jarvis, command Sequence Add Edit”. Exact matches precede typo matching; ambiguous registry
names are rejected. Internal APIs requiring object ids are not exposed this way.

**Scope:** this provides voice command dispatch, not arbitrary field dictation or mouse replacement.
Commands which normally open a dialog still open it: file selection, arbitrary text entry and
export settings require manual input. Supported effect parameters are described below. Voice edits are blocked while the covered editor dialogs are
open. A disabled command reports the normal editor error; recognition is not reported as successful
execution. Coverage of a registry name does not prove the underlying alpha editor feature works in
every project. Real microphone accuracy still depends on the speaker, noise and network.

Edits dispatch through the editor's normal command path and preserve existing selection,
track targeting, command enablement and undo/redo behavior. This is a bounded command grammar, not a general natural-language
assistant. Background speech containing the wake word can still activate it; use headphones
when editing recordings containing these phrases.

Automation: `jarvis.toggle` / `app.settings.jarvis` take `{}` (`jarvis.settings` is an alias). `ui.inspect.jarvis` reports
`active`, `ready`, `feedback` (never the API key). Widget ids: `header.jarvis`,
`settings.category.jarvis`, `jarvis.apiKey`, `settings.ok`, `settings.cancel`.

Implementation: `crates/ui-egui/src/jarvis.rs` owns recognition grammar and UI; the native
adapter `apps/filmcraft/src/jarvis.rs` owns TLS, Keychain and microphone capture.
Audio and event queues, transcript lengths and WebSocket frames are bounded. Native
worker/capture panics become errors; credentials and raw service responses are not logged.

## Local verification (2026-10-24)

- `cargo check -p filmcraft`, scoped Clippy with `-D warnings`, formatting and diff checks passed.
- The original Jarvis tests covered bilingual aliases, negation/ambiguity,
  expired wake windows, disabled/stale events, malformed UTF-8-safe text slices, and real
  selected-clip deletion followed by Undo restoring the complete project.
- Live Scribe session transcribed synthetic RU and EN audio as “Jarvis, удали со сдвигом.”
  and “Jarvis, copy.” after the user's network connection was changed. The initial network
  received ElevenLabs' regional restriction redirect.
- Installed macOS app: control-channel verification observed off → connecting → ready → off,
  with native microphone capture running, plus a rendered settings screenshot. RU and EN
  settings were inspected in the native UI. No live spoken destructive edit was performed.
- Full workspace CI was not run; no commit or push was made.

## Expanded command verification (2026-10-24)

The expanded tests enumerate every curated alias and every unique registry phrase in RU/EN,
compare 38 voice-dispatched workflows with the normal menu command (project, selection/playhead and
UI state), check parameter edits plus complete Undo restoration, reject invalid numbers/units,
and verify that modal dialogs and disabled commands cannot edit the timeline.
The optional `FILMCRAFT_JARVIS_SPEECH_FIXTURES` test consumes local JSON rows of
`{spoken, transcript, expected}` captured from Scribe; it never stores credentials.
The local live run used 25 synthetic speech samples, including negative commands in both languages.
This is a repeatable transport/grammar check, not a claim that every command was spoken by the user
through the physical microphone. Generated speech and service transcripts stay under ignored `target/`.

Expanded verification outcome: all 15 focused tests passed, including 1,474 registry phrases,
38 workflow comparisons, parameter edits/Undo and all 25 live Scribe transcripts (including both
negative commands, which did not execute). All 19 localization tests and scoped Clippy passed; full workspace CI was not run.

## Contextual editing and batch transcription

The slot grammar also supports these RU / EN examples (each preceded by Jarvis):

- «выдели клип» / “select clip”: clip at the playhead; selected track preferred, otherwise
  the highest video track with a clip there, or audio for audio-only sequences.
- «выдели второй клип на видео 2» / “select second clip on video 2”. Clip order is chronological,
  numbers start at 1. Use “audio” / «аудио» for audio tracks. Names: «выдели клип по имени Sunset».
  Ambiguous names fail; specifying a track avoids guessing.
- «перейди к следующему клипу» / “go to next clip”; previous / «предыдущему».
- «сдвинь таймлайн влево на 10 кадров» / “move playhead left by 10 frames”. This moves the playhead.
- «добавь черно белый эффект» / “add black and white”. Effect registry names and available Russian
  translations also work. Selection is preferred; with none selected, the clip at the playhead is used.
- «добавь ударный шейк» / “add hit shake”: a 280 ms decaying hit at the playhead. «сделай шейк как в EDIT» /
  “add EDIT shake” adds brief brightness flicker and ripple. «добавь тряску камеры» /
  “add handheld shake” applies layered smooth noise for the whole clip. A seed is stored per application.
- «уменьши scale на 20 процентов» / “decrease scale by 20 percent”: subtract 20 percentage points from the existing scale.
  «понизь яркость на 20 процентов» reduces brightness by 20 points; opacity also uses points.
- «смени метод наложения на screen» / “set blend mode to screen”.
- «выбери эффект camera shake» / “select effect camera shake”.
- «установи параметр frequency на 2 в эффекте camera shake» /
  “set parameter frequency to 2 in effect camera shake”. Omit the effect to use the selected effect.
  Numeric, angle, choice and boolean parameters are supported. Use “minus” / «минус» for signs,
  “point” / «точка» for spoken decimals. Points, colours, curves, paths and arbitrary text still use UI.

«расшифруй клип» / “transcribe clip” renders the selected clip's linked audio; «расшифруй таймлайн» /
“transcribe timeline” renders the sequence audio mix, including effects and routing, to mono 16 kHz.
A separate background request uses ElevenLabs **scribe_v2**, word timestamps and automatic language
recognition. It does not record the microphone. The configured key is read from Keychain on macOS.
Two hours is the maximum per analysis; cancellation is checked while rendering and after the HTTP
request (a pending upload may take up to its ten-minute timeout to finish).

A dated WAV and JSON are saved in `Transcripts` beside the project (application data for unsaved
projects). JSON includes timeline-relative millisecond word times and the exact tick transcript.
The WAV is imported into the project with its attached transcript and a dated audio-only sequence.
Open that sequence and the Text → Transcript tab to read and navigate its words. The working
sequence stays open after analysis. Original files are never modified.
The archive records the timeline **at analysis time**, not a live transcript that follows later edits.

«удали паузы учитывая все слои» / “remove pauses all layers” only removes sustained quiet gaps
outside recognized speech; music or sound effects keep a gap. «удали паузы везде только голос» /
“remove pauses everywhere voice only” ignores non-speech outside the speech boundary neighbourhood.
Use «удали паузы в голосе в клипе» / “remove pauses in clip voice only” to restrict the time span to
one selected clip. Speech timestamps are protected by 100 ms pre-roll and 160 ms release; waveform
activity extends protected regions. Gaps shorter than 400 ms are kept; an additional 40 ms is retained
at gap edges, cuts snap inward to frames. New audio edges receive independent 5 ms gain fades.

Pause removal ripples **all tracks within that time span**, keeping linked video and audio aligned;
locked tracks cause refusal. It is one Undo step, including transcript import. Each job snapshots the
project and refuses to apply if it changed during analysis; the dated archive remains available.
This is conservative energy-based editing, not source separation or a guarantee that every quiet
syllable was recognized. Listen to the result and use «отменить» / “undo” as needed.


## Spoken clarifications

«Джарвис, добавь шейк» / “Jarvis, add shake” (also «добавь здесь тряску») asks which preset:
1. Hit shake / ударный.
2. EDIT with flash / EDIT со вспышкой.
3. Handheld / живая камера с рук.

The question appears in the status line and is synthesized with the same ElevenLabs key using
`eleven_flash_v2_5` and the default George voice. Only the short question is spoken; numbered
choices remain text. The microphone stays connected during synthesis and playback. Reply immediately
using a number or preset name,
with or without the wake word. The question stays open until answered, cancelled or replaced;
there is no 30-second choice timeout. Replies such as «давай второй», «E1», «со вспышкой»,
«с рук» and “choose option two” are accepted. «Повтори вопрос» / “repeat question” speaks it again. «отмена» / “cancel” abandons the choice. An explicitly
named preset still applies immediately. A new addressed command replaces the pending question.

Questions bind to the project, sequence, selection and playhead at the time they were asked.
Changing any of those requires a new request; an answer never silently targets a different clip.
Turning Jarvis off cancels the pending question and playback. TTS failures leave the textual question
available and keep listening; HTTP failures expose only an error code, never the key.
UI/control commands: `jarvis.askShake {target?}`, `jarvis.choose {choice:0|1|2}` (control indices are
zero-based; spoken numbers start at one). Inspection exposes `speaking` and `awaitingChoice`.


### Contextual and dialogue verification (2026-10-24)

18 Jarvis tests and five engine voice-edit/transcription tests passed, including 36
real Scribe transcripts of synthetic RU/EN speech. Release build, scoped Clippy
with warnings denied, formatting and diff checks passed. Full workspace CI was not run.
The installed macOS build was exercised on an isolated demo project: spoken clarification,
listener restart, EDIT choice and Undo, effect/blend/scale edits and Undo. A real batch
Scribe request archived/imported a transcript and removed a synthetic silent gap
(131 frames became 88), with boundary gain fades. The question and Settings UI were
visually inspected. Spoken answers are parser-tested; the user's own microphone
pronunciation and subjective audio quality have not been evaluated.


### Ripple delete with overlapping synced layers

Jarvis uses the normal Ripple Delete first. If an unselected sync-locked layer
blocks it, it closes only the selected clips' tracks (or the selected gap's track).
Other tracks remain unchanged and their sync-lock settings are preserved. Linked
clips included in the selection move together. This fallback deliberately changes
timing relative to untouched layers; Undo restores the whole operation in one step.
It does not bypass locked target tracks. Other editor ripple operations retain their
existing sync-conflict checks.

Regression checks: three voice-edit engine tests and all 19 Jarvis UI/grammar tests
passed, including a selected linked clip and a selected gap blocked by V2/A2,
preserved sync-lock flags and full-project Undo. The existing strict editor
ripple-delete regression also passes. Scoped release Clippy, formatting and diff
checks passed.


## Rhythmic auto edit

Select timeline video clips and music/audio clips, then say “Jarvis, edit”,
“Jarvis, make an edit”, «Джарвис, сделай эдит» or «Джарвис, сделай монтаж под музыку».
`voice.autoEdit {}` performs local background audio analysis; it does not send music
to ElevenLabs. Realtime Scribe still recognises the spoken command.

The selected audible audio is rendered to a stereo 48 kHz WAV in `Jarvis Edits`
beside the saved project (or the application data folder). Spectral-flux attacks
use a local threshold; the existing tempo/beat tracker provides a rhythm grid,
with strong off-beat fills allowed. Weak/absent rhythmic content produces an error.
The resulting dated `Jarvis Edit` sequence opens automatically. Its continuous
music covers the selected audio range, and video alternates between source items.
Short source clips are reused from varying offsets within their selected trims.
When only one source item is available, repeated shots of that item are unavoidable.

Beat accents add separate Transform zoom, decaying camera shake, occasional
brightness flashes and glow. Existing source framing/effects are copied; the
original sequence stays intact. One Undo removes the generated project additions.
The rendered WAV remains on disk. Cancellation is available through Jobs; changing
the project, active sequence or selection during analysis prevents application.

Initial limits: 2–180 seconds of selected audio, at most 100 video and 100 audio
clips, forward clips without frame holds or animated time remapping, maximum 1200
output shots. Gaps in the selected audio range are preserved. Selection is literal:
linked source audio included in the selection participates in the mix, so deselect
it if only the music should be heard. Cut timing follows attacks/tempo, not semantic
understanding of the footage or lyrics. Complex effect dialogs remain editable by hand.


Auto-edit verification: two new engine tests passed (quiet/loud drum attacks,
silence/invalid samples, source bounds, alternation, visible rendered output,
original sequence preservation, stale selection, missing input and single Undo),
as did all 19 Jarvis tests and scoped Clippy. The installed macOS app assembled
32 shots from three selected sources and a 16-second generated drum track, then
passed Undo/Redo and H.264 export with stereo 48 kHz AAC audio. Its timeline and
program monitor were visually inspected. Tests use synthetic material; arbitrary
real-world music and aesthetic quality still require editorial judgement.

## Conversational variants and atomic chains (2026-10-24)

ElevenLabs remains the sole speech provider; no language-model account or API key is needed.
A conservative rewriting layer accepts polite introductions, infinitives, some reordered clip
references, selection pronouns, spelled-out integer percentages (including “twenty five” /
«двадцать пять»), and a unique one-letter typo in supported long verbs. Numeric values and clip
numbers are never fuzzy guessed.

Examples:
- «Хочу на втором клипе сделать яркость поменьше на 20 процентов».
- «Выдели второй клип, потом уменьши его масштаб на 20 процентов, после чего разрежь его пополам».
- “Select second clip then reduce scale by twenty five percent then split it in half”.

Explicit connectors `потом / затем / после чего / после этого / then / after that` form a
bounded sequence (at most 16 engine steps). All clauses must parse before any action starts.
Supported chain actions are clip/effect selection, effects, numeric parameters, explicit shake
presets, midpoint split, clear/ripple delete, playhead frame steps, next/previous clip.
A target named by an effect step becomes the selection for subsequent pronouns. The midpoint split
cuts one clip on a frame boundary and leaves its first half selected. For an odd frame count the
second half has the extra frame. `voice.plan` groups edits into one Undo; a failed step rolls back
project and editor selection. Commands with external side effects, asynchronous analysis, Undo,
modal dialogs, and unresolved shake choices cannot be mixed into these atomic chains.

This is still a deterministic grammar, not unrestricted semantic reasoning. “A little darker”
without a number and “align it to the third beat” are not interpreted by this layer. Unknown clauses,
negations, incomplete chains and ambiguous targets do not execute a recognized prefix. Use a
complete final utterance; partial streaming transcripts never trigger edits.

Validation: all 21 Jarvis UI/grammar tests and four engine voice-edit tests passed. Scoped release
Clippy, formatting and diff checks passed. The native release app was driven against a separate demo:
select second clip → reduce scale → split halfway, one Undo, and a failing final step that restored
all original timeline tracks. Settings examples were visually checked. The release was installed in
`~/Applications/FilmCraft.app`; microphone speech was not re-recorded for this parser-only update.

### Dialogue timeout fix (2026-10-24)

Removed the 30-second choice timer and repeated posting of unchanged feedback. A queued choice
reply may survive an editor stall; application still validates the captured project, selection,
sequence and playhead. Ordinary stale commands retain their three-second expiry. Added the
«выбери шейк» entry phrase and conversational/E1/E2/E3 replies. Regression tests cover simulated
TTS completion, microphone readiness, delayed unaddressed answers, one application, Undo and
changed-context rejection. All 22 Jarvis tests plus the engine chain test, scoped release Clippy,
formatting and diff checks passed. Real ElevenLabs testing reached microphone-ready with the
question pending; the extended live test was interrupted by a new spoken command, correctly
replacing the question. No claim of a clean end-to-end live delayed-answer test is made.

### Hide or enable Jarvis

Settings → Jarvis → Enable Jarvis controls `jarvis.enabled` (on by default for
existing and new preferences). Apply with OK; Cancel discards the change. Disabling
stops microphone/reply activity and pending clarification, hides the header button,
and blocks `jarvis.toggle`. Enabling restores the button without starting listening.
The credential stays in Keychain.

### Adjustment layers and short shake accents

“Создай/создать/добавь корректирующий слой” and “create/add adjustment layer”
create and place a real adjustment source at the playhead. It ends at the selected
video clip's end (or the clip under the playhead), with a five-second fallback.
Placement reuses an unlocked, enabled empty span above all overlapping video clips
or creates a new upper video track. Existing clips and audio are not overwritten.

Hit/EDIT presets now default to a short adjustment overlay (about 280 ms, snapped to
sequence frames and bounded by the target clip). Their envelope settles in 260 ms.
“На сам клип” / “directly on clip” applies the accent to the clip instead. Handheld
remains a full-clip effect; “корректирующим слоем” / “as adjustment layer” instead
creates an overlay covering the selected video span. These are effects, never
transitions; creation, placement and effects are one Undo operation.

### Numeric edits, duration and order

Spoken scale increases/decreases use percentage points: 120% minus 20 becomes
100%. “Сделай 100%” sets scale to 100%; “сделай 10%” sets it to 10%. Unqualified
“сделай ниже/выше на 20” refers to scale; name brightness or opacity to change
those instead. The low-level `voice.parameter` API still offers an explicit
`percent` operation for multiplicative changes.

“Разрежь второй клип по центру” splits at its midpoint on the frame grid and
includes linked audio. “Сделай две секунды второго клипа” trims its end to two
seconds (nearest frame), including linked audio; it leaves subsequent clip positions
unchanged. An impossible extension fails atomically instead of silently using a
different duration.

“Передвинь второй клип на первое место”, “третий клип сделай вторым”, and
“move third clip to second position” reorder clips on the targeted track. Ordinals
refer to chronological order. Linked audio follows; existing gap durations are
retained. Locked partners or an audio overwrite cause the whole operation to be
rolled back. “Добавь маркер под названием удар” and “поставь маркер на 10 секунд”
add named or timed timeline markers; next/previous/delete marker commands remain
available. Each edit can be undone in one step.

### Short clarification replies and barge-in

Shake choices remain visible as numbered text. ElevenLabs speaks only “Какую тряску добавить?” /
“Which shake would you like?”. Microphone capture, Scribe transport and event polling continue
during synthesis and playback. A recognized choice cancels playback without reconnecting Scribe.
Exact normalized question echoes are removed, including a question followed by a choice in one
transcript; the question contains no wake word or valid choice. This is transcript-level echo
filtering, not acoustic echo cancellation: loud speakers can still mask simultaneous user speech.
Physical speaker/microphone overlap needs confirmation on the user's audio setup.

## Manual cleanup of selected clips

Timeline clip context menu → **Remove Silence…** / **Remove Duplicate Takes…**, in a
separate group after Replace With Clip From Source Monitor. These tools do not require voice mode.

Silence analysis is local by default. Configure the peak threshold (-80 to -10 dBFS), minimum
removed duration and padding (at least 40 ms). Automatic threshold estimation uses the low/high
10 ms peak distributions; it is a signal estimate, not an AI model. Optional Scribe v2 protection
marks recognized words and extra margins as uncuttable. Analysis shows the measured levels and
proposed cut count; OK applies all cuts as one undoable edit. Silence below a chosen threshold
cannot reliably be distinguished from extremely quiet speech without reviewing/listening or
using speech protection.

Duplicate review uses Scribe v2, segments phrases at sentence endings or pauses, and groups
matching word sequences (three words minimum; longer phrases allow one changed word). This is
conservative repeated-take detection, not semantic paraphrase detection. Phrase bounds expand
100–350 ms toward quiet audio windows, snap to frames, and may not swallow neighboring transcript
words. Unsafe bounds are omitted. Each group lists its takes: Preview plays the corresponding
current timeline interval while the editor remains available; Keep ripple-deletes the other
takes immediately, advances to the next group, and creates one Undo step. Remaining review times
are remapped after each choice. No take is automatically selected for deletion.

Selected clips always include linked partners, even with Linked Selection off. Only affected tracks
ripple; unrelated tracks/captions are untouched. Cuts overlapping unselected clips on affected tracks
are rejected. 5 ms gain envelopes soften audio joins without replacing existing volume automation.
Analysis is tied to an exact project/sequence snapshot; outside edits or Undo invalidate the review.
Reset Analysis cancels pending analysis but preserves already accepted edits (Undo remains available).

Commands: `audioCleanup.analyze`, `.inspect`, `.apply`, `.preview`, `.cancel`;
UI commands: `audioCleanup.openSilence`, `audioCleanup.openDuplicates`. Both open ordinary dockable panels (Window → Silence Removal / Duplicate Takes), not modal dialogs. Analysis uses the selection when Analyze is pressed. Timeline editing remains available; project edits invalidate old results.
The six engine regression tests cover waveform margins/quiet speech, duplicate grouping, local
analysis without a transcriber, linked A/V, untouched unrelated tracks, remapped review times,
Undo and invalid inputs. Live API and UI validation is recorded separately in local QA logs.

## SoundPad and docked workflow commands

See [SoundPad](soundpad.md) for the new Russian/English search, numbered preview/import and workflow phrases. Voice previews never import automatically. Stop is accepted during SoundPad playback; other voice editing is held until playback and its echo tail end. Vertical-video analysis, selection, creation and batch export use the same commands and current panel options as manual controls. Duplicate take choices specify both take and group numbers. Search, preview and file-picker steps are separate commands; asynchronous workflows are not combined into an editing command chain.


## Shaker and manual Beat Edit

Effects → Video Effects → Transform → **Shaker** applies the same effect used by
Jarvis shake commands and Beat Edit accents. Presets are **Hit**, **EDIT Flash**,
and **Handheld**. Amount, rotation, zoom, frequency, complexity, motion blur,
flash/flicker, ripple and random seed remain editable. Mirror Edges avoids exposed
black borders. Changing a preset resets its tuning values/curves, while preserving
Hit Mode, Hit Trigger keys, strength, seed and edge mode.

Without **Hit Mode**, motion continues across the clip. With Hit Mode on, enable
animation for **Hit Trigger** using the stopwatch, then add a diamond key at each
wanted hit. Each key starts an independent decaying shake (280 ms by default);
its value controls that hit's strength. No trigger keys means no shake. Duration
uses effect/media time, so retiming the clip also retimes the hits. Rendering is
deterministic in preview and export. Old Camera Shake effects remain supported.

«Джарвис, поставь ключ удара» / “Jarvis, add shaker hit” enables Hit Mode and adds
one trigger key to the selected clip's Shaker at the playhead. «Включи хит режим» /
“shaker hit mode on”, «пресет шейкера эдит» / “shaker preset edit” change its settings.
«Добавь эдит шейк» creates a short Shaker adjustment layer; «добавь тряску с рук»
uses continuous Shaker on the target clip. Legacy EDT recognition is retained for
older commands; the displayed preset name is EDIT.

Timeline clip right-click → **Эдит / Beat Edit**, below the silence and duplicate
commands, invokes the same `voice.autoEdit` command as Jarvis. Select video clips
and music/audio first. It creates a separate beat-cut sequence, preserves originals,
and can be undone in one step; missing selection inputs produce an actionable error.
