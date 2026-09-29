import struct, zlib

W = H = 1024
# simple brand-ish gradient/diagonal split so the icon isn't a flat block
def px(x, y):
    # deep indigo background with a lighter diagonal band
    if abs((x / W) - (y / H)) < 0.14:
        return (99, 102, 241, 255)   # indigo-500 band
    return (30, 27, 75, 255)         # indigo-950 bg

raw = bytearray()
for y in range(H):
    raw.append(0)  # filter type 0
    for x in range(W):
        raw.extend(px(x, y))

def chunk(tag, data):
    return (struct.pack(">I", len(data)) + tag + data +
            struct.pack(">I", zlib.crc32(tag + data) & 0xffffffff))

ihdr = struct.pack(">IIBBBBB", W, H, 8, 6, 0, 0, 0)  # 8-bit RGBA
png = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) +
       chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b""))

with open("icon-source.png", "wb") as f:
    f.write(png)
print("wrote icon-source.png", len(png), "bytes")
