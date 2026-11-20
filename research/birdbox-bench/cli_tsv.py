"""BirdBox CLI JSON (from run_cli.sh) to the tab-separated layouts the Rust code writes.

  python -I cli_tsv.py CLI_DIR OUT_DIR NAME [NAME ...]

CLI_DIR/NAME.merged/with_algorithm_metadata.json -> OUT_DIR/NAME.cli.boxes.tsv
  (t0, t1, f0, f1, avg_conf, max_conf, merged)
CLI_DIR/NAME.raw/raw_detections.json -> OUT_DIR/NAME.cli.raw.tsv
  (clip_start, t0, t1, f0, f1, conf)
"""

import json
import os
import sys

src, out = sys.argv[1], sys.argv[2]
os.makedirs(out, exist_ok=True)
for name in sys.argv[3:]:
    merged = json.load(open(os.path.join(src, f"{name}.merged", "with_algorithm_metadata.json")))["detections"]
    with open(os.path.join(out, f"{name}.cli.boxes.tsv"), "w") as f:
        for d in merged:
            f.write("\t".join(f"{d[k]:.6f}" for k in (
                "time_start", "time_end", "freq_low_hz", "freq_high_hz",
                "avg_confidence", "max_confidence", "detections_merged")) + "\n")
    raw = json.load(open(os.path.join(src, f"{name}.raw", "raw_detections.json")))["detections"]
    with open(os.path.join(out, f"{name}.cli.raw.tsv"), "w") as f:
        for d in raw:
            f.write("\t".join(f"{d[k]:.6f}" for k in (
                "clip_start", "time_start", "time_end", "freq_low_hz", "freq_high_hz", "confidence")) + "\n")
    print(f"{name}: {len(raw)} window boxes, {len(merged)} merged boxes")
