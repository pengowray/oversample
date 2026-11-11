"""Static server for the Perch test page, with a POST /result endpoint. Copied from
../../birdnet-bench/web.

  python serve.py [--port 21391] [--results results.jsonl]

Serves this folder. Put the model and windows files under data/ (gitignored), for
example symlinks: data/models -> your model folder, data/audio -> your windows folder.
Each POST to /result is appended as one JSON line to the results file.
"""

import argparse
import http.server
import os

ap = argparse.ArgumentParser()
ap.add_argument("--port", type=int, default=21391)
ap.add_argument("--results", default="results.jsonl")
a = ap.parse_args()
root = os.path.dirname(os.path.abspath(__file__))
results = os.path.abspath(a.results)


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *args, **kw):
        super().__init__(*args, directory=root, **kw)

    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def do_POST(self):
        if self.path != "/result":
            self.send_error(404)
            return
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        with open(results, "ab") as f:
            f.write(body.replace(b"\n", b" ") + b"\n")
        self.send_response(204)
        self.end_headers()

    def log_message(self, fmt, *args):
        pass


http.server.SimpleHTTPRequestHandler.extensions_map[".wasm"] = "application/wasm"
http.server.SimpleHTTPRequestHandler.extensions_map[".js"] = "text/javascript"
print(f"serving {root} on http://localhost:{a.port}/, results to {results}", flush=True)
http.server.ThreadingHTTPServer(("127.0.0.1", a.port), Handler).serve_forever()
