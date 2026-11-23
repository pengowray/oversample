// Small browser helpers the Rust code calls by name (plain `extern "C"`
// imports in src/opfs_audio.rs and src/components/xc_browser.rs).
//
// They are global functions in this one file, not wasm-bindgen `inline_js`
// blocks: each inline block came out as about 130 copies in dist/snippets,
// and Trunk told the page to preload every copy (382 requests at start-up).

// English name of a language code ("de" gives "German"); the code itself if
// the browser doesn't know it.
function language_name(code) {
    try { return new Intl.DisplayNames(['en'], { type: 'language' }).of(code) || code; }
    catch (e) { return code; }
}

// A language's name in that language ("de" gives "Deutsch").
function language_autonym(code) {
    try { return new Intl.DisplayNames([code], { type: 'language' }).of(code) || code; }
    catch (e) { return code; }
}

function opfs_audio_supported() {
    return typeof FileSystemFileHandle !== "undefined"
        && "createWritable" in FileSystemFileHandle.prototype
        && !!(navigator.storage && navigator.storage.getDirectory);
}

// Takes the lock `name` and keeps it until the tab closes. Resolves once the
// lock is held.
function opfs_audio_hold_lock(name) {
    if (!navigator.locks) return Promise.resolve();
    return new Promise((held) => {
        navigator.locks.request(name, () => { held(); return new Promise(() => {}); });
    });
}

// Deletes every folder in `dir` whose lock (`prefix` and the folder's name)
// is free, which means the tab that made it has closed.
async function opfs_audio_sweep(dir, prefix, own) {
    if (!navigator.locks) return;
    const names = [];
    for await (const name of dir.keys()) names.push(name);
    for (const name of names) {
        if (name === own) continue;
        await navigator.locks.request(prefix + name, { ifAvailable: true }, async (lock) => {
            if (lock) await dir.removeEntry(name, { recursive: true }).catch(() => {});
        });
    }
}

async function opfs_audio_remove(dir, name) {
    await dir.removeEntry(name, { recursive: true }).catch(() => {});
}

// Bytes the site may still store, or -1 when the browser does not say.
async function opfs_audio_free_bytes() {
    if (!navigator.storage || !navigator.storage.estimate) return -1;
    const e = await navigator.storage.estimate();
    if (typeof e.quota !== "number" || typeof e.usage !== "number") return -1;
    return Math.max(0, e.quota - e.usage);
}
