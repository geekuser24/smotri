#!/usr/bin/env python3
"""Build-time icon derivation. Requires Pillow; end users need no Python."""
from pathlib import Path
from PIL import Image, ImageDraw


def macos_icon(source):
    """Use one rounded, inset tile for both the bundle and running Dock icon."""
    # Render at 2x for smooth corners even in the largest ICNS representation.
    size, inset, radius = 2048, 192, 368
    tile_size = size - 2 * inset
    tile = source.resize((tile_size, tile_size), Image.Resampling.LANCZOS)
    mask = Image.new('L', (tile_size, tile_size))
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, tile_size - 1, tile_size - 1), radius=radius, fill=255)
    tile.putalpha(mask)
    canvas = Image.new('RGBA', (size, size))
    canvas.alpha_composite(tile, (inset, inset))
    return canvas.resize((1024, 1024), Image.Resampling.LANCZOS)


root=Path(__file__).resolve().parents[1]
with Image.open(root/'assets/smotri/icon.png') as source:
    image=source.convert('RGBA')
    for size in (16,24,32,48,64,128,256,512):
        image.resize((size,size),Image.Resampling.LANCZOS).save(root/f'assets/app-icon/hicolor/{size}x{size}/apps/ai.storyteller.filmcraft.png')
    for path,size in [('assets/app-icon/filmcraft-1024.png',1024),('apps/filmcraft-web/web/favicon.png',128)]:
        image.resize((size,size),Image.Resampling.LANCZOS).save(root/path)
    image.save(root/'assets/app-icon/filmcraft.ico',sizes=[(s,s) for s in (16,24,32,48,64,128,256)])
    mac = macos_icon(image)
    mac.resize((512,512),Image.Resampling.LANCZOS).save(root/'assets/app-icon/filmcraft-macos-512.png')
    mac.save(root/'assets/smotri/Smotri.icns')
    mac.save(root/'assets/app-icon/filmcraft.icns')
