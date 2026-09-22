# ShinyFlakes

A private, multi-chain wallet that runs entirely on your own machine. No
account, no backend, no telemetry — your keys are encrypted on the device and
never leave it.

Built with Tauri: a Rust core and a Svelte front end, for Android, Windows and
Linux.

**[Download](https://nodropedd.github.io/ShinyFlakes/)** ·
[Releases](https://github.com/Nodropedd/ShinyFlakes/releases) · [Spec](SPEC.md)

## Coins

Bitcoin, Litecoin, Monero, Ethereum, Solana and Tron, plus USDC and USDT on
Solana, Ethereum and Tron.

## What it does

- **Stays on your device.** The seed is sealed with AES-256-GCM under a key
  your OS credential store holds. Add a passphrase and unlocking also needs
  something only you know.
- **Routes through Tor.** Balance, price and history lookups otherwise show a
  server your IP next to your addresses. Turn Tor on and they see a Tor exit
  instead. On Android, Tor ships inside the app.
- **Runs Monero properly.** Monero's own wallet daemon runs alongside the app.
  Your keys go from memory straight into it over loopback — never to a file or
  the screen.
- **Swaps without an account.** Trade one coin for another through ChangeNOW.
  You send from your own wallet and receive to your own address.
- **Asks again when it matters.** A TOTP code, checked offline, before
  revealing the seed, before a large send, and on the first unlock after a
  long absence.
- **Clears itself if abandoned.** Leave the wallet unopened past a limit you
  set and the device forgets it. The coins stay on-chain; the seed restores
  everything.

## Download

Prebuilt downloads for every platform are on the
[project site](https://nodropedd.github.io/ShinyFlakes/) and under
[Releases](https://github.com/Nodropedd/ShinyFlakes/releases), each with a
published SHA-256.

Android needs 7.0 or newer; Monero needs an ARM device, which is nearly all
phones. Windows needs 10 or 11. Linux ships as an `.AppImage`, a `.deb`, an
`.rpm` and an Arch package.

## Sending

Every send previews first. The app builds and signs the real transaction, then
has a node simulate it — catching a bad address, an unaffordable amount or a
malformed transaction before anything is broadcast. Confirming fetches a fresh
blockhash and simulates once more.

> **Move a small amount first.** No send path has been exercised with
> significant real funds yet.

## Two-factor

The second factor is TOTP: a six-digit code from an authenticator app on your
phone, checked entirely offline. Nothing crosses a network, so there is no
server and nothing in the binary to extract. The seed phrase works as a
fallback when the authenticator isn't to hand.

| Prompt | Configurable |
|---|---|
| Showing the seed or a private key | Always on |
| A send over $50 that is also ≥10% of the wallet | On/off |
| First unlock after 15 days unopened | Days, or off |

The large-send threshold is priced in Rust, not read from the interface, so an
attacker who already controls the UI can't understate it. If prices can't be
fetched it fails closed — anything over the dollar threshold counts as large.

## Updates

Settings shows when a newer version is out. It asks `api.github.com` once per
session, through Tor when that's on, and otherwise only when you press the
button. **Update** opens the download page in your browser.

The app never downloads or installs updates itself. A wallet that fetched and
ran its own builds would hand anyone who seized the release account a path onto
every user's machine. You download and install each version yourself.

## Network access

Nothing is contacted until you press **Connect and load balances**. That choice
is remembered per machine.

| Data | Endpoint |
|---|---|
| BTC balance | mempool.space |
| LTC balance | litecoinspace.org |
| SOL and USDC balances | api.mainnet-beta.solana.com |
| TRX and USDT balances | api.trongrid.io |
| USD prices | api.coingecko.com |

Each endpoint learns the addresses it is asked about and the machine's IP, and
can link them — turn on Tor to hide the IP. Monero is different: its outputs
can't be read from an address, so its own daemon scans the chain with your
private view key.

## Build from source

The source builds the same app as the official downloads, with two differences:

- **Swaps are off.** ChangeNOW needs a partner key on every request. That key
  is in the official builds only, not in this repository, so a source build
  shows a disabled swap screen. To turn it on, get your own key from ChangeNOW
  and follow the steps at the top of `src-tauri/src/swapcfg.rs`.
- **Android needs your own signing key.** Android won't install an unsigned
  app, and the key the official builds use isn't published. Make one below.

Everything else — Tor, Monero, balances, sending, two-factor — is identical.

### Prerequisites

Node 20+ and a Rust toolchain (via rustup).

### Run and build

```bash
npm run tauri dev     # dev window against Vite on :1420
npm run tauri build   # installer for the current OS
```

On Linux, `tauri build` produces a `.deb`, `.rpm` and `.AppImage` in
`src-tauri/target/release/bundle/`. The AppImage needs FUSE 2 (`fuse2` on
Arch), or run it with `--appimage-extract-and-run`.

**Arch.** Tauri makes no Arch package, so `packaging/arch/PKGBUILD` repackages
the `.deb`: copy the `.deb` next to it and run `makepkg -si`. It links the
system WebKitGTK, so it's a few MB where the AppImage is around 80.

**Windows, cross-compiled from Linux.** Needs `clang-cl`, `lld-link`,
`llvm-rc`, `llvm-lib`, NSIS, cmake, nasm, `cargo-xwin` and the
`x86_64-pc-windows-msvc` target. `cargo-xwin` downloads Microsoft's CRT and SDK
(about 1 GB), which means accepting Microsoft's licence:

```bash
XWIN_ACCEPT_LICENSE=1 npm run tauri build -- --runner cargo-xwin \
  --target x86_64-pc-windows-msvc --bundles nsis
```

Neither Windows build is code-signed, so SmartScreen warns on first run.

### Android

Needs the Android SDK, the NDK, and a JDK Gradle accepts (21 works, 25 doesn't):

```bash
export ANDROID_HOME="$HOME/Android/Sdk"
export NDK_HOME="$ANDROID_HOME/ndk/26.1.10909125"
export JAVA_HOME=/usr/lib/jvm/java-21-openjdk
npm run tauri android build -- --apk --target aarch64 --target armv7 --target x86_64
```

The first build downloads about 226 MB of Tor and Monero binaries into
`src-tauri/target/android-binaries` and reuses them afterwards.

The APK comes out unsigned. Make a key once and keep the file and its password
safe — `*.keystore` is gitignored:

```bash
keytool -genkeypair -keystore src-tauri/sideload.keystore -alias sideload \
  -keyalg RSA -keysize 4096 -validity 10000
```

Then align and sign:

```bash
BT="$ANDROID_HOME/build-tools/35.0.0"
IN=src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk
"$BT/zipalign" -P 16 -f 4 "$IN" aligned.apk
"$BT/apksigner" sign --ks src-tauri/sideload.keystore --out ShinyFlakes.apk aligned.apk
```

> **`-P 16` matters.** Arm64 phones on Android 15+ (and Pixel 8 and later) use
> a 16 KB memory page. A library aligned for the old 4 KB page won't map, and
> the app opens to a black screen with only `UnsatisfiedLinkError` in the log.
> `build.rs` aligns the library's segments; `-P 16` aligns it within the APK.
> x86_64 falls back to 4 KB, so a mistake here passes on the emulator and fails
> on real hardware.

Android only updates an app with an APK signed by the same key, so a build you
signed yourself won't install over the official one — uninstall that first. The
seed restores the wallet.

## Address derivation

| Asset | Path | Format | Verified against |
|---|---|---|---|
| BTC | `m/84'/0'/0'/0/0` | bech32 P2WPKH | the address published in BIP-84 |
| LTC | `m/84'/2'/0'/0/0` | bech32 P2WPKH, `ltc` prefix | structure |
| SOL | `m/44'/501'/0'/0'` | base58 ed25519 | SLIP-0010 vectors |
| TRON | `m/44'/195'/0'/0/0` | base58check, `0x41` prefix | structure |
| XMR | `m/44'/128'/0'/0/0` | base58 mainnet | structure |

Solana uses the same path as Phantom. Solflare uses `m/44'/501'/0'`, so a
wallet restored there shows different addresses from the same seed. USDC and
USDT have no addresses of their own — they show the Solana and Tron addresses
that hold them.

Monero has no standard BIP-39 mapping, so this wallet defines one: derive a
secp256k1 key at `m/44'/128'/0'/0/0`, reduce it modulo the ed25519 order for the
spend key, then take the view key as keccak256 of the spend key. **The BIP-39
phrase alone won't restore Monero in the official GUI** — that wants its own
25-word seed. Cross-check the Solana and Tron addresses against a reference
wallet before sending real funds.

## Security

- The vault key is random and lives in the OS credential store, never derived
  from anything you type. The encrypted vault file is useless copied off the
  machine on its own.
- The key is read per operation, not cached, so it sits in memory only for the
  length of a call.
- Seed material is held in zeroizing wrappers and wiped on lock, logout and
  exit.
- The webview holds `core:default` only — no filesystem, shell, HTTP, dialog or
  clipboard access from JavaScript. Every privileged action is an explicit Rust
  command.
- `contentProtected` sets the OS exclude-from-capture flag, so screenshots and
  most screen recorders see a blank window.
- On Android the webview has no network of its own: every request is made by
  the Rust core, and the webview's own traffic is dead-ended so nothing leaks
  around Tor.

None of this hides the process from something already running with real
privileges on the machine. It raises the cost of casual access, not of a
compromised OS.

## Layout

```
src/                     Svelte front end
  lib/ipc.ts             The full UI-to-core API, typed
  routes/                Login and wallet screens
src-tauri/src/
  crypto/                AES-256-GCM, BIP-39, SLIP-0010
  keychain/              Vault key in the OS credential store
  store.rs               Encrypted vault file, atomic writes
  chains/                Address derivation and per-chain calls
  ipc/commands.rs        Commands the UI can call
```

## License

© 2026 ShinyFlakes. All rights reserved.

You're welcome to build on the source. If you ship something based on it, keep
a visible ShinyFlakes credit in the app and link back to this repository.
