# Downloader panel

Window → Downloader is a dockable native FilmCraft panel. Paste one HTTP(S) media
link and press Download. The program checks available formats, then offers video
resolutions or Audio only (WAV). Settings → Downloader can skip this choice and use
a fixed default quality (a maximum height; lower source resolutions are not upscaled).

Downloads run in a worker with progress and cancellation. Separate video/audio
streams are merged and prepared as H.264/AAC MP4 or 48 kHz stereo PCM WAV. On macOS,
preparation uses VideoToolbox through the standalone FFmpeg process. FilmCraft's
existing media import, timeline, render and export engines remain unchanged.

Settings control the folder, Project import, timeline placement and cookies. The
default folder is `Downloads` beside a saved project; unsaved projects use a chosen
folder or FilmCraft's application-data Downloads directory. The destination,
project/sequence and playhead are captured at download start. A changed project
prevents automatic import; the completed file remains available for manual import.
Timeline placement creates new video/audio tracks without overwriting existing clips,
and uses the editor's normal linked-clip placement and Undo.

Choose Chrome, Edge, Firefox, Safari or Brave to read the current browser session on
each request, or select a Netscape cookies file. No Google password is collected.
The sign-in button opens YouTube login in the system browser; select that same
browser in settings afterward. macOS can require Keychain or browser-data permission.
Cookies are never sent to the reference Telegram server, embedded into projects or
included in application packages. Websites can expire cookies, block requests, or
require an updated extractor. There is no guarantee for every URL. DRM, playlists
and ongoing live streams are not supported. Age-restricted media can work when the
site permits access with the selected authenticated session.

## Standalone macOS packaging

The user explicitly authorized an exception to the repository's pure-Rust/FFmpeg
restriction for this downloader on 2026-10-24. Tools run as separate subprocesses,
never linked into the Rust editor. No third-party codec implementation was copied
into FilmCraft's source. The existing Telegram bot was inspected read-only for its
format selection/cookies workflow; its code, credentials and server cookies were not
copied into the application, and it is not a runtime dependency.

`packaging/macos/prepare-downloader.py` is a build-machine step, currently pinned for
Apple Silicon (macOS 12 or later, required by the bundled Deno runtime). It downloads and verifies yt-dlp 2026.10.21 and Deno 2.9.8, and builds
unmodified FFmpeg 8.0.1 without GPL/nonfree libraries against macOS system frameworks,
with static dav1d 1.5.4 (BSD-2-Clause) for software AV1 decoding. Meson, Ninja and
pkg-config are build-machine requirements only; they are not runtime dependencies.
`bundle-downloader.py` checks hashes and copies all tools, notices and source archives
inside `FilmCraft.app/Contents/Resources/DownloaderTools` before signing. `package.sh`
requires a tool bundle matching its target architecture; it refuses to ship a partial
or mismatched bundle. No Python, Homebrew, server connection or first-run component
download is required on the end user's machine. Runtime needs network only for the
media site, normal site challenges and any explicitly selected authentication.

The editor's entitlements remain separate from helper entitlements (Deno JIT and
PyInstaller library loading). Updating bundled tools is a new app build, preserving
code signatures, not a self-update that mutates a signed application.

Licensing: the upstream yt-dlp standalone binary includes GPLv3+ dependencies even
though yt-dlp itself is Unlicense; FFmpeg here is LGPL-2.1-or-later; Deno is MIT.
The bundle includes notices and unmodified yt-dlp/FFmpeg source archives. Before
public distribution, collect corresponding source for the **complete standalone
yt-dlp dependency set** too, and publish it alongside the installer; the yt-dlp source
archive alone does not cover all its bundled dependencies. Public macOS releases also
need the existing Developer ID signing/notarization flow. This task installs a local
ad-hoc signed build and does not publish a release. See upstream documentation:
https://github.com/yt-dlp/yt-dlp#licensing and https://ffmpeg.org/legal.html.

## Automation and validation

Commands: `downloader.probe {url}`, `downloader.download {quality}`, `downloader.cancel`,
`downloader.import {timeline?:bool}`, `downloader.reveal`, `downloader.inspect`.
`app.settings.downloader` and `window.panel.Downloader` open the corresponding UI.
Widget IDs: `downloader.url`, `.probe`, `.quality`, `.start`, `.cancel`, `.settings`,
`.import`, `.timeline`, `.reveal`, `.pickFolder`, `.pickCookies`, `.signIn`.
Preferences use the `downloader.*` namespace and the normal OK/Cancel persistence.

No shell is used. Arguments are separate, input URLs are HTTP(S), inherited yt-dlp
configuration is disabled, metadata/process output and channels are bounded. A worker
panic reports an error. Cancellation/timeout stops the process group, including merge
children. Failed/incomplete work directories are removed; completed downloads never
overwrite an existing asset. Errors do not echo URLs, cookie values or raw subprocess
output. One operation runs at a time; the current UI retains the latest completed file.

Automatic quality starts the native download directly, without waiting for a render
frame to consume a probe response. This lets the transfer and preparation proceed
when the display becomes inactive. Completion and project import are polled in the
application logic pass, including when an occluded window skips rendering.

Local installed-app validation (2026-10-24, Apple Silicon): launched with PATH limited
to /usr/bin:/bin, downloaded a generated HTTP video, verified H.264/AAC normalization,
captured-playhead placement on new linked tracks, Undo, automatic WAV mode, and
manual audio placement. FilmCraft itself exported the imported sequence as MP4 and
WAV; the exported audio contained nonzero samples. Native tests cover cancellation
and bounded subprocess output; UI tests cover hostile URLs/quality values and direct
automatic-mode startup. Browser-authenticated and age-restricted links were not
live-tested with a user account.

The installed app also downloaded the public YouTube clip “Me at the zoo”, including
AV1 input decoded by bundled dav1d, and produced H.264/AAC MP4. Cancellation completed
cleanly. The UI completion/import path remained responsive without requiring
continuous window rendering.
