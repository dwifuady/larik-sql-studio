import os
import struct
import sys

here = os.path.dirname(os.path.abspath(__file__))
sizes = [16, 24, 32, 48, 256]

frames = {}
for s in sizes:
    path = os.path.join(here, f"_frame-{s}.png")
    if not os.path.exists(path):
        sys.exit(f"missing {path} — render it from AppIcon.svg first")
    with open(path, "rb") as f:
        data = f.read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        sys.exit(f"{path} is not a PNG")
    frames[s] = data

header = struct.pack("<HHH", 0, 1, len(sizes))
offset = 6 + 16 * len(sizes)
entries = b""
payload = b""
for s in sizes:
    data = frames[s]
    dim = 0 if s == 256 else s
    entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset)
    offset += len(data)
    payload += data

out = os.path.join(here, "AppIcon.ico")
with open(out, "wb") as f:
    f.write(header + entries + payload)

print(f"{out}: {os.path.getsize(out)} bytes, {len(sizes)} frames")
