#!/usr/bin/env python3
"""Package native preview builds, without private data or unready downloader helpers."""
import argparse
import hashlib
import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import zipfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', required=True)
    parser.add_argument('--platform', required=True)
    parser.add_argument('--version', required=True)
    parser.add_argument('--sha', required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    binary = root / 'target' / args.target / 'release'
    stage = root / 'target' / 'preview-package' / args.platform
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)
    windows = 'windows' in args.target
    mac = 'apple-darwin' in args.target
    suffix = '.exe' if windows else ''
    if mac:
        app = stage / 'Smotri.app'
        executable = app / 'Contents' / 'MacOS' / 'Smotri'
        resources = app / 'Contents' / 'Resources'
        resources.mkdir(parents=True)
        executable.parent.mkdir(parents=True)
        shutil.copy2(root / 'assets/smotri/Smotri.icns', resources / 'Smotri.icns')
        template = (root / 'packaging/macos/Info.plist.in').read_text()
        template = template.replace('@VERSION@', args.version).replace('@SHORT_VERSION@', '0.12.0').replace('@BUILD_SHA@', args.sha)
        plistlib.loads(template.encode())
        (app / 'Contents/Info.plist').write_text(template)
    else:
        executable = stage / ('Smotri' + suffix)
    shutil.copy2(binary / ('filmcraft' + suffix), executable)
    executable.chmod(0o755)
    cli = stage / ('Smotri-cli' + suffix)
    shutil.copy2(binary / ('filmcraft-cli' + suffix), cli)
    cli.chmod(0o755)
    for name in ('LICENSE-MIT', 'LICENSE-APACHE', 'NOTICE', 'CHANGELOG-SMOTRI.md'):
        shutil.copy2(root / name, stage / name)
    manifest = {'version': args.version, 'commit': args.sha, 'target': args.target,
                'channel': 'preview', 'downloader_bundled': False,
                'sha256': {str(p.relative_to(stage)): hashlib.sha256(p.read_bytes()).hexdigest()
                           for p in (executable, cli)}}
    (stage / 'README.txt').write_text(
        'Smotri preview / Предварительная сборка\n'
        'https://smotri.cc | https://github.com/geekuser24/smotri\n\n'
        'Extract the entire archive before launching Smotri. On macOS move Smotri.app to Applications.\n'
        'Распакуйте архив целиком. На macOS перенесите Smotri.app в Программы.\n'
        'macOS: ad-hoc signature, not notarized. Windows: unsigned.\n'
        'Downloader helpers are NOT included; this preview cannot download videos.\n'
        'Компоненты загрузчика видео НЕ включены; загрузка видео в этой сборке недоступна.\n'
        'Other editor features are available; see CHANGELOG-SMOTRI.md and feature documentation.\n'
        'API keys must be entered by the user. No author credentials are included.\n', encoding='utf-8')
    if mac:
        for path in (executable, cli, app):
            subprocess.run(['codesign', '--force', '--sign', '-', '--timestamp=none', str(path)], check=True)
        subprocess.run(['codesign', '--verify', '--strict', '--deep', str(app)], check=True)
        manifest['sha256'] = {str(p.relative_to(stage)): hashlib.sha256(p.read_bytes()).hexdigest() for p in (executable, cli)}
    (stage / 'BUILD.json').write_text(json.dumps(manifest, indent=2) + '\n')
    out = root / 'dist' / 'preview'
    out.mkdir(parents=True, exist_ok=True)
    archive = out / ('Smotri-' + args.version + '-' + args.platform + '-preview.zip')
    with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=6) as z:
        for path in sorted(stage.rglob('*')):
            if path.is_file():
                z.write(path, Path('Smotri') / path.relative_to(stage))
    (out / (archive.name + '.sha256')).write_text(hashlib.sha256(archive.read_bytes()).hexdigest() + '  ' + archive.name + '\n')
    print(archive)


if __name__ == '__main__':
    main()
