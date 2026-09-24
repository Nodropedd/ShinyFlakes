// Typed wrappers over the Tauri IPC bridge.

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

export type NetworkId = "SOL" | "ETH" | "TRON";

export interface UpdateInfo {
  current: string;
  latest: string;

  available: boolean;
}

export interface StaySignedIn {

  enabled: boolean;

  available: boolean;
}

export interface VaultStatus {

  initialized: boolean;

  unlocked: boolean;

  needsPassphrase: boolean;

  keyMissing: boolean;
}

export interface AssetAddress {
  asset: AssetId;

  address: string | null;

  path: string | null;

  host: AssetId | null;

  unsupported: string | null;
}

export interface AssetBalance {
  asset: AssetId;

  minor: string | null;
  error: string | null;

  network?: NetworkId | null;
}

export interface SendQuote {
  asset: AssetId;
  to: string;
  amountMinor: string;
  feeMinor: string;

  creatorFeeMinor: string;
  totalMinor: string;

  simulated: boolean;
}

export interface SendLimits {
  asset: AssetId;
  balanceMinor: string;
  feeMinor: string;

  maxMinor: string;

  rentMinimumMinor: string;
}

export interface Quote {
  price: number;

  change24h: number | null;
}

export interface ActivityEntry {
  asset: AssetId;
  id: string;
  direction: "in" | "out";

  amountMinor: string;

  timestamp: number | null;
  confirmed: boolean;
}

export interface UtxoEntry {
  txid: string;
  vout: number;
  valueMinor: string;

  keyIndex: number;
}

export interface UtxoState {
  asset: AssetId;
  totalMinor: string;
  outputs: UtxoEntry[];

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

  sources: number;
}

export interface Spendable {

  outpoint: string;
  valueMinor: string;
  keyIndex: number;
  address: string;
}

export interface ReceiveAddress {
  asset: AssetId;
  address: string;
  index: number;

  rotates: boolean;
}

export interface MoneroKeys {
  address: string;
  spendKey: string;
  viewKey: string;
  restoreHeightHint: string;
}

export interface MoneroSetup {

  installed: boolean;

  walletExists: boolean;

  running: boolean;
  version: string;
}

export interface MoneroStatus {
  address: string;
  height: number;

  matchesWallet: boolean;
}

export interface MoneroBalance {
  totalMinor: string;

  unlockedMinor: string;
}

export interface MoneroTransfer {
  txHash: string;
  feeMinor: string;
  amountMinor: string;

  creatorFeeMinor: string;
}

export interface Donation {
  asset: AssetId;
  address: string;

  host: AssetId | null;
}

export interface TorState {

  installed: boolean;

  running: boolean;

  routing: boolean;
  version: string;
}

export interface Inactivity {

  lastSeen: number | null;

  months: number;

  action: "delete" | "donate";
  daysSince: number;

  daysRemaining: number | null;

  graceActive: boolean;
  graceDaysRemaining: number | null;

  sweepDue: boolean;

  wiped: boolean;
}

export interface SweepResult {
  asset: AssetId;

  txid: string | null;

  skipped: string | null;

  error: string | null;
}

export interface TwoFactorState {
  enabled: boolean;

  passValid: boolean;
}

export interface TotpSetup {

  secret: string;

  uri: string;
}

export interface VerifyResult {

  status: "ok" | "wrong" | "lockedOut" | "none";

  remaining: number | null;

  lockedUntil: number | null;
}

export interface SwapQuote {
  from: AssetId;
  to: AssetId;
  amountFromMinor: string;

  amountToMinor: string;
  provider: string;
}

export interface SwapTrade {
  id: string;
  from: AssetId;
  to: AssetId;

  depositAddress: string;

  depositMemo: string;

  depositAmountMinor: string;

  payoutAddress: string;
  amountToMinor: string;
  provider: string;
  status: string;
}

export interface StepUpView {

  dormantDays: number;
  largeSend: boolean;

  totpAvailable: boolean;
  largeSendUsd: number;
  largeSendShare: number;
}

export interface Challenge {
  totp: boolean;
  totpDone: boolean;
}

export type GuardedAction = "reveal" | "send" | "dormant";

export interface IpcError {
  kind: string;
  message: string;
}

function asIpcError(e: unknown): IpcError {
  if (typeof e === "object" && e !== null && "kind" in e && "message" in e) {
    return e as IpcError;
  }

  console.error("unexpected IPC failure", e);
  const detail = e instanceof Error ? e.message : String(e);
  return {
    kind: "Unknown",
    message: `The wallet core did not respond (${detail}). Restart the app and try again.`,
  };
}

const QUICK = new Set(["vault_status", "inactivity_check", "auto_unlock", "generate_mnemonic"]);
const QUICK_MS = 8000;

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    const answer = invoke<T>(cmd, args);
    if (!QUICK.has(cmd)) return await answer;
    return await Promise.race([
      answer,
      new Promise<never>((_, reject) => {
        timer = setTimeout(
          () =>
            reject({
              kind: "Timeout",
              message: `The wallet core did not answer "${cmd}" within ${QUICK_MS / 1000} s.`,
            }),
          QUICK_MS,
        );
      }),
    ]);
  } catch (e) {
    throw asIpcError(e);
  } finally {
    clearTimeout(timer);
  }
}

export const ipc = {
  vaultStatus: () => call<VaultStatus>("vault_status"),

  generateMnemonic: () => call<string>("generate_mnemonic"),

  createVault: (mnemonic: string, fresh = false) =>
    call<void>("create_vault", { mnemonic, fresh }),

  unlock: (mnemonic: string, passphrase?: string | null) =>
    call<void>("unlock", { mnemonic, passphrase }),

  setVaultPassphrase: (current: string | null, next: string | null) =>
    call<void>("set_vault_passphrase", { current, next }),

  lock: () => call<void>("lock"),

  autoUnlock: () => call<boolean>("auto_unlock"),

  staySignedInState: () => call<StaySignedIn>("stay_signed_in_state"),

  checkForUpdate: () => call<UpdateInfo>("check_for_update"),

  openDownloadPage: () => call<void>("open_download_page"),

  setStaySignedIn: (on: boolean) => call<StaySignedIn>("set_stay_signed_in", { on }),

  logout: () => call<void>("logout"),

  forgetWallet: () => call<void>("forget_wallet"),

  listAddresses: () => call<AssetAddress[]>("list_addresses"),

  fetchBalances: () => call<AssetBalance[]>("fetch_balances"),

  fetchPrices: (currency: string) =>
    call<Record<string, Quote>>("fetch_prices", { currency }),

  fetchActivity: () => call<ActivityEntry[]>("fetch_activity"),

  utxoState: (asset: AssetId) => call<UtxoState>("utxo_state", { asset }),

  consolidatePreview: (asset: AssetId) =>
    call<SendQuote>("consolidate_preview", { asset }),

  consolidateExecute: (asset: AssetId) =>
    call<string>("consolidate_execute", { asset }),

  fragmentPreview: (asset: AssetId, amountMinor: string, pieces: number) =>
    call<FragmentQuote>("fragment_preview", { asset, amountMinor, pieces }),

  fragmentExecute: (asset: AssetId, amountMinor: string, pieces: number) =>
    call<string>("fragment_execute", { asset, amountMinor, pieces }),

  sendLimits: (asset: AssetId, network?: NetworkId | null) =>
    call<SendLimits>("send_limits", { asset, network }),

  inactivityCheck: () => call<Inactivity>("inactivity_check"),

  inactivitySetMonths: (months: number) =>
    call<Inactivity>("inactivity_set_months", { months }),

  inactivitySetAction: (action: "delete" | "donate") =>
    call<Inactivity>("inactivity_set_action", { action }),

  inactivitySweep: () => call<SweepResult[]>("inactivity_sweep"),

  donationAddresses: () => call<Donation[]>("donation_addresses"),

  torState: () => call<TorState>("tor_state"),

  torStart: () => call<TorState>("tor_start"),

  torStop: () => call<TorState>("tor_stop"),

  moneroSetupState: () => call<MoneroSetup>("monero_setup_state"),

  moneroSetupRun: (daemon?: string | null) =>
    call<MoneroSetup>("monero_setup_run", { daemon }),

  moneroStop: () => call<MoneroSetup>("monero_stop"),

  xmrStatus: (endpoint: string) => call<MoneroStatus>("xmr_status", { endpoint }),

  xmrBalance: (endpoint: string) => call<MoneroBalance>("xmr_balance", { endpoint }),

  xmrPreview: (
    endpoint: string,
    to: string,
    amountMinor: string,
    amountUsd?: number | null,
  ) => call<MoneroTransfer>("xmr_preview", { endpoint, to, amountMinor, amountUsd }),

  xmrSend: (
    endpoint: string,
    to: string,
    amountMinor: string,
    amountUsd?: number | null,
  ) => call<MoneroTransfer>("xmr_send", { endpoint, to, amountMinor, amountUsd }),

  listSpendable: (asset: AssetId) => call<Spendable[]>("list_spendable", { asset }),

  nextReceiveAddress: (asset: AssetId) =>
    call<ReceiveAddress>("next_receive_address", { asset }),

  revealMoneroKeys: () => call<MoneroKeys>("reveal_monero_keys"),

  settingsUnreadable: () => call<boolean>("settings_unreadable"),

  resetSettings: () => call<void>("reset_settings"),

  stepUpSettings: () => call<StepUpView>("step_up_settings"),

  setStepUpSettings: (dormantDays: number, largeSend: boolean) =>
    call<StepUpView>("set_step_up_settings", { dormantDays, largeSend }),

  stepUpChallenge: (action: GuardedAction) =>
    call<Challenge>("step_up_challenge", { action }),

  dormantStepUpPending: () => call<boolean>("dormant_step_up_pending"),

  clearDormantStepUp: () => call<boolean>("clear_dormant_step_up"),

  twoFactorState: () => call<TwoFactorState>("two_factor_state"),

  beginTotpSetup: () => call<TotpSetup>("begin_totp_setup"),

  confirmTotp: (code: string) => call<boolean>("confirm_totp", { code }),

  disableTwoFactor: () => call<void>("disable_two_factor"),

  verify2fa: (code: string) => call<VerifyResult>("verify_2fa", { code }),

  verify2faSeed: (mnemonic: string) =>
    call<boolean>("verify_2fa_seed", { mnemonic }),

  revealSeed: () => call<string>("reveal_seed"),

  swapQuote: (from: AssetId, to: AssetId, amountMinor: string) =>
    call<SwapQuote>("swap_quote", { from, to, amountMinor }),

  swapCreate: (from: AssetId, to: AssetId, amountMinor: string) =>
    call<SwapTrade>("swap_create", { from, to, amountMinor }),

  swapFund: (
    from: AssetId,
    depositAddress: string,
    amountMinor: string,
    memo?: string | null,
    endpoint?: string | null,
  ) => call<string>("swap_fund", { from, depositAddress, amountMinor, memo, endpoint }),

  swapStatus: (id: string) => call<string>("swap_status", { id }),

  sendPreview: (
    asset: AssetId,
    to: string,
    amountMinor: string,
    amountUsd?: number | null,
    outpoints?: string[] | null,
    network?: NetworkId | null,
  ) =>
    call<SendQuote>("send_preview", { asset, to, amountMinor, amountUsd, outpoints, network }),

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
