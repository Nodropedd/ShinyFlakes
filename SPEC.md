# ShinyFlakes — Privacy-Focused Multi-Chain Wallet

## Overview
A desktop wallet supporting BTC, LTC, XMR, SOL, TRON, USDC, and USDT, built around
bucketed fund management ("wallets within a wallet") and strong local privacy/security
defaults. Runs entirely on the user's machine — no backend server.

## Platform & Stack
- Shell: Tauri (not Electron) — smaller footprint, no Node/Chromium runtime bloat,
  and native Rust backend integration.
- Backend: Rust — memory-safe by design, mature crypto ecosystem
  (rust-bitcoin, monero-rs, solana-sdk), zeroize crate for wiping key
  material from memory after use.
- Frontend: Svelte 5 + TypeScript, talking to the Rust backend via Tauri's IPC
  bridge only — no external network-facing IPC of any kind.
- OS target: Windows first, cross-platform later if desired (Tauri supports this
  natively without a rewrite).

## Core Security Model

### Login
- Seedphrase-only login (BIP-39). No password-only fallback.
- Keys derived into memory on unlock, held only as long as needed, zeroed on
  lock/timeout/logout via zeroize.
- Full logout state = complete key wipe, return to cold-start screen.

### App visibility / isolation
- No dock/taskbar icon by default (configurable).
- No global hotkey registration.
- Single-instance lock; no external deep-link or IPC handler — nothing outside the
  signed binary itself can trigger it to open or query it.
- Generic/randomized process name instead of an identifiable one.
- Disable OS accessibility-API exposure of window contents where the platform
  allows it, to block screen-reader/automation-based content scraping.
- Caveat to set expectations correctly: true invisibility from every other
  process on the OS isn't achievable — anything with sufficient OS permissions can
  enumerate running processes. This hardens against casual/opportunistic access,
  not a fully compromised OS.

### Encryption at rest
- AES-256-GCM for all local secrets: seed (encrypted, never plaintext), SMTP
  credentials, bucket metadata, settings.
- Key derived from an OS-keychain-backed secret, not from a user-guessable value.

### Email notifications (local-only, no server)
- Direct SMTP connection from the app to the user's own mail provider (Rust
  lettre crate), using an app-specific password stored encrypted locally.
- Triggers a notification email on:
  - Any seed phrase reveal
  - 2FA code dispatch (see below)

### 2FA (email-code based)
- On trigger, generate a random numeric code — 6 digits (000000-999999),
  not 4 digits, to avoid a trivially brute-forceable 10,000-value space.
- Delivered via the same local SMTP connection.
- Code held in memory only, 3-5 minute expiry.
- Attempt policy:
  - Attempts 1-3 wrong: standard "incorrect code" message, retry allowed.
  - After 3rd failure: hard lockout, 1 minute — input disabled, countdown
    shown, same code remains valid (not reissued).
  - Attempts 4-5 (post-cooldown) use the same code.
  - After 5th failure total: 2FA abandoned, force fallback to seedphrase login.
- Seedphrase fallback policy:
  - 3 failed seedphrase attempts leads to full logout (complete key wipe,
    cold-start screen), not just a re-prompt.
- 2FA gates: seed reveal, private key reveal, seed regeneration/rotation.
  (Optional, configurable: large payments, bucket add/remove, SMTP settings changes.)

## Bucketed Wallet Model ("wallets within a wallet")

The user can divide funds into named buckets inside one master wallet. Payments draw
from a specific bucket; if that bucket lacks sufficient funds, the app prompts the
user to pull the shortfall from another bucket before completing the payment. More
buckets touched means more fees, shown transparently before confirming.

### BTC / LTC (true UTXO chains)
- Each UTXO tagged locally with a bucket label; bucket balance = sum of its UTXOs.
- Payment from bucket A uses A's UTXOs first.
- Shortfall handling: pull the full shortfall from one other bucket in a single
  transfer (lump pull, not incremental across many buckets) — minimizes both fee
  events and the number of visible inter-bucket linkages on-chain.
- Route consolidation transfers to a fresh unused address within the receiving
  bucket rather than reusing a previously-public address, to avoid trivial address
  reuse heuristics.
- UTXO fragmentation UI: before/after diagram of UTXO layout, fee shown in native
  unit + live USD equivalent, total value retained vs. spent on fees.

### XMR
- Use native subaddresses per bucket/account — the correct native primitive
  for this, cheaper and more private than emulating the BTC/LTC bucket model.
- Ring signatures + stealth addresses already provide the privacy properties the
  other chains need bolt-on workarounds for.

### SOL / TRON / USDC / USDT (account-based chains)
- Buckets = separate derived addresses/accounts under the same seed, tracked via
  local ledger metadata.
- Cross-bucket top-up = a real on-chain transfer between the user's own addresses
  (two-hop cost: transfer + final payment, both with fees) — shown clearly in the
  fee estimate before confirming.
- Known limitation (to disclose in-app, not hide): because buckets share a
  seed, a chain analyst can potentially cluster same-owner addresses via transfer
  patterns. No mixing infrastructure is planned initially — CoinJoin-equivalent
  coordination is a large, security-critical undertaking better scoped as a
  separate future project rather than bolted on early.

## Seed Regeneration ("rotate to new seed")

Not a true "reset" — a seed phrase is inseparable from its derived addresses.
"Regenerating" necessarily means: generate a new seed, then sweep all balances
from every old address/bucket to new ones. This is real on-chain activity with
real fees for every asset holding a balance.

Flow:
1. Require current seedphrase + 2FA to initiate.
2. Scan all buckets/assets for balances.
3. Calculate and display total estimated sweep fees in USD across every asset
   needing a sweep.
4. Explicit typed confirmation required (e.g. type "CONFIRM") before proceeding —
   irreversible action, no undo.
5. Generate new seed only after confirmation; execute sweeps; show the new seed
   once for backup, and don't discard the old seed's derived key material until
   sweep success is confirmed on-chain.

## Design / UI

- Reference style: Exodus-like — dark near-black/charcoal background (not pure
  black), per-asset accent colors used sparingly (icons, highlights, not full
  backgrounds), rounded cards, soft shadows, large legible balance numbers,
  smooth transaction/balance animations, real per-coin icons.
- Explicitly avoid: generic AI-app look (flat purple/blue gradients everywhere,
  glassmorphism overload, default component-library palettes).
- App wordmark "ShinyFlakes" set in a bold connected script, one flat gold,
  no gradient. A gradient across strokes this thin darkens the ends to brown.
  Reference for the letterforms is the Rainbet logotype, a smooth monoline
  retro script with rounded terminals and a swashed capital. Used as the
  primary branding mark on the splash and login screens and in the settings
  header. The gold is fixed and does not follow the user's accent colour.
- The face is Pacifico, bundled in the binary under the SIL Open Font License.
  Nothing is fetched from a font CDN at runtime, because that would leak every
  app launch to a third party.
- Copy style: plain, professional, no marketing slogans, no filler tagline
  formulas — reads like a real product, not generated placeholder text.
- UTXO/bucket operations always show: amounts moved, fee in native unit + live USD
  equivalent (price feed via CoinGecko or similar), before/after layout diagram,
  net value retained vs. spent on fees.

## Settings

- GUI theme/accent color customization (full color picker, not just presets).
- SMTP/email configuration.
- 2FA on/off, lockout thresholds (configurable within safe bounds).
- Bucket rules: default split behavior for incoming funds, fee-pull-order
  preference for shortfalls.
- Tip the creator: a settings button that sends a payment to a fixed,
  developer-controlled address. Shows the destination address and an editable
  amount before sending, with an explicit confirm step — never a silent/automatic
  transfer.

## Chain Connectivity (recommended starting point)

TODO — this section was truncated in the source document. Fill it in before any
network-facing work begins. Questions the scaffold deliberately does not answer:

- Which node or indexer per chain, own node versus third-party API.
- How outbound requests are proxied for privacy, if at all.
- Rate limits and failover between providers.
- Trust model for balance, UTXO set, and fee data returned by a third party,
  given the wallet shows those numbers to the user as authoritative.
