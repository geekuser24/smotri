#!/usr/bin/env python3
"""Copy a verified tool bundle into an app before code signing (build-time only)."""
import hashlib, json, shutil, sys
from pathlib import Path
source, app = map(Path, sys.argv[1:3])
arch = sys.argv[3] if len(sys.argv) > 3 else 'aarch64'
manifest = json.loads((source/'manifest.json').read_text())
expected = {'aarch64':'arm64', 'x86_64':'x86_64', 'universal':'universal'}[arch]
if manifest['architecture'] != expected:
    raise SystemExit('Downloader bundle architecture does not match the application: '+expected)
for name in ['yt-dlp','ffmpeg','ffprobe','deno']:
    if hashlib.sha256((source/name).read_bytes()).hexdigest() != manifest['sha256'][name]:
        raise SystemExit('Downloader tool failed checksum validation: '+name)
for required in ['Licenses/README.txt','Licenses/yt-dlp-THIRD_PARTY_LICENSES.txt','Sources/ffmpeg-8.0.1.tar.xz','Licenses/dav1d-COPYING','Sources/dav1d-1.5.4.tar.xz']:
    if not (source/required).is_file(): raise SystemExit('Missing bundled notice/source: '+required)
dest = app/'Contents/Resources/DownloaderTools'
if dest.exists(): shutil.rmtree(dest)
shutil.copytree(source,dest)
print('Bundled downloader tools:',dest)
