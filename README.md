# ShinyFlakes

A local-only, privacy-focused multi-chain wallet. Tauri shell, Rust core,
Svelte 5 front end. There is no backend server and no network surface other
than the outbound connections the Rust side makes on the user's behalf.

The full product spec is in [SPEC.md](SPEC.md).

## Status

Working: vault creation, encryption at rest, seed validation, unlock and lock,
and real receiving addresses derived from the seed for all five chains.
Derivation runs offline and is covered by tests, including the address
published in BIP-84 and the SLIP-0010 ed25519 vectors.

Balances and USD prices are live for Bitcoin, Litecoin, Solana, Tron, USDC and
USDT, read from public endpoints after the user opts in.

Sending works for Solana only. The transfer is built, signed, and run through
a node in simulation before the user is asked to confirm, then broadcast.

Not built: sending on any other chain, buckets beyond an empty list, SMTP,
2FA, and Monero balances.

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
- `skipTaskbar` is on and drag-drop is disabled. Single-instance is enforced.
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
