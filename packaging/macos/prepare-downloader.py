#!/usr/bin/env python3
"""Build-time only: prepare standalone downloader tools for the current Mac architecture.
No Python, Homebrew or first-run downloads are required by the resulting Smotri.app.
Usage: python3 packaging/macos/prepare-downloader.py [output-directory]
"""
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[2]
OUT = Path(sys.argv[1] if len(sys.argv) > 1 else ROOT / 'target/downloader-tools/bundle').resolve()
CACHE = ROOT / 'target/downloader-tools'
ARCH = platform.machine()
if sys.platform != 'darwin' or ARCH not in ('arm64', 'x86_64'):
    raise SystemExit('Run this build-time script on a native macOS ARM64 or Intel runner.')
CACHE.mkdir(parents=True, exist_ok=True)
OUT.mkdir(parents=True, exist_ok=True)
licenses = OUT / 'Licenses'
sources = OUT / 'Sources'
licenses.mkdir(exist_ok=True)
sources.mkdir(exist_ok=True)

def fetch(url, dest, sha=None):
    if not dest.exists() or (sha and hashlib.sha256(dest.read_bytes()).hexdigest() != sha):
        with urllib.request.urlopen(url, timeout=60) as src, dest.with_suffix(dest.suffix+'.part').open('wb') as dst:
            shutil.copyfileobj(src, dst)
        dest.with_suffix(dest.suffix+'.part').replace(dest)
    digest = hashlib.sha256(dest.read_bytes()).hexdigest()
    if sha and digest != sha:
        raise RuntimeError('Checksum mismatch: ' + dest.name)
    return digest

yt = 'https://github.com/yt-dlp/yt-dlp/releases/download/2026.10.21/yt-dlp_macos'
fetch(yt, CACHE/'yt-dlp', '7a6fe71d7d680f1fb0ac18d4949873d77db2859e90017c0f44bd94b23be4ee40')
deno_arch,deno_sha = ('aarch64','9f0bf660393da2eaff044cdc7444cb08c73f235849b61d224d825a194ce7c98e') if ARCH == 'arm64' else ('x86_64','4b2f5edd2639b7470f034238b321d7002d8dfb4726d8558f52c17afac370eb2c')
fetch('https://github.com/denoland/deno/releases/download/v2.9.8/deno-'+deno_arch+'-apple-darwin.zip', CACHE/'deno.zip', deno_sha)
with zipfile.ZipFile(CACHE/'deno.zip') as z:
    (OUT/'deno').write_bytes(z.read('deno'))
shutil.copy2(CACHE/'yt-dlp', OUT/'yt-dlp')
archive = CACHE/'ffmpeg-8.0.1.tar.xz'
fetch('https://ffmpeg.org/releases/ffmpeg-8.0.1.tar.xz', archive, '05ee0b03119b45c0bdb4df654b96802e909e0a752f72e4fe3794f487229e5a41')
# FFmpeg's build system cannot use source paths containing spaces.
work = Path(tempfile.gettempdir())/('smotri-ffmpeg-build-'+ARCH)
work.mkdir(exist_ok=True)
source = work/'ffmpeg-8.0.1'
if not source.exists():
    subprocess.run(['tar', '-xf', str(archive), '-C', str(work)], check=True)
dav_archive = CACHE/'dav1d-1.5.4.tar.xz'
fetch('https://download.videolan.org/pub/videolan/dav1d/1.5.4/dav1d-1.5.4.tar.xz', dav_archive, '686616b7c69eb88d44459391ab25cac13b6647a3b288835c5784e71c1514a5c5')
dav_source = work/'dav1d-1.5.4'
if not dav_source.exists():
    subprocess.run(['tar', '-xf', str(dav_archive), '-C', str(work)], check=True)
env = dict(os.environ, MACOSX_DEPLOYMENT_TARGET='12.0')
env['PKG_CONFIG_PATH'] = str(work/'install/lib/pkgconfig')
if not (work/'install/lib/libdav1d.a').exists():
    for dep in ['meson', 'ninja', 'pkg-config']:
        if not shutil.which(dep):
            raise SystemExit('Build machine requires '+dep+'; it is not needed by app users.')
    subprocess.run(['meson', 'setup', str(work/'dav1d-build'), str(dav_source), '--prefix='+str(work/'install'), '--libdir=lib', '--default-library=static', '--buildtype=release', '-Denable_tools=false', '-Denable_tests=false'], env=env, check=True)
    subprocess.run(['meson', 'compile', '-C', str(work/'dav1d-build')], env=env, check=True)
    subprocess.run(['meson', 'install', '-C', str(work/'dav1d-build')], env=env, check=True)
shutil.copy2(dav_archive, sources/dav_archive.name)
shutil.copy2(dav_source/'COPYING', licenses/'dav1d-COPYING')
config = ['--enable-libdav1d', '--pkg-config-flags=--static', '--prefix='+str(work/'install'), '--disable-doc', '--disable-debug', '--disable-ffplay', '--disable-shared', '--enable-static', '--disable-gpl', '--disable-nonfree', '--enable-securetransport', '--enable-videotoolbox', '--disable-encoders', '--enable-encoder=h264_videotoolbox,aac,pcm_s16le', '--disable-autodetect']
config_stamp = work/'filmcraft-ffmpeg-config.json'
if not (work/'install/bin/ffmpeg').exists() or not config_stamp.exists() or json.loads(config_stamp.read_text()) != config:
    subprocess.run(['./configure', *config], cwd=source, env=env, check=True)
    subprocess.run(['make','-j6'], cwd=source, env=env, check=True)
    subprocess.run(['make','install'], cwd=source, env=env, check=True)
    config_stamp.write_text(json.dumps(config))
for name in ['ffmpeg','ffprobe']:
    shutil.copy2(work/'install/bin'/name, OUT/name)
for name in ['yt-dlp','ffmpeg','ffprobe','deno']:
    (OUT/name).chmod(0o755)
    # No dependencies on Homebrew or developer-specific paths.
    deps=subprocess.check_output(['otool','-L',str(OUT/name)],text=True)
    deps = '\n'.join(line for line in deps.splitlines() if line.startswith('\t'))
    if '/opt/homebrew/' in deps or '/usr/local/' in deps or '/Users/' in deps:
        raise RuntimeError('Non-system runtime dependency in '+name)
shutil.copy2(archive, sources/archive.name)
for name in ['COPYING.LGPLv2.1','COPYING.LGPLv3','LICENSE.md']:
    shutil.copy2(source/name, licenses/('FFmpeg-'+name))
(licenses/'FFmpeg-build.txt').write_text('FFmpeg 8.0.1, unmodified upstream source.\n'+' '.join(config)+'\nStatic dav1d (BSD-2-Clause) and macOS system frameworks; no GPL or nonfree libraries enabled.\n')
for name,url in [
    ('yt-dlp-UNLICENSE','https://raw.githubusercontent.com/yt-dlp/yt-dlp/2026.10.21/LICENSE'),
    ('yt-dlp-THIRD_PARTY_LICENSES.txt','https://raw.githubusercontent.com/yt-dlp/yt-dlp/2026.10.21/THIRD_PARTY_LICENSES.txt'),
    ('Deno-LICENSE.md','https://raw.githubusercontent.com/denoland/deno/v2.9.8/LICENSE.md'),
    ('GPL-3.0.txt','https://www.gnu.org/licenses/gpl-3.0.txt'),
]:
    fetch(url,licenses/name)
fetch('https://github.com/yt-dlp/yt-dlp/releases/download/2026.10.21/yt-dlp.tar.gz',sources/'yt-dlp-2026.10.21.tar.gz')
(licenses/'README.txt').write_text('Smotri Downloader runs yt-dlp, FFmpeg and Deno as independent processes.\nFFmpeg 8.0.1: LGPL-2.1-or-later; corresponding unmodified source archive is in Sources.\nyt-dlp 2026.10.21: standalone macOS distribution is GPL-3.0-or-later because of bundled dependencies; see THIRD_PARTY_LICENSES.\nDeno 2.9.8: MIT, https://github.com/denoland/deno/tree/v2.9.8\nBefore public distribution, provide corresponding source for the complete yt-dlp standalone dependency set as required by its licenses; the included yt-dlp source archive alone is not that complete set.\n')
manifest={'architecture':ARCH,'versions':{'yt-dlp':'2026.10.21','ffmpeg':'8.0.1','dav1d':'1.5.4','deno':'2.9.8'},'sha256':{name:hashlib.sha256((OUT/name).read_bytes()).hexdigest() for name in ['yt-dlp','ffmpeg','ffprobe','deno']}}
(OUT/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
print('Prepared standalone bundle:',OUT)
