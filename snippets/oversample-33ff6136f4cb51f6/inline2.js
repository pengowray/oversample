
export function opfs_audio_supported() {
  return typeof FileSystemFileHandle !== "undefined"
    && "createWritable" in FileSystemFileHandle.prototype
    && !!(navigator.storage && navigator.storage.getDirectory);
}
// Takes the lock `name` and keeps it until the tab closes. Resolves once the
// lock is held.
export function opfs_audio_hold_lock(name) {
  if (!navigator.locks) return Promise.resolve();
  return new Promise((held) => {
    navigator.locks.request(name, () => { held(); return new Promise(() => {}); });
  });
}
// Deletes every folder in `dir` whose lock (`prefix` and the folder's name)
// is free, which means the tab that made it has closed.
export async function opfs_audio_sweep(dir, prefix, own) {
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
export async function opfs_audio_remove(dir, name) {
  await dir.removeEntry(name, { recursive: true }).catch(() => {});
}
// Bytes the site may still store, or -1 when the browser does not say.
export async function opfs_audio_free_bytes() {
  if (!navigator.storage || !navigator.storage.estimate) return -1;
  const e = await navigator.storage.estimate();
  if (typeof e.quota !== "number" || typeof e.usage !== "number") return -1;
  return Math.max(0, e.quota - e.usage);
}
