# ShinyFlakes

A local-only, privacy-focused multi-chain wallet. Tauri shell, Rust core,
Svelte 5 front end. There is no backend server and no network surface other
than the outbound connections the Rust side makes on the user's behalf.

The full product spec is in [SPEC.md](SPEC.md).

**Downloads** for Android, Windows and Linux are on the project site,
https://nodropedd.github.io/ShinyFlakes/, and under Releases. Building it yourself works too, with the
differences listed under "Building from source" below.

## Status

Assets: Bitcoin, Litecoin, Monero, Ethereum, Solana, Tron, and USDC and USDT
on Solana, Ethereum and Tron.

Working: vault creation, encryption at rest, seed validation, unlock and lock,
and receiving addresses derived from the seed for every asset. Derivation runs
offline and is covered by tests, including the address published in BIP-84
and the SLIP-0010 ed25519 vectors.

Balances and prices are read from public endpoints after the user opts in,
through Tor when that is on. Monero balances come from Monero's own wallet
daemon running locally, since they cannot be read from an address.

Sending is built for every asset and always previews the transaction before
asking for confirmation. Swaps go through ChangeNOW (official builds only; see
"Building from source").

Two-factor works, as an authenticator app (TOTP) checked entirely offline.
See "Asking again" below.

## Sending

Solana transfers are built against the legacy transaction format directly
rather than through solana-sdk. The encoding is verified two ways: unit tests
assert the exact byte layout and check the signature against the message, and
a live test asks mainnet to price the serialized message, which it can only do
by deserializing it successfully.

Every send goes preview first. The preview builds and signs the real
transaction and has a node execute it in simulation, which catches a bad
recipient, an unaffordable amount or a malformed transaction before anything
is broadcast. Confirming re-fetches a blockhash and simulates once more, since
the balance may have moved in between.

No send path has been exercised with real funds. The one account available for
testing is a well-known public address that has been assigned to a third-party
program, so it cannot pay fees. Move a small amount first.

## Asking again

Unlocking needs the seed phrase. Some moments are worth a second question,
for when the machine itself is what went wrong — stolen, or running malware:

| Trigger | Configurable |
|---|---|
| Showing the seed phrase or a private key | always on |
| A send over $50 that is also 10% or more of the wallet | on/off |
| First unlock after 15 days unopened | days, or off |

The factor is TOTP: a six-digit code computed on the user's phone from a
shared secret. It never crosses a network, so there is nothing in transit to
intercept, no server involved and nothing in the binary to extract. The seed
phrase works as a fallback when the authenticator is not to hand, since
proving the seed is at least as strong as proving a code.

The large-send threshold is priced on the Rust side rather than taken from the
interface, because a figure supplied by the caller is one that an attacker who
already owns the interface could understate. If prices cannot be fetched it
fails closed: anything over the dollar threshold counts as large.

There is no email in this wallet. Delivering a code by mail needs either a
domain to authenticate from or a relay account to send through — Gmail and the
other large providers now reject unauthenticated senders outright — and both
mean a credential or a third party the wallet would rather not have. TOTP is
stronger on every axis that matters here anyway.

## Android

The same Rust core and the same Svelte front end, built as an APK. Several
things work differently on a phone, mostly because the platform's rules
leave no other way:

**The vault key has no OS keychain to live in.** Android offers no Secret
Service and no store a process can address by name, so the key is a file in
the directory Android gives this app. That directory is sandboxed — no other
app reads it without root — but it is weaker than the desktop backends, where
the OS holds the key and hands it to a logged-in session. What closes the gap
is the optional vault passphrase, which is mixed with this key exactly as it
is on desktop. On a phone it stops being optional in spirit. Hardware-backed
storage would mean the Android Keystore over JNI, which is not done yet.

**Tor and Monero come inside the APK.** On the desktop both are downloaded the
first time they are used. Android will not run a file an app downloaded, but
it will run one the package installer extracted from the APK's native-library
folder — read-only to the app, and verified with the rest of the package. So
`scripts/android-binaries.mjs` fetches the projects' own Android releases at
build time (Tor 15.0.22, Monero v0.18.5.1), checks each archive against a
pinned SHA-256, and copies the one file needed into `jniLibs` byte for byte
as `libtor.so` and `libmonero_wallet_rpc.so`. Gradle's legacy packaging keeps
them compressed in the APK and has the installer extract them; `bundled.rs`
finds them there at run time. From then on both behave as on the desktop: Tor
is a child process whose SOCKS port every lookup goes through, and the Monero
wallet daemon is driven over loopback. They add about 16 MB to the APK.

Monero publishes its Android build for ARM only, so on an x86_64 device the
Monero section says so and Tor works alone. Both children are started with
every inherited descriptor marked close-on-exec: the WebView shares the app's
process and does not always do that itself, and without it Tor and the daemon
held the WebView's shared memory, its GPU channels and its data-directory lock.

The Monero daemon talks to its node directly, as it does on the desktop; Tor
covers the wallet's own lookups, not the Monero node connection.

**The WebView has no network of its own.** Every wallet request is made by the
Rust core, and the app's pages are served in-process, so the WebView never
needs to fetch anything. It tried to anyway: finding the seed-phrase box, it
sent the form's shape to `content-autofill.googleapis.com`, directly and so
around Tor. `MainActivity` points all of the WebView's own traffic at a proxy
address where nothing listens, so anything it fetches for itself fails without
leaving the phone (release builds only; `tauri android dev` needs the network
to reach Vite). It also excludes the whole window from Android autofill, so no
autofill service is offered the seed phrase or passphrase fields, and the
manifest opts the WebView out of Safe Browsing and metrics.

**The WebView may be years old.** Android updates it through the Play Store,
so a phone that is old, lacks Play Services, or runs a vendor ROM can be far
behind Chrome. Svelte 5's runtime calls `String.prototype.replaceAll` on every
render, and that arrived in Chrome 85, so `index.html` polyfills it and shows
a readable message if start-up fails for some other reason. Lowering the Vite
target does not help with a missing built-in: esbuild rewrites syntax, not
library functions.

This was once thought to be why the APK came up black. It was not — the cause
was the 16 KB page alignment described under the build steps below, which
stops the app before a single line of front-end code runs. The polyfill is
still worth keeping for genuinely old WebViews, but a black screen is not the
symptom it fixes. If the app is black again, check `adb logcat` for
`UnsatisfiedLinkError` first.

**HTTPS trusts Mozilla's roots, not the phone's.** reqwest checks
certificates through the OS by default, and on Android that verifier has to be
handed the JVM before its first use. Nothing did, so the first request —
pressing "Connect and load balances" — panicked, and `panic = "abort"` took the
app with it. Android clients now carry Mozilla's root store compiled in, which
also works on Android 7.0, whose own store predates Let's Encrypt's root. A CA
installed on the phone is not trusted, which for a wallet is the point. Every
client must be built from `http_client::builder()`; one made straight from
`reqwest::Client::builder()` will crash on Android the same way.

**A failed start says why.** The launcher is `StartupActivity`, a plain
activity that checks the Rust library loads and a WebView exists before
starting the wallet. Both are loaded inside `WryActivity.onCreate`, and a
failure there cannot be caught in place — it re-fires from every lifecycle
callback `WryActivity` overrides — so the activity dies with the window still
showing the theme background and the user gets a black screen and nothing to
report. Checking first turns that into a readable message with the device,
Android version, ABIs, WebView version and the exception. If the front end
loads but never mounts, `index.html` does the same from the other side after
five seconds. Together they mean a black screen is itself a bug report: if you
ever see one, the diagnostics are what failed.

These live in `src-tauri/gen/android/`, which is otherwise generated and
ignored; `.gitignore` re-admits `StartupActivity.kt`, `MainActivity.kt` and
`AndroidManifest.xml` specifically, because regenerating would otherwise drop
them without a word.

**The layout is vertical.** Below 720px the sidebar becomes a bottom bar and
the gutters tighten. One stylesheet, no forked components.

Building one needs the Android SDK, the NDK, and a JDK Gradle accepts — 21
works, 25 does not:

```bash
export ANDROID_HOME="$HOME/Android/Sdk"
export NDK_HOME="$ANDROID_HOME/ndk/26.1.10909125"
export JAVA_HOME=/usr/lib/jvm/java-21-openjdk-amd64
npm run tauri android build -- --apk --target aarch64 --target armv7 --target x86_64
```

The first Android build downloads about 226 MB — Tor for all three
architectures and Monero for both ARM ones — into `src-tauri/target/
android-binaries`, and later builds reuse it. That step runs from Tauri's
before-build hook and does nothing for desktop builds.

That APK comes out unsigned, which Android will not install. Sign it with a
key of your own. Make one once, and keep the file and its password to
yourself — `*.keystore` is ignored by git so it cannot be committed by
accident:

```bash
keytool -genkeypair -keystore src-tauri/sideload.keystore -alias sideload \
  -keyalg RSA -keysize 4096 -validity 10000
```

Then align and sign (apksigner asks for the password):

```bash
BT="$ANDROID_HOME/build-tools/35.0.0"
IN=src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release-unsigned.apk
"$BT/zipalign" -P 16 -f 4 "$IN" aligned.apk
"$BT/apksigner" sign --ks src-tauri/sideload.keystore --out ShinyFlakes.apk aligned.apk
```

Android only updates an installed app with an APK signed by the same key, so
an APK you sign yourself will not install over the official one: uninstall
that first. Uninstalling removes the wallet from the phone, which the seed
phrase restores.

`-P 16` is load-bearing. Pixel 8 and later, and every arm64 phone on Android
16, use a 16 KB memory page. A library laid out for the old 4 KB page cannot
be mapped on one: the linker rejects it, `WryActivity.onCreate` dies before it
builds a WebView, and the app shows a black window with nothing in the log but
`UnsatisfiedLinkError`. Two separate things have to be right. `zipalign`'s
`-p` means 4 KB specifically, so `-P 16` is what keeps the library aligned
*within* the APK; the link argument set in `build.rs` is what aligns the
segments *inside* the library. x86_64 has a 4 KB fallback and arm64 does not,
so getting either wrong passes on every emulator and fails on the phone.

## Network access

Nothing is contacted until the user presses "Connect and load balances" on the
portfolio. That choice is remembered per machine.

| Data | Endpoint |
| --- | --- |
| BTC balance | mempool.space |
| LTC balance | litecoinspace.org |
| SOL and USDC balances | api.mainnet-beta.solana.com |
| TRX and USDT balances | api.trongrid.io |
| USD prices | api.coingecko.com |

Each of these learns the addresses being asked about and the machine's IP, and
can link them. Requests carry a generic user agent rather than naming the
wallet. Making the endpoints configurable, and routing them through a proxy,
is the open work described under Chain Connectivity in SPEC.md.

Monero is absent by necessity, not oversight. Its outputs cannot be found from
an address; discovering them means scanning the chain with the private view
key, which needs a node or a light-wallet server rather than a REST call.

## Building from source

The source here builds the same app as the official downloads, with these
differences:

- **Swaps are off.** ChangeNOW needs a partner API key on every request, and
  the official downloads carry the distributor's key compiled in (why, and how
  thinly it is hidden, is at the top of `src-tauri/src/swapcfg.rs`). That key
  is not in this repository. A build without it shows that no key was compiled
  in and never contacts ChangeNOW. To enable swaps in your own build, get a
  key from ChangeNOW and follow the steps at the top of `swapcfg.rs` to write
  `src-tauri/swap_key.obf`, which git ignores.
- **Android builds need your own signing key.** The key the downloads are
  signed with is not published; see the Android section for making one.

Everything else — Tor, Monero, balances, sending, two-factor — works the same.

## Prerequisites

Node 20+ and a Rust toolchain. Both are installed on this machine: Node 20.11
and Rust 1.98.1 via rustup. Cargo's bin directory was added to the user PATH,
so open a new shell if `cargo` is not found in an existing one. Visual Studio
build tools and the WebView2 runtime were already present.

## Running

```bash
npm run tauri dev
```

That starts Vite on port 1420 and launches the Tauri window against it.
`npm run dev` alone runs the front end in a browser, where every IPC call
fails, which is only useful for styling work.

To build an installer:

```bash
npm run tauri build
```

On Linux that makes a `.deb`, an `.rpm` and an `.AppImage` under
`src-tauri/target/release/bundle/`. The AppImage needs FUSE 2 to start
(`fuse2` on Arch), or run it with `--appimage-extract-and-run`.

**Arch.** Tauri makes no Arch package, so `packaging/arch/PKGBUILD`
repackages the `.deb`: copy the `.deb` next to it and run `makepkg -si`
(needs base-devel). It links against the system WebKitGTK, so it is a few
megabytes where the AppImage is seventy-five.

**Windows, from Linux.** Cross-compiling needs `clang-cl`, `lld-link`,
`llvm-rc`, `llvm-lib`, NSIS, cmake and nasm, plus `cargo-xwin` and the
`x86_64-pc-windows-msvc` Rust target. `cargo-xwin` downloads Microsoft's CRT
and Windows SDK (about 1 GB, cached in `~/.cache/cargo-xwin`), which means
accepting Microsoft's licence — setting `XWIN_ACCEPT_LICENSE=1` does that.

```bash
XWIN_ACCEPT_LICENSE=1 npm run tauri build -- --runner cargo-xwin \
  --target x86_64-pc-windows-msvc --bundles nsis
```

That gives the app at `src-tauri/target/x86_64-pc-windows-msvc/release/
shinyflakes.exe` and an installer under `bundle/nsis/`. Only NSIS works this
way; an MSI needs WiX, which runs on Windows alone. Neither is signed, so
SmartScreen warns on first run.

## Layout

```
src/                    Svelte front end
  lib/ipc.ts            The entire UI-to-core API surface, typed
  lib/session.svelte.ts Which screen we belong on
  routes/               Login and Wallet screens
  assets/fonts/         Pacifico, the brand script face, bundled not fetched
src-tauri/src/
  crypto/aead.rs        AES-256-GCM seal and open
  crypto/seed.rs        BIP-39 generate, parse, derive
  keychain/mod.rs       Vault key in the OS credential store
  store.rs              Encrypted vault file, atomic writes
  session.rs            In-memory unlocked state
  chains/mod.rs         Address derivation per chain
  chains/slip10.rs      SLIP-0010 ed25519 derivation, used by Solana
  ipc/commands.rs       The commands the UI can call
```

## Address derivation

| Asset | Path | Format | Verified against |
| --- | --- | --- | --- |
| BTC | `m/84'/0'/0'/0/0` | bech32 P2WPKH | the address published in BIP-84 |
| LTC | `m/84'/2'/0'/0/0` | bech32 P2WPKH, `ltc` prefix | structure only |
| SOL | `m/44'/501'/0'/0'` | base58 ed25519 key | SLIP-0010 vectors for the derivation steps |
| TRON | `m/44'/195'/0'/0/0` | base58check, `0x41` prefix | structure only |
| XMR | `m/44'/128'/0'/0/0` | base58, standard mainnet | structure only |

Solana uses the same path as Phantom. Solflare uses `m/44'/501'/0'` instead,
so a wallet restored there will show different addresses from the same seed.

USDC and USDT have no addresses of their own. They are held by the account on
their host chain, so they show the Solana and Tron addresses respectively.

Monero does not use BIP-32, and there is no standard mapping from a BIP-39
phrase to Monero keys. The convention here is: derive a secp256k1 key at
`m/44'/128'/0'/0/0`, read those bytes as a little-endian scalar and reduce them
modulo the ed25519 group order to get the private spend key, then take the
private view key as keccak256 of the spend key reduced the same way. The last
step is Monero's own rule; the first two are this wallet's choice.

That means **the BIP-39 phrase alone will not restore Monero in the official
GUI**. That wallet wants its own 25 word seed, or a restore from the spend and
view keys. Exposing those keys behind a confirmation is not built yet, so the
derivation above is the only record of how to reproduce them.

**Before sending real funds**, cross-check the Solana and Tron addresses
against a reference wallet. Only the Bitcoin value is pinned to a published
vector; the other two are locked against regression but not against an
external source.

## Security posture as built

- The vault key is random and lives in the Windows credential store. It is
  never derived from anything the user types, so the encrypted vault file is
  useless if copied off this machine alone.
- The key is fetched per vault read or write rather than cached, so it sits in
  process memory only for the length of a call.
- Seed material is held in `Zeroizing` wrappers and dropped on lock, logout,
  and process exit.
- The webview holds `core:default` only. No filesystem, shell, HTTP, dialog, or
  clipboard plugin is reachable from JavaScript. Every privileged action is an
  explicit command in `ipc/commands.rs`.
- `contentProtected` is on, which sets the Windows exclude-from-capture flag,
  so screenshots and most screen recorders see a blank window.
- `skipTaskbar` is on for Windows, where it removes the taskbar button and Alt+Tab\n  still reaches the window. Linux turns it off: on X11 GNOME honours it by hiding\n  the window from the dash, Alt+Tab and the overview together, so a covered\n  window could not be found again. Drag-drop is disabled. Single-instance is\n  enforced.
  No deep-link or global-hotkey handler is registered anywhere.

Worth stating plainly, per the spec's own caveat: none of this hides the
process from anything running with real privileges on the machine. It raises
the cost of casual access, not of a compromised OS.

## Open decisions

These are places the scaffold made a call that you should confirm.

**Logout is non-destructive.** The spec calls a failed-attempt logout a
"complete key wipe". That was read as in-memory key material only. The
encrypted vault and the keychain entry survive, so three mistyped attempts
return the user to seed entry rather than destroying the wallet. The
destructive reading is irreversible for anyone whose offline backup is not
current, so it should not be the default without an explicit decision.

**The dev CSP is loose.** `connect-src` in `tauri.conf.json` allows
`ws://localhost:1420` and `http://localhost:1420` so Vite hot reload works.
Both must come out before any release build.

**Frontend framework.** The spec left React or Svelte open. Svelte was chosen
because it compiles away rather than shipping a runtime, which matches the
stated reason for choosing Tauri over Electron. Swapping is cheap now and
expensive later.

**Process name.** The spec asks for a generic or randomized process name. The
binary is currently `shinyflakes.exe`. Renaming is a one-line change in
`Cargo.toml`, but a randomized name per install fights code signing and
SmartScreen reputation, so it needs a decision rather than a default.

**Accessibility-API exposure.** The spec asks to disable it. `contentProtected`
covers screen capture but not UI Automation. Blocking that on Windows means
opting the window out of UIA, which also breaks real screen readers. Not
implemented pending a call on that tradeoff.

**Chain connectivity is unspecified.** SPEC.md ends with the section truncated.
Nothing network-facing should be built until it says which nodes or indexers
each chain talks to and what the trust model is for the numbers they return.
