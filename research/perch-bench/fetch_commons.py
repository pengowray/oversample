"""Download sound-event test recordings (rain, traffic, wind, voices...) from Wikimedia
Commons into a folder, with an index.json of each file's kind, source, author and licence.

  python fetch_commons.py OUT_DIR

Files already in OUT_DIR/sounds are not downloaded again.
"""

import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

FILES = {
    "rain": [
        "Rain and wind in London 2026 08 28.wav",
        "Heavy Rain in Summer.ogg",
        "Heavy rain in Glenshaw, PA.ogg",
        "RAIN-1.ogg",
    ],
    # A music track by Antti Luode, despite the name; Perch called it music.
    "music": ["Fall Rain (Antti Luode).mp3"],
    "traffic": ["Ambient sound street traffic Berlin 2026-05-17.oga", "Sounds of Traffic and Sellers.ogg"],
    "wind": ["Howling wind.ogg"],
    "thunder": ["Summer thunderstorm in the woods.ogg"],
    "voices": ["Group discussion 1.ogg", "1 minute at the alexa mall in berlin.ogg"],
    "water": ["Shallow small river with stony riverbed.ogg"],
}
UA = {"User-Agent": "perch-bench-research/0.1"}
API = "https://commons.wikimedia.org/w/api.php"

def fetch(url):
    """Commons answers 429 to quick runs of downloads: wait between files, and longer
    after each 429."""
    for wait in (5, 30, 60, 120):
        time.sleep(wait)
        try:
            with urllib.request.urlopen(urllib.request.Request(url, headers=UA)) as r:
                return r.read()
        except urllib.error.HTTPError as e:
            if e.code != 429:
                raise
            print(f"429, waiting before retrying {url}")
    raise SystemExit(f"still 429 after retries: {url}")


out = sys.argv[1]
os.makedirs(os.path.join(out, "sounds"), exist_ok=True)
titles = [f"File:{n}" for names in FILES.values() for n in names]
q = urllib.parse.urlencode(
    {
        "action": "query",
        "titles": "|".join(titles),
        "prop": "imageinfo",
        "iiprop": "url|size|extmetadata",
        "format": "json",
    }
)
pages = json.load(urllib.request.urlopen(urllib.request.Request(f"{API}?{q}", headers=UA)))["query"]["pages"]
info = {p["title"][len("File:") :]: p["imageinfo"][0] for p in pages.values()}

index = []
for kind, names in FILES.items():
    for name in names:
        ii = info[name]
        meta = ii["extmetadata"]
        path = os.path.join(out, "sounds", name)
        if not os.path.exists(path):
            data = fetch(ii["url"])
            with open(path, "wb") as f:
                f.write(data)
            print(f"{name}: {len(data) / 1e6:.1f} MB")
        index.append(
            {
                "filename": name,
                "kind": kind,
                "source": ii["descriptionurl"],
                "author": meta.get("Artist", {}).get("value", ""),
                "licence": meta.get("LicenseShortName", {}).get("value", ""),
                "seconds": round(ii.get("duration") or 0, 1),
            }
        )
with open(os.path.join(out, "index.json"), "w", encoding="utf-8") as f:
    json.dump({"sounds": index}, f, indent=1, ensure_ascii=False)
print(f"{len(index)} files in {out}")
