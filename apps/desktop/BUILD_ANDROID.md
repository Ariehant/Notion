# Building the Android app (Android 11 / API 30+)

The desktop and Android apps are the **same Tauri 2 project** (`apps/desktop`).
The Rust engine (`core`), the command layer (`src-tauri`), and the React frontend
are shared verbatim; only the entry point differs (`src/lib.rs`'s
`mobile_entry_point`, called by Tauri on Android — there is no `main` there). This
guide covers turning that shared project into an installable `.apk`/`.aab`.

> **Why this isn't done in CI / the web session:** compiling for Android needs the
> Android SDK **and** NDK, plus a cross-compiled SQLCipher + OpenSSL. Those aren't
> present in the headless build image. Run the steps below on a machine (or CI
> runner) with the Android toolchain installed.

## 1. Prerequisites (one time)

- **JDK 17**, **Android SDK** (Platform 35 + Build-Tools), **Android NDK** (r26+).
  Easiest via Android Studio → SDK Manager, or `sdkmanager`.
- Environment:
  ```bash
  export ANDROID_HOME="$HOME/Android/Sdk"
  export NDK_HOME="$ANDROID_HOME/ndk/<version>"   # Tauri also reads ANDROID_NDK_HOME
  export PATH="$ANDROID_HOME/platform-tools:$PATH"
  ```
- Rust Android targets + `cargo-ndk`:
  ```bash
  rustup target add aarch64-linux-android armv7-linux-androideabi \
    i686-linux-android x86_64-linux-android
  cargo install cargo-ndk
  ```
- JS deps: `pnpm install` (the `@tauri-apps/cli` is already a devDependency).

## 2. Generate the Android project

From `apps/desktop`:

```bash
pnpm exec tauri android init
```

This scaffolds `src-tauri/gen/android` (a Gradle project). `gen/` is **gitignored**
— it is regenerated from `tauri.conf.json` and the config below, so treat it as
build output, not source.

`tauri.conf.json` already sets `bundle.android.minSdkVersion = 30` (Android 11).

## 3. The one real risk: SQLCipher + OpenSSL on the NDK

`core` links SQLCipher via `rusqlite` with
`bundled-sqlcipher-vendored-openssl`. The vendored OpenSSL (`openssl-src`) must
compile with the **NDK** toolchain. `cargo-ndk` wires the per-target `CC`/`AR`/
linker; a clean `tauri android build` will drive it. If the vendored build fights
the NDK, fall back to either:

1. an NDK-built OpenSSL on `OPENSSL_DIR` + `bundled-sqlcipher` (non-vendored), or
2. a prebuilt SQLCipher NDK static lib linked via `SQLCIPHER_LIB_DIR`.

**Do this as the first spike** — prove `cargo ndk -t arm64-v8a build -p notion_core`
opens an encrypted DB on an emulator before investing in the rest. Keep the DB /
KDF / wrap formats byte-identical to desktop so a vault stays portable between
phone and desktop (required once sync exists).

> Argon2id is configured at 128 MiB (t=3). That's fine on Android 11+ hardware;
> validate on a low-RAM (~3 GB) device. **Do not lower the parameters** if vaults
> are meant to open on both desktop and mobile — interop requires identical KDF
> params.

## 4. Manifest overrides (edit after `init`, in `gen/android`)

In `src-tauri/gen/android/app/src/main/AndroidManifest.xml`:

- Permissions (minimal — no storage permission; the vault lives in app-private
  internal storage):
  ```xml
  <uses-permission android:name="android.permission.INTERNET" />
  <uses-permission android:name="android.permission.USE_BIOMETRIC" />
  <uses-permission android:name="android.permission.POST_NOTIFICATIONS" />
  ```
- Disable Google cloud auto-backup so the encrypted vault never leaves the device:
  ```xml
  <application android:allowBackup="false" ... >
  ```
- Block screenshots / app-switcher previews while unlocked by adding
  `getWindow().setFlags(FLAG_SECURE, FLAG_SECURE)` in the generated
  `MainActivity` (Kotlin). The frontend already auto-locks on background
  (`visibilitychange`) as a second layer.

Because `gen/` is regenerated, keep these edits documented here (or add a small
post-`init` patch script) so they survive a re-init.

## 5. Run & build

```bash
pnpm exec tauri android dev                 # emulator or attached device (USB debugging)
pnpm exec tauri android build --apk         # universal/split APKs
pnpm exec tauri android build --aab         # Play Store bundle
```

Artifacts land under `src-tauri/gen/android/app/build/outputs/`.

## 6. Signing

- **Play Store:** create an upload keystore, configure `signingConfigs` in the
  generated `app/build.gradle.kts` (or `keystore.properties`), and enable **Play
  App Signing**.
- **Direct APK / F-Droid:** sign with a stable release key you control.
- Never commit keystores or passwords (already covered by `.gitignore`'s
  `*.key` / `.env` rules — keep secrets out of the repo).

## 7. Distribution

- **Google Play** — upload the `.aab`; fill the Data safety form as _no data
  collected, on-device only, no account_.
- **F-Droid** — a strong fit for an open-source, no-telemetry app (reproducible
  builds, no Google dependency).
- **Direct APK** — attach to GitHub Releases alongside the desktop artifacts.

## 8. Documented follow-ups (not wired yet)

These are deliberately left as clean next steps so the first build stays minimal:

- **Biometric unlock:** add `tauri-plugin-biometric` (+ `@tauri-apps/plugin-biometric`)
  and a mobile-only capability (`biometric:default`), then wrap the DEK with an
  Android Keystore key gated by `BiometricPrompt`. The password stays the portable
  root; biometrics only guard a local re-wrap.
- **Home-screen agenda widget + Quick Settings tile + reminders:** a Jetpack
  Glance widget reading the shared `calendar_events` table (shows a "locked" state
  when the vault is locked) — the Android analog of the desktop GNOME companion.
- **Encrypted phone↔desktop sync:** wire the pairing / wrapped-DEK / Ed25519
  identity already in `core` to the relay — ship only after the external security
  review.
