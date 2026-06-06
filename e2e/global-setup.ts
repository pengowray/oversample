import fs from "fs";
import path from "path";

/**
 * Generates the fake-audio fixture fed to Chromium's
 * `--use-file-for-fake-audio-capture` so the live-listening e2e specs have a
 * real, *varying* mic stream. A linear chirp (so each spectrogram column
 * differs → the overview visibly repaints), 48 kHz mono 16-bit PCM, looped by
 * Chromium for the duration of the session.
 */
export default async function globalSetup() {
  const dir = path.resolve(process.cwd(), "e2e", "fixtures");
  fs.mkdirSync(dir, { recursive: true });
  const file = path.join(dir, "fake-audio.wav");

  const sampleRate = 48000;
  const seconds = 2;
  const f0 = 400;
  const f1 = 6000;
  const amp = 0.3;
  const n = sampleRate * seconds;
  const dataBytes = n * 2;
  const buf = Buffer.alloc(44 + dataBytes);

  buf.write("RIFF", 0);
  buf.writeUInt32LE(36 + dataBytes, 4);
  buf.write("WAVE", 8);
  buf.write("fmt ", 12);
  buf.writeUInt32LE(16, 16);
  buf.writeUInt16LE(1, 20); // PCM
  buf.writeUInt16LE(1, 22); // mono
  buf.writeUInt32LE(sampleRate, 24);
  buf.writeUInt32LE(sampleRate * 2, 28); // byte rate
  buf.writeUInt16LE(2, 32); // block align
  buf.writeUInt16LE(16, 34); // bits/sample
  buf.write("data", 36);
  buf.writeUInt32LE(dataBytes, 40);

  for (let i = 0; i < n; i++) {
    const t = i / sampleRate;
    // Linear chirp: instantaneous freq sweeps f0 -> f1 over `seconds`.
    const phase = 2 * Math.PI * (f0 * t + ((f1 - f0) * t * t) / (2 * seconds));
    const s = Math.sin(phase) * amp;
    const v = Math.max(-32768, Math.min(32767, Math.round(s * 32767)));
    buf.writeInt16LE(v, 44 + i * 2);
  }

  fs.writeFileSync(file, buf);
}
