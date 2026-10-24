#!/usr/bin/env python3
"""Build-time icon derivation. Requires Pillow; end users need no Python."""
from pathlib import Path
from PIL import Image
root=Path(__file__).resolve().parents[1]
with Image.open(root/'assets/smotri/icon.png') as source:
    image=source.convert('RGBA')
    for size in (16,24,32,48,64,128,256,512):
        image.resize((size,size),Image.Resampling.LANCZOS).save(root/f'assets/app-icon/hicolor/{size}x{size}/apps/ai.storyteller.filmcraft.png')
    for path,size in [('assets/app-icon/filmcraft-1024.png',1024),('assets/app-icon/filmcraft-macos-512.png',512),('apps/filmcraft-web/web/favicon.png',128)]:
        image.resize((size,size),Image.Resampling.LANCZOS).save(root/path)
    image.save(root/'assets/app-icon/filmcraft.ico',sizes=[(s,s) for s in (16,24,32,48,64,128,256)])
    image.save(root/'assets/smotri/Smotri.icns')
    image.save(root/'assets/app-icon/filmcraft.icns')
