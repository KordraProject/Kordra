#!/usr/bin/env python3
"""Generate the build-time icon files using Python stdlib only.
Icons are generated on GitHub-hosted runners and never vendored as font files.
"""
from pathlib import Path
import struct
import zlib
import binascii

ROOT = Path(__file__).resolve().parent.parent / "src-tauri" / "icons"
ROOT.mkdir(parents=True, exist_ok=True)

def chunk(label: bytes, payload: bytes) -> bytes:
    return struct.pack(">I", len(payload)) + label + payload + struct.pack(">I", binascii.crc32(label + payload) & 0xffffffff)

def png(n: int) -> bytes:
    raw = bytearray()
    for y in range(n):
        raw.append(0)
        for x in range(n):
            color = (24, 33, 52) if x < n//8 or x >= n*7//8 or y < n//8 or y >= n*7//8 else (88, 121, 240)
            raw.extend((*color, 255))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">2I5B", n, n, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(bytes(raw))) + chunk(b"IEND", b"")

for n in (32, 128):
    (ROOT / f"{n}x{n}.png").write_bytes(png(n))
small = png(32)
ico = struct.pack("<HHH", 0, 1, 1) + struct.pack("<BBBBHHII", 32, 32, 0, 0, 1, 32, len(small), 22) + small
(ROOT / "icon.ico").write_bytes(ico)
print("Generated Kordra PNG and ICO assets")
