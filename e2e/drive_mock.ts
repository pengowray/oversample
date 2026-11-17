import type { Page, Route } from "@playwright/test";

/**
 * A mock of the Oversample API's `/google/*` routes and of the parts of the
 * Google Drive REST API the app uses, for e2e specs (google_drive.spec.ts).
 */

export const EMAIL = "bat.person@example.com";
/** The mock account's Drive `permissionId` (its cache folder on this device). */
export const ACCOUNT_ID = "04512345678901234567";

type DriveFile = {
  id: string;
  name: string;
  mimeType: string;
  parents: string[];
  appProperties: Record<string, string>;
  content: string;
  createdTime: string;
  modifiedTime: string;
  headRevisionId: string;
  trashed: boolean;
};

type Upload = { at: number; method: string; name: string; appProperties: Record<string, string>; content: string };

export class MockDrive {
  files = new Map<string, DriveFile>();
  uploads: Upload[] = [];
  logouts = 0;
  signedIn = true;
  configured = true;
  /** Answer uploads with 503 (Drive unreachable). */
  failUploads = false;
  /** Called for each file query that names an `osKey`, before filtering. */
  onKeyQuery: ((kind: string, key: string) => void) | null = null;
  private n = 0;

  add(f: Partial<DriveFile> & { name: string }): DriveFile {
    const now = new Date(Date.now() - 60_000).toISOString();
    const file: DriveFile = {
      id: `f${++this.n}`,
      mimeType: "text/yaml",
      parents: [],
      appProperties: {},
      content: "",
      createdTime: now,
      modifiedTime: now,
      headRevisionId: `r${this.n}`,
      trashed: false,
      ...f,
    };
    this.files.set(file.id, file);
    return file;
  }

  meta(f: DriveFile) {
    const { content: _content, ...rest } = f;
    return rest;
  }
}

const CORS = {
  "access-control-allow-origin": "*",
  "access-control-allow-headers": "*",
  "access-control-allow-methods": "GET, POST, PATCH, OPTIONS",
};

function json(route: Route, status: number, body: unknown) {
  return route.fulfill({ status, contentType: "application/json", headers: CORS, body: JSON.stringify(body) });
}

/** The `key='…' and value='…'` pairs of a Drive query. */
function queryProps(q: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const m of q.matchAll(/key='((?:[^'\\]|\\.)*)' and value='((?:[^'\\]|\\.)*)'/g)) {
    out[m[1].replace(/\\(.)/g, "$1")] = m[2].replace(/\\(.)/g, "$1");
  }
  return out;
}

function parseMultipart(body: string, contentType: string): { meta: Record<string, unknown>; content: string } {
  const boundary = /boundary=(.+)$/.exec(contentType)![1];
  const parts = body.split(`--${boundary}`).slice(1, -1);
  const partBody = (p: string) => p.slice(p.indexOf("\r\n\r\n") + 4).replace(/\r\n$/, "");
  return { meta: JSON.parse(partBody(parts[0])), content: partBody(parts[1]) };
}

export async function mockServer(page: Page, drive: MockDrive) {
  // Same-origin API base: see the note at the top.
  await page.addInitScript(() => localStorage.setItem("oversample_xc_api_base", location.origin));

  await page.route("**/google/*", async (route) => {
    const url = new URL(route.request().url());
    const path = url.pathname;
    if (path.endsWith("/google/status")) {
      if (!drive.configured) return route.fulfill({ status: 404, body: "" });
      return json(route, 200, { configured: true, signed_in: drive.signedIn });
    }
    if (path.endsWith("/google/token")) {
      if (!drive.signedIn) return route.fulfill({ status: 401, body: "" });
      return json(route, 200, { access_token: "test-token", expires_in: 3600 });
    }
    if (path.endsWith("/google/logout")) {
      drive.logouts++;
      drive.signedIn = false;
      return route.fulfill({ status: 204, body: "" });
    }
    return route.fulfill({ status: 404, body: "" });
  });

  await page.route("https://www.googleapis.com/**", async (route) => {
    const req = route.request();
    const method = req.method();
    if (method === "OPTIONS") return route.fulfill({ status: 204, headers: CORS, body: "" });
    if (req.headers()["authorization"] !== "Bearer test-token") return json(route, 401, { error: "no token" });
    const url = new URL(req.url());
    const path = url.pathname;

    if (path === "/drive/v3/about") {
      return json(route, 200, { user: { emailAddress: EMAIL, displayName: "Bat Person", permissionId: ACCOUNT_ID } });
    }

    const upload = path.startsWith("/upload/drive/v3/files");
    const idMatch = /\/files\/([^/]+)$/.exec(path);
    const id = idMatch ? decodeURIComponent(idMatch[1]) : null;

    if (upload) {
      if (drive.failUploads) return json(route, 503, { error: "unavailable" });
      const { meta, content } = parseMultipart(req.postData() ?? "", req.headers()["content-type"]);
      const props = (meta.appProperties ?? {}) as Record<string, string>;
      drive.uploads.push({ at: Date.now(), method, name: String(meta.name), appProperties: props, content });
      let file: DriveFile;
      if (id) {
        file = drive.files.get(id)!;
        file.content = content;
        file.name = String(meta.name);
        file.appProperties = { ...file.appProperties, ...props };
        file.modifiedTime = new Date().toISOString();
        file.headRevisionId = `${file.headRevisionId}+`;
      } else {
        file = drive.add({ name: String(meta.name), parents: (meta.parents as string[]) ?? [], appProperties: props, content });
        file.modifiedTime = new Date().toISOString();
      }
      return json(route, 200, drive.meta(file));
    }

    if (!id && method === "GET") {
      const q = url.searchParams.get("q") ?? "";
      const props = queryProps(q);
      if (props.osKey && drive.onKeyQuery) drive.onKeyQuery(props.osKind, props.osKey);
      const wantFolder = q.includes("mimeType='application/vnd.google-apps.folder'");
      const files = [...drive.files.values()].filter(
        (f) =>
          !f.trashed &&
          (!wantFolder || f.mimeType === "application/vnd.google-apps.folder") &&
          Object.entries(props).every(([k, v]) => f.appProperties[k] === v),
      );
      return json(route, 200, { files: files.map((f) => drive.meta(f)) });
    }
    if (!id && method === "POST") {
      const meta = JSON.parse(req.postData() ?? "{}");
      const folder = drive.add({ name: meta.name, mimeType: meta.mimeType, parents: meta.parents ?? [], appProperties: meta.appProperties ?? {} });
      return json(route, 200, drive.meta(folder));
    }
    if (id && method === "GET") {
      const file = drive.files.get(id);
      if (!file) return json(route, 404, { error: "not found" });
      if (url.searchParams.get("alt") === "media") {
        return route.fulfill({ status: 200, contentType: "text/yaml", headers: CORS, body: file.content });
      }
      return json(route, 200, drive.meta(file));
    }
    if (id && method === "PATCH") {
      const file = drive.files.get(id);
      if (!file) return json(route, 404, { error: "not found" });
      Object.assign(file, JSON.parse(req.postData() ?? "{}"));
      return json(route, 200, drive.meta(file));
    }
    return json(route, 400, { error: `unexpected ${method} ${path}` });
  });
}

/** The desktop app's Google commands, stubbed (google_drive_desktop.spec.ts). */
export type Stub = {
  available: boolean;
  signedIn: boolean;
  persistent: boolean;
  /** How the next google_sign_in ends: "ok" or an error code. */
  nextSignIn: string;
  signInCalls: number;
  central: Record<string, string>;
};

/** Run the page as if inside the desktop app, with the Google commands and
 * the central annotations folder answered by a stub. */
export async function desktopApp(page: Page, stub: Partial<Stub>) {
  await page.addInitScript((initial) => {
    const st = {
      available: true,
      signedIn: false,
      persistent: true,
      nextSignIn: "ok",
      signInCalls: 0,
      central: {} as Record<string, string>,
      cancel: null as null | (() => void),
      ...initial,
    };
    (window as unknown as { __googleStub: typeof st }).__googleStub = st;
    const status = () => ({ available: st.available, signedIn: st.signedIn, persistent: st.persistent });
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {
      transformCallback: () => 0,
      metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
      invoke: (cmd: string, args: Record<string, string>) => {
        switch (cmd) {
          case "google_status":
            return Promise.resolve(status());
          case "google_sign_in":
            st.signInCalls++;
            return new Promise((resolve, reject) => {
              st.cancel = () => reject("cancelled");
              if (st.nextSignIn === "wait") return;
              setTimeout(() => {
                if (st.nextSignIn === "ok") {
                  st.signedIn = true;
                  resolve(status());
                } else reject(st.nextSignIn);
              }, 300);
            });
          case "google_sign_in_cancel":
            st.cancel?.();
            return Promise.resolve(null);
          case "google_access_token":
            return st.signedIn
              ? Promise.resolve({ accessToken: "test-token", expiresIn: 3600 })
              : Promise.reject("signed_out");
          case "google_sign_out":
            st.signedIn = false;
            return Promise.resolve(null);
          case "list_central_annotations":
            return Promise.resolve(Object.keys(st.central));
          case "read_central_annotations":
            return Promise.resolve(st.central[args.fileKey] ?? null);
          case "write_central_annotations":
            st.central[args.fileKey] = args.yaml;
            return Promise.resolve(null);
          default:
            return Promise.resolve(null);
        }
      },
    };
  }, stub);
}
