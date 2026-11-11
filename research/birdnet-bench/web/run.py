"""Run the test page in one browser and record its results and peak memory.

  python run.py LABEL -- COMMAND ... URL

Starts serve.py, runs COMMAND (a browser given the page URL, with {url} replaced),
samples the summed resident memory (RSS) of the browser's process tree every 200 ms,
waits until the page posts its "finished" result (or the timeout), then stops both.
Writes results-LABEL.jsonl next to this script and prints a summary.

Example:
  python run.py firefox -- firefox --headless --no-remote --profile /tmp/p "{url}"
"""

import json
import os
import signal
import subprocess
import sys
import time

here = os.path.dirname(os.path.abspath(__file__))
label = sys.argv[1]
cmd = sys.argv[sys.argv.index("--") + 1 :]
port = int(os.environ.get("BENCH_PORT", "21390"))
timeout = float(os.environ.get("BENCH_TIMEOUT", "300"))
phases = os.environ.get("BENCH_PHASES", "cold,warm,main")
results = os.path.join(here, f"results-{label}.jsonl")
if os.path.exists(results):
    os.remove(results)
query = f"label={label}&phases={phases}"
url = f"http://localhost:{port}/index.html?{query}"
cmd = [c.replace("{url}", url) for c in cmd]
with open(os.path.join(here, "data", "default-query"), "w") as f:
    f.write(query)

server = subprocess.Popen(
    [sys.executable, "-I", os.path.join(here, "serve.py"), "--port", str(port), "--results", results],
    stdout=subprocess.DEVNULL,
)
time.sleep(0.5)


def pss_kb(pid):
    try:
        with open(f"/proc/{pid}/smaps_rollup") as f:
            for line in f:
                if line.startswith("Pss:"):
                    return int(line.split()[1])
    except OSError:
        pass
    return 0


def tree_rss_mb(root):
    """Summed PSS of the process tree: shared pages are split between processes."""
    out = subprocess.run(["ps", "-eo", "pid=,ppid="], capture_output=True, text=True).stdout
    kids = {}
    for line in out.split("\n"):
        p = line.split()
        if len(p) == 2:
            pid, ppid = map(int, p)
            kids.setdefault(ppid, []).append(pid)
    total, stack = 0, [root]
    while stack:
        pid = stack.pop()
        total += pss_kb(pid)
        stack += kids.get(pid, [])
    return total / 1024


browser = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
t0 = time.time()
peak = 0.0
samples = []
finished = False
while time.time() - t0 < timeout:
    m = tree_rss_mb(browser.pid)
    peak = max(peak, m)
    samples.append((round(time.time() - t0, 1), round(m)))
    if os.path.exists(results):
        with open(results) as f:
            lines = [json.loads(l) for l in f if l.strip()]
        if any(l.get("phase") in ("finished",) or "error" in l for l in lines):
            finished = True
            break
    time.sleep(0.2)

os.killpg(browser.pid, signal.SIGTERM)
server.terminate()
time.sleep(0.5)
try:
    os.killpg(browser.pid, signal.SIGKILL)
except ProcessLookupError:
    pass

print(f"{label}: {'finished' if finished else 'TIMED OUT'} in {time.time() - t0:.0f} s, peak browser PSS {peak:.0f} MB")
with open(results, "a") as f:
    f.write(json.dumps({"label": label, "phase": "rss", "peakMB": round(peak), "samples": samples}) + "\n")
if os.path.exists(results):
    for l in open(results):
        d = json.loads(l)
        if d.get("phase") in ("rss",):
            continue
        d.pop("top1", None)
        print(json.dumps(d))
