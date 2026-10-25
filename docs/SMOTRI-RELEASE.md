# Smotri release checklist

- Publish only vetted source files; never include plan/, target/, server configuration, cookies, keys, or user projects.
- Preserve FilmCraft authorship, LICENSE-MIT, LICENSE-APACHE and NOTICE.
- Build macOS Apple Silicon and Intel, Windows x64 and ARM64, Linux x64 and ARM64 on matching native runners.
- Platform support in source is not a verified installer. Test launch, import, export, audio and bundled Downloader helpers on each platform before promoting artifacts to the website.
- Each public bundle needs third-party license texts and complete corresponding sources for GPL/LGPL components; packaging must not silently omit helper programs.
- macOS public releases should be signed and notarized; unsigned developer builds must be explicitly labelled.
- API keys are entered by the user after installation. Keychain service IDs and historical project identifiers intentionally stay compatible.

## Initial fork changes

Smotri eyes logo and branding; AKT.MP4 links; Jarvis voice control and editing commands; GIPHY inserts; Downloader; dockable silence and duplicate removal; bot-based website sign-in and download gates. Premiere preset adaptation is not yet validated.

## Preview publication

`smotri-v*` tags run `.github/workflows/release.yml`. After all six native jobs and
WASM succeed, a GitHub prerelease receives versioned ZIPs with checksums and
`BUILD.json` identifying the exact source commit. These preview archives deliberately
exclude Downloader helpers and say so in the archive, README and release notes.
They must not be presented as complete installers or silently replace a full local app.
macOS previews are ad-hoc signed application bundles; Windows uses the static CRT.

The complete downloader package remains a separate gate: the current standalone
yt-dlp archive's own source tarball does not include all bundled dependencies' sources,
and video normalization currently uses macOS VideoToolbox. Resolve both before a full
cross-platform release. Keep existing site catalogs, credentials and download gates intact.
