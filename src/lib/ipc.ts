// Typed wrappers over the Tauri IPC bridge.
//
// This is the only channel between the UI and the wallet core. Nothing here
// talks to the network, and there is no HTTP or websocket surface anywhere in
// the app: the Rust side owns every secret and every outbound connection.
//
// Secrets crossing this boundary (a mnemonic on unlock) exist as JS strings,
// which cannot be reliably zeroed. Keep them short-lived, never store them in
// component state longer than the call, and never log them.

import { invoke } from "@tauri-apps/api/core";

export type AssetId =
  | "BTC"
  | "LTC"
  | "XMR"
  | "ETH"
  | "SOL"
  | "TRON"
  | "USDC"
  | "USDT";

export interface VaultStatus {
  /** A vault file exists on disk. False means cold start, no wallet yet. */
  initialized: boolean;
  /** Keys are currently derived and held in memory. */
  unlocked: boolean;
  /** Unlocking needs a passphrase on top of the seed phrase. */
  needsPassphrase: boolean;
}

export interface Bucket {
  id: string;
  name: string;
  asset: AssetId;
  /** Smallest unit of the asset, as a decimal string. JS numbers cannot hold
   *  satoshi or atomic-unit values without loss, so amounts never cross the
   *  bridge as numbers. */
  balanceMinor: string;
  addressCount: number;
}

export interface AssetAddress {
  asset: AssetId;
  /** Null when the chain is not supported yet. */
  address: string | null;
  /** BIP-32 path the address came from. */
  path: string | null;
  /** Set when this is a token and the address belongs to its host chain. */
  host: AssetId | null;
  /** Set when there is no address, explaining why. */
  unsupported: string | null;
}

export interface AssetBalance {
  asset: AssetId;
  /** Smallest units as a decimal string, or null if it could not be read. */
  minor: string | null;
  error: string | null;
}

export interface SendQuote {
  asset: AssetId;
  to: string;
  amountMinor: string;
  feeMinor: string;
  totalMinor: string;
  /** A node executed the transfer in simulation and accepted it. */
  simulated: boolean;
}

export interface SendLimits {
  asset: AssetId;
  balanceMinor: string;
  feeMinor: string;
  /** Largest sendable amount. Empties the account. */
  maxMinor: string;
  /** Any remainder must be zero or at least this, never between. */
  rentMinimumMinor: string;
}

export interface Quote {
  price: number;
  /** Percentage move over the last 24 hours, when the feed supplies one. */
  change24h: number | null;
}

export interface ActivityEntry {
  asset: AssetId;
  id: string;
  direction: "in" | "out";
  /** Absolute net movement in smallest units. */
  amountMinor: string;
  /** Unix seconds, absent while unconfirmed. */
  timestamp: number | null;
  confirmed: boolean;
}

export interface UtxoEntry {
  txid: string;
  vout: number;
  valueMinor: string;
  /** Which sub-wallet holds it. Zero is the main receiving address. */
  keyIndex: number;
}

export interface UtxoState {
  asset: AssetId;
  totalMinor: string;
  outputs: UtxoEntry[];
  /** Distinct sub-wallets currently holding something. */
  sources: number;
  feeRate: number;
  maxPieces: number;
  dustMinor: string;
}

export interface FragmentQuote {
  asset: AssetId;
  pieces: number;
  perPieceMinor: string;
  amountMinor: string;
  feeMinor: string;
  changeMinor: string;
  inputsUsed: number;
  /** How many sub-wallets had to be combined to fund it. */
  sources: number;
}

export interface Spendable {
  /** "txid:vout", the way an output is named on chain. */
  outpoint: string;
  valueMinor: string;
  keyIndex: number;
  address: string;
}

export interface ReceiveAddress {
  asset: AssetId;
  address: string;
  index: number;
  /** False when the chain reuses one account rather than rotating. */
  rotates: boolean;
}

export interface MoneroKeys {
  address: string;
  spendKey: string;
  viewKey: string;
  restoreHeightHint: string;
}

export interface MoneroSetup {
  /** Monero's wallet daemon is present and verified. */
  installed: boolean;
  /** A wallet file for this account exists. */
  walletExists: boolean;
  /** The bridge is running right now. */
  running: boolean;
  version: string;
}

export interface MoneroStatus {
  address: string;
  height: number;
  /** True when the daemon is serving the account this wallet derives. */
  matchesWallet: boolean;
}

export interface MoneroBalance {
  totalMinor: string;
  /** Spendable now. Change needs ten blocks to unlock. */
  unlockedMinor: string;
}

export interface MoneroTransfer {
  txHash: string;
  feeMinor: string;
  amountMinor: string;
}

export interface Donation {
  asset: AssetId;
  address: string;
  /** Set when the asset is a token held on another chain. */
  host: AssetId | null;
}

export interface TorState {
  /** The Tor binary is present and verified. */
  installed: boolean;
  /** A Tor process is running. */
  running: boolean;
  /** Requests are being routed through it. */
  routing: boolean;
  version: string;
}

export interface Inactivity {
  /** Unix seconds, absent on a machine that has never been used. */
  lastSeen: number | null;
  /** Months of silence before the switch fires. Zero is never. */
  months: number;
  /** "delete" clears the local wallet; "donate" sweeps first. */
  action: "delete" | "donate";
  daysSince: number;
  /** Days left before the switch fires. Null when set to never. */
  daysRemaining: number | null;
  /** Donate mode: the deadline passed and grace is counting down. Unlocking
   *  now cancels it. */
  graceActive: boolean;
  graceDaysRemaining: number | null;
  /** Donate mode: grace has also passed, a sweep is owed. */
  sweepDue: boolean;
  /** Delete mode: the wallet was cleared by this check, just now. */
  wiped: boolean;
}

export interface SweepResult {
  asset: AssetId;
  /** The transaction id, when something was sent. */
  txid: string | null;
  /** Why nothing was sent, when that is expected. */
  skipped: string | null;
  /** A real failure. */
  error: string | null;
}

/** Mirrors the `WalletError` enum on the Rust side. */
export interface IpcError {
  kind: string;
  message: string;
}

function asIpcError(e: unknown): IpcError {
  if (typeof e === "object" && e !== null && "kind" in e && "message" in e) {
    return e as IpcError;
  }
  // Anything the core did not raise itself is a bridge or runtime fault. Show
  // the user a sentence they can act on and keep the detail in the console;
  // a raw stack trace in a wallet dialog reads as a crash.
  console.error("unexpected IPC failure", e);
  return {
    kind: "Unknown",
    message: "The wallet core did not respond. Restart the app and try again.",
  };
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw asIpcError(e);
  }
}

export const ipc = {
  vaultStatus: () => call<VaultStatus>("vault_status"),

  /** Generates a fresh BIP-39 mnemonic for a new wallet. The caller shows it
   *  once for backup and must not persist it anywhere. */
  generateMnemonic: () => call<string>("generate_mnemonic"),

  createVault: (mnemonic: string) => call<void>("create_vault", { mnemonic }),

  unlock: (mnemonic: string, passphrase?: string | null) =>
    call<void>("unlock", { mnemonic, passphrase }),

  /** Adds, changes or removes the vault passphrase. Empty next removes it. */
  setVaultPassphrase: (current: string | null, next: string | null) =>
    call<void>("set_vault_passphrase", { current, next }),

  /** Drops derived key material, keeps the vault file. */
  lock: () => call<void>("lock"),

  /** Full wipe back to cold start, per the security model. */
  logout: () => call<void>("logout"),

  /** Destroys the wallet stored on this machine. Irreversible without the
   *  seed phrase written down elsewhere. */
  forgetWallet: () => call<void>("forget_wallet"),

  listBuckets: () => call<Bucket[]>("list_buckets"),

  /** Receiving addresses derived from the seed. Local only, no network. */
  listAddresses: () => call<AssetAddress[]>("list_addresses"),

  /** Reaches third-party endpoints and reveals these addresses to them. */
  fetchBalances: () => call<AssetBalance[]>("fetch_balances"),

  /** Spot prices in the given currency, keyed by asset. */
  fetchPrices: (currency: string) =>
    call<Record<string, Quote>>("fetch_prices", { currency }),

  /** Recent transactions across every chain that reports them. */
  fetchActivity: () => call<ActivityEntry[]>("fetch_activity"),

  /** Current output layout on a UTXO chain. */
  utxoState: (asset: AssetId) => call<UtxoState>("utxo_state", { asset }),

  /** Plans sweeping every output back onto the main address. */
  consolidatePreview: (asset: AssetId) =>
    call<SendQuote>("consolidate_preview", { asset }),

  /** Signs and broadcasts the sweep. Irreversible. */
  consolidateExecute: (asset: AssetId) =>
    call<string>("consolidate_execute", { asset }),

  /** Plans a split without signing anything. */
  fragmentPreview: (asset: AssetId, amountMinor: string, pieces: number) =>
    call<FragmentQuote>("fragment_preview", { asset, amountMinor, pieces }),

  /** Signs and broadcasts the split. Irreversible. */
  fragmentExecute: (asset: AssetId, amountMinor: string, pieces: number) =>
    call<string>("fragment_execute", { asset, amountMinor, pieces }),

  /** What the account can afford, so the UI can offer a working maximum. */
  sendLimits: (asset: AssetId) => call<SendLimits>("send_limits", { asset }),

  /** Runs the inactivity check, clearing the wallet if the period passed.
   *  Needs no keys, so it runs before unlocking. */
  inactivityCheck: () => call<Inactivity>("inactivity_check"),

  /** Changes how long the wallet may sit unopened. */
  inactivitySetMonths: (months: number) =>
    call<Inactivity>("inactivity_set_months", { months }),

  /** Chooses whether the switch deletes the wallet or sweeps to donations. */
  inactivitySetAction: (action: "delete" | "donate") =>
    call<Inactivity>("inactivity_set_action", { action }),

  /** Runs a due sweep to the donation addresses, then clears the wallet.
   *  Re-checked in the core, so it only acts when genuinely owed. */
  inactivitySweep: () => call<SweepResult[]>("inactivity_sweep"),

  /** Where a tip goes, per asset. Fixed and compiled into the program. */
  donationAddresses: () => call<Donation[]>("donation_addresses"),

  /** Whether Tor is installed, running and carrying requests. */
  torState: () => call<TorState>("tor_state"),

  /** Installs, starts and routes through Tor. Slow the first time. */
  torStart: () => call<TorState>("tor_start"),

  /** Stops routing and shuts Tor down. */
  torStop: () => call<TorState>("tor_stop"),

  /** Whether Monero is installed, set up and running. */
  moneroSetupState: () => call<MoneroSetup>("monero_setup_state"),

  /** Installs, starts and configures Monero in one go. Slow the first time:
   *  it downloads about 90 MB. */
  moneroSetupRun: (daemon?: string | null) =>
    call<MoneroSetup>("monero_setup_run", { daemon }),

  /** Stops the Monero daemon this session started. */
  moneroStop: () => call<MoneroSetup>("monero_stop"),

  /** Checks the local Monero wallet daemon is up and holds this account. */
  xmrStatus: (endpoint: string) => call<MoneroStatus>("xmr_status", { endpoint }),

  /** Balance according to the local Monero wallet. */
  xmrBalance: (endpoint: string) => call<MoneroBalance>("xmr_balance", { endpoint }),

  /** Prices a transfer by building it and discarding it. */
  xmrPreview: (endpoint: string, to: string, amountMinor: string) =>
    call<MoneroTransfer>("xmr_preview", { endpoint, to, amountMinor }),

  /** Sends Monero. Irreversible. */
  xmrSend: (endpoint: string, to: string, amountMinor: string) =>
    call<MoneroTransfer>("xmr_send", { endpoint, to, amountMinor }),

  /** Every coin available to spend, for choosing between them. */
  listSpendable: (asset: AssetId) => call<Spendable[]>("list_spendable", { asset }),

  /** A fresh address to hand out, so payments are not all linked. */
  nextReceiveAddress: (asset: AssetId) =>
    call<ReceiveAddress>("next_receive_address", { asset }),

  /** The Monero spend and view keys. Spending authority: handle carefully. */
  revealMoneroKeys: () => call<MoneroKeys>("reveal_monero_keys"),

  /** Builds and simulates a transfer. Broadcasts nothing. `outpoints` picks
   *  exactly which coins to spend; omit it to let the wallet choose. */
  sendPreview: (
    asset: AssetId,
    to: string,
    amountMinor: string,
    outpoints?: string[] | null,
  ) => call<SendQuote>("send_preview", { asset, to, amountMinor, outpoints }),

  /** Signs and broadcasts. Irreversible. Returns the transaction id. */
  sendExecute: (
    asset: AssetId,
    to: string,
    amountMinor: string,
    outpoints?: string[] | null,
  ) => call<string>("send_execute", { asset, to, amountMinor, outpoints }),
};
