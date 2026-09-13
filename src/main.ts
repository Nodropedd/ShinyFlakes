import "./app.css";
import { mount } from "svelte";
import App from "./App.svelte";
import { settings } from "./lib/settings.svelte";

// Applied before the first paint so the window never flashes the wrong theme.
settings.apply();

// Dev-only browser preview. Opening the Vite server with ?mock installs a
// stand-in for the Tauri bridge so wallet screens can be styled in a browser
// without a Rust build. It is guarded by import.meta.env.DEV, so Vite drops
// the whole block from a production bundle. It never runs inside the app: a
// real Tauri window already provides the bridge, and the check below leaves
// it alone.
if (import.meta.env.DEV && new URLSearchParams(location.search).has("mock")) {
  const w = window as unknown as Record<string, unknown>;
  if (!w.__TAURI_INTERNALS__) {
    console.warn("mock IPC bridge active: no wallet core, no real data");
    // Preview the connected state without a click, since there is no real
    // network call behind the mock to consent to.
    try {
      localStorage.setItem("shinyflakes.network", "yes");
    } catch {
      /* ignore */
    }
    w.__TAURI_INTERNALS__ = {
      invoke: async (cmd: string, args?: Record<string, unknown>) => {
        switch (cmd) {
          case "vault_status":
            return {
              initialized: true,
              unlocked: true,
              needsPassphrase: false,
              keyMissing: false,
            };
          case "set_vault_passphrase":
            return null;
          case "list_buckets":
            return [];
          case "fetch_balances":
            return [
              { asset: "BTC", minor: "125000", error: null },
              { asset: "LTC", minor: "0", error: null },
              { asset: "XMR", minor: null, error: "Monero cannot be queried by address." },
              { asset: "ETH", minor: "0", error: null },
              { asset: "SOL", minor: "2113349556", error: null },
              { asset: "TRON", minor: "0", error: null },
              { asset: "USDC", minor: "0", error: null },
              { asset: "USDT", minor: "0", error: null },
            ];
          case "utxo_state":
            return {
              asset: "BTC", totalMinor: "2450000", feeRate: 4.2,
              sources: 1, maxPieces: 9, dustMinor: "546",
              outputs: [
                { txid: "9f96ade4b41d5433f4eda31e1738ec2b36f6e7d1420d94a6af99801a88f7f7ff", vout: 0, valueMinor: "1800000", keyIndex: 0 },
                { txid: "8ac60eb9575db5b2d987e29f301b5b819ea83a5c6579d282d189cc04b8e151ef", vout: 1, valueMinor: "650000", keyIndex: 0 },
              ],
            };
          case "consolidate_preview":
            return {
              asset: "BTC", to: "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu",
              amountMinor: "2448600", feeMinor: "1400", creatorFeeMinor: "0",
              totalMinor: "2450000", simulated: false,
            };
          case "fragment_preview":
            return {
              asset: "BTC", pieces: 4, perPieceMinor: "500000",
              amountMinor: "2000000", feeMinor: "1050", changeMinor: "448950",
              inputsUsed: 2, sources: 1,
            };
          case "list_spendable":
            return [
              { outpoint: "9f96ade4b41d5433f4eda31e1738ec2b36f6e7d1420d94a6af99801a88f7f7ff:0", valueMinor: "1800000", keyIndex: 0, address: "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu" },
              { outpoint: "8ac60eb9575db5b2d987e29f301b5b819ea83a5c6579d282d189cc04b8e151ef:1", valueMinor: "650000", keyIndex: 3, address: "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu" },
            ];
          case "next_receive_address":
            return { asset: "BTC", address: "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu", index: 4, rotates: true };
          case "tor_state":
            return { installed: false, running: false, routing: false, version: "15.0.22" };
          case "tor_start":
            return { installed: true, running: true, routing: true, version: "15.0.22" };
          case "tor_stop":
            return { installed: true, running: false, routing: false, version: "15.0.22" };
          case "inactivity_check":
          case "inactivity_set_months":
          case "inactivity_set_action":
            return { lastSeen: Math.floor(Date.now() / 1000) - 86400 * 40, months: 12, action: "delete", daysSince: 40, daysRemaining: 325, graceActive: false, graceDaysRemaining: null, sweepDue: false, wiped: false };
          case "inactivity_sweep":
            return [{ asset: "BTC", txid: "0".repeat(64), skipped: null, error: null }];
          case "donation_addresses":
            return [
              { asset: "BTC", address: "bc1qpy3p4gxa4d3x3w0lryma77qqdrfgwhfqwhw7tn", host: null },
              { asset: "LTC", address: "LVhh3tqqbo7bQmQtCgsbuVbdVvCpKhEpCy", host: null },
              { asset: "XMR", address: "42oUemzbsb9A5fWPhaCaBbKmafMXqyTpSKnE8Rco5iyjQNN7NYmct8CS7HFcA8omm6ABgBzDy2NPQTu1zubFH3UuRLwnNUL", host: null },
              { asset: "ETH", address: "0x7aE8380cF08BD44629d099F05eD85570a8d7B930", host: null },
              { asset: "SOL", address: "7oW2bBM5iU4At2ZBJqv81zG7dV2XGWeBHdDYozdZojnb", host: null },
              { asset: "TRON", address: "TKoYYY3jnZHUJhgS8HodUXKzwhZQjDyeLW", host: null },
              { asset: "USDC", address: "7oW2bBM5iU4At2ZBJqv81zG7dV2XGWeBHdDYozdZojnb", host: "SOL" },
              { asset: "USDT", address: "TKoYYY3jnZHUJhgS8HodUXKzwhZQjDyeLW", host: "TRON" },
            ];
          case "monero_setup_state":
            return { installed: false, walletExists: false, running: false, version: "v0.18.5.1" };
          case "monero_setup_run":
            return { installed: true, walletExists: true, running: true, version: "v0.18.5.1" };
          case "monero_stop":
            return { installed: true, walletExists: true, running: false, version: "v0.18.5.1" };
          case "xmr_status":
            return { address: "43SMrTtLZsyZL81653f6b3BWpU5u6XZ2SRdAaM1MxLCGDcTq6mKi9D11ZgN2hbmCdS9j66xu8Wz3J9wgiwkYssLnEK44756", height: 3421100, matchesWallet: true };
          case "xmr_balance":
            return { totalMinor: "0", unlockedMinor: "0" };
          case "xmr_preview":
          case "xmr_send":
            return { txHash: "0".repeat(64), feeMinor: "30000000", amountMinor: "1000000000", creatorFeeMinor: "10000000" };
          case "reveal_monero_keys":
            return {
              address: "43SMrTtLZsyZL81653f6b3BWpU5u6XZ2SRdAaM1MxLCGDcTq6mKi9D11ZgN2hbmCdS9j66xu8Wz3J9wgiwkYssLnEK44756",
              spendKey: "0".repeat(64), viewKey: "0".repeat(64),
              restoreHeightHint: "the block height when you first received Monero here",
            };
          case "email_config":
          case "set_email_config":
            return {
              configured: true, host: "smtp.gmail.com", port: 587,
              username: "you@gmail.com", from: "you@gmail.com",
              hasPassword: true,
            };
          case "send_test_email":
            return null;
          case "two_factor_state":
            return { enabled: false, passValid: false };
          case "begin_totp_setup":
            return {
              secret: "JBSWY3DPEHPK3PXP",
              uri: "otpauth://totp/ShinyFlakes:wallet?secret=JBSWY3DPEHPK3PXP&issuer=ShinyFlakes&algorithm=SHA1&digits=6&period=30",
            };
          case "confirm_totp":
            return args?.code === "000000";
          case "disable_two_factor":
            return null;
          case "verify_2fa":
            // In the mock, 000000 is the code; anything else is wrong.
            return args?.code === "000000"
              ? { status: "ok", remaining: null, lockedUntil: null }
              : { status: "wrong", remaining: 2, lockedUntil: null };
          case "verify_2fa_seed":
            return true;
          case "reveal_seed":
            return "mock seed phrase not usable as a wallet";
          case "swap_quote":
            return {
              from: args?.from ?? "SOL", to: args?.to ?? "LTC",
              amountFromMinor: args?.amountMinor ?? "1000000000",
              amountToMinor: "205000000", provider: "ChangeNOW",
            };
          case "swap_create":
            return {
              id: "TRX" + "0".repeat(9), from: args?.from ?? "SOL", to: args?.to ?? "LTC",
              depositAddress: "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk",
              depositMemo: "",
              depositAmountMinor: args?.amountMinor ?? "1000000000",
              payoutAddress: "ltc1qjmxnz78nmc8nq77wuxh25n2es7rzm5c2rkk4wh",
              amountToMinor: "205000000", provider: "ChangeNOW", status: "waiting",
            };
          case "swap_fund":
            return "e".repeat(64);
          case "swap_status":
            return "confirming";
          case "send_limits":
            return {
              asset: "SOL", balanceMinor: "9684292", feeMinor: "5000",
              maxMinor: "9679292", rentMinimumMinor: "810624",
            };
          case "send_preview":
            return {
              asset: "SOL", to: "1nc1nerator11111111111111111111111111111111",
              amountMinor: "1000000", feeMinor: "5000", creatorFeeMinor: "10000",
              totalMinor: "1015000", simulated: true,
            };
          case "fetch_activity":
            return [
              { asset: "SOL", id: "5xk2aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", direction: "in", amountMinor: "9679292", timestamp: Math.floor(Date.now() / 1000) - 600, confirmed: true },
              { asset: "SOL", id: "3wnQbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", direction: "out", amountMinor: "9684292", timestamp: Math.floor(Date.now() / 1000) - 1800, confirmed: true },
              { asset: "LTC", id: "0f7ccccccccccccccccccccccccccccccccccccccccc", direction: "out", amountMinor: "9022589", timestamp: Math.floor(Date.now() / 1000) - 500000, confirmed: true },
            ];
          case "fetch_prices":
            return {
              BTC: { price: 79231, change24h: -1.4 },
              LTC: { price: 54.28, change24h: -2.3 },
              XMR: { price: 510.97, change24h: 0.8 },
              ETH: { price: 2410.5, change24h: -0.9 },
              SOL: { price: 104.06, change24h: -1.1 },
              TRON: { price: 0.338623, change24h: 0.2 },
              USDC: { price: 0.999925, change24h: 0 },
              USDT: { price: 0.999824, change24h: 0 },
            };
          case "list_addresses": {
            // The BIP-39 reference mnemonic's addresses. These are published
            // in the specs, publicly known, and must never hold funds. They
            // are here so layout can be checked against real address lengths
            // rather than invented strings.
            const SOL = "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk";
            const TRX = "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH";
            return [
              { asset: "BTC", address: "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu", path: "m/84'/0'/0'/0/0", host: null, unsupported: null },
              { asset: "LTC", address: "ltc1qjmxnz78nmc8nq77wuxh25n2es7rzm5c2rkk4wh", path: "m/84'/2'/0'/0/0", host: null, unsupported: null },
              { asset: "XMR", address: "43SMrTtLZsyZL81653f6b3BWpU5u6XZ2SRdAaM1MxLCGDcTq6mKi9D11ZgN2hbmCdS9j66xu8Wz3J9wgiwkYssLnEK44756", path: "m/44'/128'/0'/0/0", host: null, unsupported: null },
              { asset: "ETH", address: "0x9858EfFD232B4033E47d90003D41EC34EcaEda94", path: "m/44'/60'/0'/0/0", host: null, unsupported: null },
              { asset: "SOL", address: SOL, path: "m/44'/501'/0'/0'", host: null, unsupported: null },
              { asset: "TRON", address: TRX, path: "m/44'/195'/0'/0/0", host: null, unsupported: null },
              { asset: "USDC", address: SOL, path: "m/44'/501'/0'/0'", host: "SOL", unsupported: null },
              { asset: "USDT", address: TRX, path: "m/44'/195'/0'/0/0", host: "TRON", unsupported: null },
            ];
          }
          case "generate_mnemonic":
            return "mock phrase not usable as a seed";
          default:
            return null;
        }
      },
    };
  }
}

// The app never navigates, so the context menu and drag-drop only offer ways
// to leak or alter wallet chrome. Both are off outside text inputs.
window.addEventListener("contextmenu", (e) => {
  if (!(e.target as HTMLElement)?.closest("input, textarea, .selectable")) {
    e.preventDefault();
  }
});
window.addEventListener("dragover", (e) => e.preventDefault());
window.addEventListener("drop", (e) => e.preventDefault());

export default mount(App, { target: document.getElementById("app")! });
