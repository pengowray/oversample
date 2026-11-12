"""Build countries-110m.json, the offline country outlines the app uses to turn a
recording's GPS position into a country (and from that a Bat book region).

Source: Natural Earth 1:110m Admin 0 countries, version 5.1.2 (public domain),
https://github.com/nvkelso/natural-earth-vector/blob/v5.1.2/geojson/ne_110m_admin_0_countries.geojson

  python scripts/make-countries.py ne_110m_admin_0_countries.geojson countries-110m.json

Output: a JSON list, one entry per country: [names, rings], where names lists
the English name variants (NAME, NAME_LONG, ADMIN, NAME_EN, deduplicated) and
rings is a list of rings, each a flat list [lon, lat, lon, lat, ...] rounded to
0.01 degrees (about 1 km). Holes are kept as rings of their own; the app's
even-odd point-in-polygon test treats them correctly.
"""

import json
import sys

src, dst = sys.argv[1], sys.argv[2]
out = []
for f in json.load(open(src, encoding="utf-8"))["features"]:
    p = f["properties"]
    names = []
    for k in ("NAME", "NAME_LONG", "ADMIN", "NAME_EN"):
        v = (p.get(k) or "").strip()
        if v and v not in names:
            names.append(v)
    g = f["geometry"]
    polys = g["coordinates"] if g["type"] == "MultiPolygon" else [g["coordinates"]]
    rings = []
    for poly in polys:
        for ring in poly:
            flat = []
            for lon, lat in ring:
                flat += [round(lon, 2), round(lat, 2)]
            rings.append(flat)
    out.append([names, rings])
json.dump(out, open(dst, "w", encoding="utf-8"), separators=(",", ":"))
print(dst, len(out), "countries")
