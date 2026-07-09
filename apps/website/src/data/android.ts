/**
 * Content for the Android landing page (/android), kept as typed data so the
 * sections stay presentational and the copy is unit-testable. Mirrors the
 * desktop content but adapted for the mobile story: biometric unlock, a
 * home-screen agenda widget, on-device AI, and mobile-lifecycle security.
 *
 * The Android app is in development (the desktop app ships today); the download
 * section is framed accordingly.
 */
import type { Feature, Pillar, SecurityStep, TrustChip } from "./features";

export interface Store {
  name: string;
  icon: TrustChip["icon"];
  channel: string;
  note: string;
}

export const androidTagline = "The offline-first, encrypted notes app — now on Android.";

export const androidTrustChips: TrustChip[] = [
  { icon: "wifiOff", label: "Works offline" },
  { icon: "lock", label: "Encrypted on-device" },
  { icon: "fingerprint", label: "Biometric unlock" },
  { icon: "sparkle", label: "On-device AI" },
  { icon: "github", label: "Open source" },
];

export const androidPillars: Pillar[] = [
  {
    icon: "lock",
    eyebrow: "Write",
    title: "An encrypted notebook in your pocket",
    summary:
      "The same fast block editor as the desktop app, with every page stored inside an encrypted database on your phone.",
    features: [
      {
        title: "Block editor, touch-first",
        body: "Headings, lists, to-dos, quotes and code via a slash menu (/) and markdown shortcuts — laid out for a phone, with a slide-in page drawer.",
      },
      {
        title: "Encrypted at rest",
        body: "Notes, the search index, and version snapshots live in SQLite + SQLCipher in your app-private storage. Nothing is written in the clear.",
      },
      {
        title: "Full-text search",
        body: "Search across your vault instantly — the FTS index lives inside the encrypted file, never a plaintext sidecar.",
      },
      {
        title: "Never locked out",
        body: "A one-time recovery code resets a forgotten password without losing data — the encryption key, not the password, is the root.",
      },
    ],
  },
  {
    icon: "calendar",
    eyebrow: "See",
    title: "Your agenda on the home screen",
    summary:
      "A native home-screen widget and Quick Settings tile surface your day — the Android take on the desktop's calendar companion.",
    features: [
      {
        title: "Home-screen widget",
        body: "Today's and upcoming events at a glance, reading the same encrypted database the app writes. No second app, no copies of your data.",
      },
      {
        title: "Quick Settings tile & reminders",
        body: "Jump straight to quick-add from the notification shade, and get reminders for what's next.",
      },
      {
        title: "Locked means locked",
        body: "The widget shows only a locked state until you unlock the app — it never caches your keys.",
      },
      {
        title: "One encrypted database",
        body: "Everything reads and writes one SQLCipher file — the same design that keeps the desktop companion in sync with zero duplication.",
      },
    ],
  },
  {
    icon: "sparkle",
    eyebrow: "Think",
    title: "On-device AI, no cloud",
    summary:
      "Optional AI runs locally — semantic search, ingestion, and an action agent. Nothing is sent to a server you don't control.",
    features: [
      {
        title: "Search that works offline",
        body: "Hybrid semantic + keyword search finds notes by meaning and exact term, fully offline with a built-in embedder — no model download required.",
      },
      {
        title: "Ask AI, type an action",
        body: "“Add a meeting tomorrow at 3pm” becomes a real calendar event; ask it to summarize or draft. Every action is logged for transparency.",
      },
      {
        title: "Your model, your device",
        body: "Generative features run on-device or against a local model on your own network (Ollama) — never a cloud API.",
      },
      {
        title: "Off by default",
        body: "AI is opt-in. With it off, the app is a pure encrypted notebook and nothing AI-related runs.",
      },
    ],
  },
];

export const androidSecuritySteps: SecurityStep[] = [
  {
    label: "Password → Argon2id",
    detail: "Your password is stretched with Argon2id (128 MiB, t=3) — never stored.",
  },
  {
    label: "HKDF → DEK",
    detail: "HKDF-SHA256 derives subkeys that wrap a random 256-bit Data Encryption Key.",
  },
  {
    label: "Fingerprint → fast unlock",
    detail: "The Android Keystore (hardware-backed) can re-wrap the key for biometric unlock.",
  },
  {
    label: "XChaCha20-Poly1305",
    detail: "Content is sealed with authenticated encryption; keys never touch the WebView.",
  },
];

export const androidSecurityPoints: Feature[] = [
  {
    title: "Fingerprint or face unlock",
    body: "Unlock with biometrics backed by the hardware Android Keystore. Your password stays the portable root — biometrics only guard a local re-wrap, and it works across devices.",
  },
  {
    title: "Locks the moment you leave",
    body: "The vault auto-locks when the app is backgrounded, wiping keys from memory, and blocks screenshots and app-switcher previews while unlocked.",
  },
  {
    title: "Nothing leaves the device",
    body: "Stored in app-private storage with cloud auto-backup disabled. No account, no telemetry, no cloud — and it's fully open source.",
  },
];

export const androidStores: Store[] = [
  { name: "Google Play", icon: "android", channel: "App bundle (.aab)", note: "Coming soon" },
  { name: "F-Droid", icon: "android", channel: "Open-source store", note: "Reproducible build" },
  {
    name: "Direct APK",
    icon: "download",
    channel: "GitHub Releases",
    note: "Sideload on Android 11+",
  },
];
