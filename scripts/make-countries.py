"""Build countries-50m.bin, the offline country outlines the app uses to turn a
recording's GPS position into a country (and from that a Bat book region).

Source: Natural Earth 1:50m Admin 0 countries, version 5.1.2 (public domain),
https://github.com/nvkelso/natural-earth-vector/blob/v5.1.2/geojson/ne_50m_admin_0_countries.geojson
(1:110m was tried first: it has no Corfu, Rhodes, Gotland, Orkney, Malta, Okinawa,
Singapore or Hong Kong.)

  python scripts/make-countries.py ne_50m_admin_0_countries.geojson src/data/countries-50m.bin

Coordinates are rounded to 0.01 degrees (about 1 km) and stored as hundredths of
a degree in signed 16-bit integers; repeated points after rounding are dropped.
Holes are kept as rings of their own; the app's even-odd point-in-polygon test
treats them correctly.

Format, all little-endian, read by src/bat_book/countries.rs:
  u16 country count, then per country:
    u8 name count, then per name: u8 byte length + UTF-8 bytes
      (the English name variants NAME, NAME_LONG, ADMIN, NAME_EN, deduplicated)
    u16 ring count, then per ring: u32 point count + that many (i16 lon, i16 lat)
"""

import json
import struct
import sys

src, dst = sys.argv[1], sys.argv[2]
out = bytearray()
features = json.load(open(src, encoding="utf-8"))["features"]
out += struct.pack("<H", len(features))
points = 0
for f in features:
    p = f["properties"]
    names = []
    for k in ("NAME", "NAME_LONG", "ADMIN", "NAME_EN"):
        v = (p.get(k) or "").strip()
        if v and v not in names:
            names.append(v)
    out += struct.pack("<B", len(names))
    for n in names:
        b = n.encode("utf-8")
        out += struct.pack("<B", len(b)) + b
    g = f["geometry"]
    polys = g["coordinates"] if g["type"] == "MultiPolygon" else [g["coordinates"]]
    rings = []
    for poly in polys:
        for ring in poly:
            pts = []
            for lon, lat in ring:
                pt = (round(lon * 100), round(lat * 100))
                if not pts or pts[-1] != pt:
                    pts.append(pt)
            if len(pts) >= 4:  # at least 3 distinct corners plus the closing point
                rings.append(pts)
    out += struct.pack("<H", len(rings))
    for pts in rings:
        out += struct.pack("<I", len(pts))
        for lon, lat in pts:
            out += struct.pack("<hh", lon, lat)
        points += len(pts)
open(dst, "wb").write(out)
print(dst, len(features), "countries,", points, "points,", len(out), "bytes")
