"""Look at the BirdNET+ V3.0 labels CSV, write a trimmed label file for the app, and
check how the classes join to the Geomodel files.

  python -I labels.py Labels.csv out/labels.tsv [--geo305 GEOMODEL_V3.0.5_LABELS]
         [--geo304 birdnet-geomodel-v3.0.4-labels.txt] [--bnl BNL_Labels.csv]

The trimmed file is UTF-8, tab-separated, one line per model output in output order:
birdnet_id, scientific name, English name (the scientific name where the CSV has none),
taxon group. No header.

--geo305 is the labels file from BirdNET+_Geomodel_V3.0.5_Global_14K.zip (CSV with a
birdnet_id column, or the tab-separated text file). --geo304 is the label file the app
downloads today (`GEOMODEL_LABELS` in catalog.rs: species code, scientific name, English
name, tab-separated); it has no birdnet_id, so the join is on scientific name.
"""

import argparse
import collections
import csv
import gzip

ap = argparse.ArgumentParser()
ap.add_argument("labels")
ap.add_argument("out")
ap.add_argument("--geo305", default=None)
ap.add_argument("--geo304", default=None)
ap.add_argument("--bnl", default=None)
a = ap.parse_args()

with open(a.labels, encoding="utf-8", newline="") as f:
    rows = list(csv.DictReader(f))
cols = list(rows[0].keys())
print(f"{a.labels}: {len(rows)} rows, {len(cols)} columns")
print("  columns:", ", ".join(c for c in cols if not c.startswith("common_name_")),
      f"+ {sum(c.startswith('common_name_') for c in cols)} common_name_<lang> columns")
assert [int(r["ix"]) for r in rows] == list(range(len(rows))), "ix is not 0..n-1 in order"
print("  taxon groups:", dict(collections.Counter(r["taxon_group"] for r in rows).most_common()))
no_en = [r for r in rows if not r["common_name_en"]]
print(f"  rows with no English name: {len(no_en)} (common_name then repeats the scientific name);"
      f" by group {dict(collections.Counter(r['taxon_group'] for r in no_en))}")
print(f"  unique birdnet_id: {len({r['birdnet_id'] for r in rows})}, "
      f"unique scientific_name: {len({r['scientific_name'] for r in rows})}")
print(f"  ebird_code empty: {sum(not r['ebird_code'] for r in rows)}, inat_id empty: {sum(not r['inat_id'] for r in rows)}")

lines = []
for r in rows:
    en = r["common_name_en"] or r["scientific_name"]
    for v in (r["birdnet_id"], r["scientific_name"], en, r["taxon_group"]):
        assert "\t" not in v and "\n" not in v
    lines.append("\t".join((r["birdnet_id"], r["scientific_name"], en, r["taxon_group"])))
data = ("\n".join(lines) + "\n").encode("utf-8")
with open(a.out, "wb") as f:
    f.write(data)
print(f"trimmed: {a.out}: {len(data):,} bytes, {len(gzip.compress(data, 9)):,} gzipped")

ids = {r["birdnet_id"] for r in rows}
sci = {r["scientific_name"] for r in rows}
animal = {r["birdnet_id"] for r in rows if r["taxon_group"] not in ("Anthropogenic", "Geophony", "Music")}

if a.geo305:
    with open(a.geo305, encoding="utf-8", newline="") as f:
        head = f.readline()
        f.seek(0)
        if "birdnet_id" in head:
            g = list(csv.DictReader(f, delimiter="\t" if "\t" in head else ","))
            gid = [r["birdnet_id"] for r in g]
        else:
            g = [ln.rstrip("\n").split("\t") for ln in f if ln.strip()]
            gid = [r[0] for r in g]
    hit = ids & set(gid)
    print(f"Geomodel V3.0.5 labels: {len(gid)} rows; audio classes with a Geomodel output "
          f"(joined on birdnet_id): {len(hit)} of {len(ids)} ({len(animal & hit)} of {len(animal)} animal classes)")

if a.geo304:
    g4 = [ln.rstrip("\n").split("\t") for ln in open(a.geo304, encoding="utf-8") if ln.strip()]
    s4 = {r[1] for r in g4}
    print(f"Geomodel v3.0.4 labels (app today): {len(g4)} rows; audio classes matched on "
          f"scientific name: {len(sci & s4)} of {len(sci)}")

if a.bnl:
    with open(a.bnl, encoding="utf-8", newline="") as f:
        b = list(csv.DictReader(f))
    bid = [r["birdnet_id"] for r in b]
    pos = {r["birdnet_id"]: int(r["ix"]) for r in rows}
    order = [pos[i] for i in bid]
    print(f"BirdNET Live labels: {len(b)} rows, all in the 12K list: {set(bid) <= ids}, "
          f"same order as the 12K list: {order == sorted(order)}; with geo_ix: {sum(bool(r['geo_ix']) for r in b)}")
    dropped = collections.Counter(r["taxon_group"] for r in rows if r["birdnet_id"] not in set(bid))
    print(f"  12K classes not in BirdNET Live: {sum(dropped.values())}, by group {dict(dropped.most_common())}")
