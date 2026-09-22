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

/** The chains a stablecoin can live on. Same underlying address as the native
 *  coin of that chain. */
export type NetworkId = "SOL" | "ETH" | "TRON";

/** Whether the app opens straight into the wallet on start. */
export interface StaySignedIn {
  /** The owner turned it on. */
  enabled: boolean;
  /** It can work on this wallet: not with a vault passphrase, which has to be
   *  typed on every open. */
  available: boolean;
}

export interface VaultStatus {
  /** A vault file exists on disk. False means cold start, no wallet yet. */
  initialized: boolean;
  /** Keys are currently derived and held in memory. */
  unlocked: boolean;
  /** Unlocking needs a passphrase on top of the seed phrase. */
  needsPassphrase: boolean;
  /** A vault file exists but its decryption key is gone from the OS
   *  credential store, so it can never be opened here. Restore from seed. */
  keyMissing: boolean;
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
  /** For a token, which network this balance is on. Null for a native coin or
   *  for a token's summed-across-networks total. */
  network?: NetworkId | null;
}

export interface SendQuote {
  asset: AssetId;
  to: string;
  amountMinor: string;
  feeMinor: string;
  /** The creator fee, an extra output to the donation address. */
  creatorFeeMinor: string;
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
  /** The creator fee bundled into the transfer. */
  creatorFeeMinor: string;
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


export interface TwoFactorState {
  enabled: boolean;
  /** A recent code check still authorises a reveal. */
  passValid: boolean;
}

export interface TotpSetup {
  /** The base32 secret, for manual entry into an authenticator app. */
  secret: string;
  /** The otpauth URI to render as a QR code. */
  uri: string;
}

export interface VerifyResult {
  /** ok, wrong, lockedOut, or none. */
  status: "ok" | "wrong" | "lockedOut" | "none";
  /** Tries left before the lockout, when wrong. */
  remaining: number | null;
  /** Unix second the lockout lifts, when locked out. */
  lockedUntil: number | null;
}

export interface SwapQuote {
  from: AssetId;
  to: AssetId;
  amountFromMinor: string;
  /** Expected proceeds. A variable-rate swap can settle a little different. */
  amountToMinor: string;
  provider: string;
}

export interface SwapTrade {
  id: string;
  from: AssetId;
  to: AssetId;
  /** Where the "from" coin must be paid. */
  depositAddress: string;
  /** A tag some chains need; empty for the ones this wallet sends. */
  depositMemo: string;
  /** Exactly how much to pay, in the "from" asset's smallest unit. */
  depositAmountMinor: string;
  /** Where the proceeds land: an address this wallet owns. */
  payoutAddress: string;
  amountToMinor: string;
  provider: string;
  status: string;
}

/** When the wallet asks for the authenticator a second time. */
export interface StepUpView {
  /** Days unopened before the next unlock has to step up. Zero is off. */
  dormantDays: number;
  largeSend: boolean;
  /** Whether the authenticator is set up. Nothing is asked for when it is not:
   *  a gate nobody can pass is a lockout, not a protection. */
  totpAvailable: boolean;
  largeSendUsd: number;
  largeSendShare: number;
}

/** What still has to be produced for a guarded action. */
export interface Challenge {
  totp: boolean;
  totpDone: boolean;
}

/** The actions that can be guarded, as the Rust side names them. */
export type GuardedAction = "reveal" | "send" | "dormant";

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

  /** Drops derived key material, keeps the vault file. Also pauses staying
   *  signed in until the phrase is typed again. */
  lock: () => call<void>("lock"),

  /** On start: opens the wallet without the phrase if the owner asked for
   *  that. False means ask for the phrase as usual. */
  autoUnlock: () => call<boolean>("auto_unlock"),

  staySignedInState: () => call<StaySignedIn>("stay_signed_in_state"),

  /** Turning it on needs two-factor when that is on, like revealing the seed. */
  setStaySignedIn: (on: boolean) => call<StaySignedIn>("set_stay_signed_in", { on }),

  /** Full wipe back to cold start, per the security model. */
  logout: () => call<void>("logout"),

  /** Destroys the wallet stored on this machine. Irreversible without the
   *  seed phrase written down elsewhere. */
  forgetWallet: () => call<void>("forget_wallet"),


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

  /** What the account can afford, so the UI can offer a working maximum. For a
   *  token, `network` picks which chain's balance to read. */
  sendLimits: (asset: AssetId, network?: NetworkId | null) =>
    call<SendLimits>("send_limits", { asset, network }),

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
  xmrPreview: (
    endpoint: string,
    to: string,
    amountMinor: string,
    amountUsd?: number | null,
  ) => call<MoneroTransfer>("xmr_preview", { endpoint, to, amountMinor, amountUsd }),

  /** Sends Monero. Irreversible. */
  xmrSend: (
    endpoint: string,
    to: string,
    amountMinor: string,
    amountUsd?: number | null,
  ) => call<MoneroTransfer>("xmr_send", { endpoint, to, amountMinor, amountUsd }),

  /** Every coin available to spend, for choosing between them. */
  listSpendable: (asset: AssetId) => call<Spendable[]>("list_spendable", { asset }),

  /** A fresh address to hand out, so payments are not all linked. */
  nextReceiveAddress: (asset: AssetId) =>
    call<ReceiveAddress>("next_receive_address", { asset }),

  /** The Monero spend and view keys. Spending authority: handle carefully.
   *  Gated by two-factor when it is on; rejects with TwoFactorRequired. */
  revealMoneroKeys: () => call<MoneroKeys>("reveal_monero_keys"),

  /** Whether the settings file is present but unreadable, so the interface
   *  can offer a reset instead of throwing at every panel. */
  settingsUnreadable: () => call<boolean>("settings_unreadable"),

  /** Puts the unreadable settings aside and starts fresh. Loses the step-up
   *  choices and the TOTP pairing — never the seed. */
  resetSettings: () => call<void>("reset_settings"),

  /** When the wallet asks again. */
  stepUpSettings: () => call<StepUpView>("step_up_settings"),

  setStepUpSettings: (dormantDays: number, largeSend: boolean) =>
    call<StepUpView>("set_step_up_settings", { dormantDays, largeSend }),

  /** What the user still has to produce for a given action. */
  stepUpChallenge: (action: GuardedAction) =>
    call<Challenge>("step_up_challenge", { action }),

  /** Whether this session still owes a factor for returning after a long
   *  silence. Asked right after unlocking. */
  dormantStepUpPending: () => call<boolean>("dormant_step_up_pending"),

  /** Clears that debt once the code has been produced. False means it has
   *  not been. */
  clearDormantStepUp: () => call<boolean>("clear_dormant_step_up"),

  /** Whether two-factor is on, and whether a pass is live now. */
  twoFactorState: () => call<TwoFactorState>("two_factor_state"),

  /** Begins TOTP setup: mints a secret and returns the QR to scan. Nothing is
   *  saved until a code confirms it. */
  beginTotpSetup: () => call<TotpSetup>("begin_totp_setup"),

  /** Confirms setup with a code from the authenticator; turns two-factor on. */
  confirmTotp: (code: string) => call<boolean>("confirm_totp", { code }),

  /** Turns two-factor off and forgets the secret. */
  disableTwoFactor: () => call<void>("disable_two_factor"),

  /** Checks a code from the authenticator, advancing the lockout on wrong. */
  verify2fa: (code: string) => call<VerifyResult>("verify_2fa", { code }),

  /** The seed-phrase fallback when the authenticator is not to hand. */
  verify2faSeed: (mnemonic: string) =>
    call<boolean>("verify_2fa_seed", { mnemonic }),

  /** The seed phrase, for backup. Gated by two-factor when on. */
  revealSeed: () => call<string>("reveal_seed"),

  /** A swap rate. Reaches ChangeNOW; broadcasts nothing. */
  swapQuote: (from: AssetId, to: AssetId, amountMinor: string) =>
    call<SwapQuote>("swap_quote", { from, to, amountMinor }),

  /** Creates a trade: a deposit address and a locked-in amount. */
  swapCreate: (from: AssetId, to: AssetId, amountMinor: string) =>
    call<SwapTrade>("swap_create", { from, to, amountMinor }),

  /** Pays the deposit for a created trade. Irreversible once broadcast. The
   *  endpoint is only used when funding from Monero. */
  swapFund: (
    from: AssetId,
    depositAddress: string,
    amountMinor: string,
    memo?: string | null,
    endpoint?: string | null,
  ) => call<string>("swap_fund", { from, depositAddress, amountMinor, memo, endpoint }),

  /** The current status of a trade, for polling until proceeds arrive. */
  swapStatus: (id: string) => call<string>("swap_status", { id }),

  /** Builds and simulates a transfer. Broadcasts nothing. `amountUsd` sets
   *  the creator-fee tier; `outpoints` picks exactly which coins to spend,
   *  omitted to let the wallet choose. */
  sendPreview: (
    asset: AssetId,
    to: string,
    amountMinor: string,
    amountUsd?: number | null,
    outpoints?: string[] | null,
    network?: NetworkId | null,
  ) =>
    call<SendQuote>("send_preview", { asset, to, amountMinor, amountUsd, outpoints, network }),

  /** Signs and broadcasts. Irreversible. Returns the transaction id. For a
   *  token, `network` picks which chain to send it on. */
  sendExecute: (
    asset: AssetId,
    to: string,
    amountMinor: string,
    amountUsd?: number | null,
    outpoints?: string[] | null,
    network?: NetworkId | null,
  ) =>
    call<string>("send_execute", { asset, to, amountMinor, amountUsd, outpoints, network }),
};
