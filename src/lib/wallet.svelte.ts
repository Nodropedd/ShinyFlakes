// Live wallet data shared by every screen: addresses, balances and prices.
//
// Addresses come from the seed and cost nothing, so they load on unlock.
// Balances and prices reach third parties, so they wait for consent and then
// refresh on a timer.

import { ASSETS } from "./assets";
import {
  ipc,
  type ActivityEntry,
  type AssetAddress,
  type AssetBalance,
  type AssetId,
  type NetworkId,
  type Quote,
} from "./ipc";
import type { CurrencyCode } from "./settings.svelte";
import { settings } from "./settings.svelte";

const CONSENT_KEY = "shinyflakes.network";

/** Balances are keyed by asset, except a token's per-network entries, which
 *  add the network so they sit alongside the summed total (keyed by asset). */
function balanceKey(e: { asset: string; network?: NetworkId | null }): string {
  return e.network ? `${e.asset}:${e.network}` : e.asset;
}

// Refresh on a jittered interval rather than a fixed beat. A request landing
// at a public endpoint every exact 60 seconds is itself a fingerprint that
// ties separate lookups to one wallet; a random gap in a range breaks that
// regularity without polling so often it becomes a burden.
const REFRESH_MIN_MS = 45_000;
const REFRESH_MAX_MS = 90_000;

function nextDelay() {
  return REFRESH_MIN_MS + Math.random() * (REFRESH_MAX_MS - REFRESH_MIN_MS);
}

function stored(key: string) {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

class Wallet {
  addresses = $state<Record<string, AssetAddress>>({});
  balances = $state<Record<string, AssetBalance>>({});
  prices = $state<Record<string, Quote>>({});
  activity = $state<ActivityEntry[]>([]);
  activityLoading = $state(false);
  activityError = $state<string | null>(null);

  /** Monero spendable balance, which lags the total while change matures. */
  moneroUnlocked = $state<string | null>(null);

  moneroRunning = $state(false);
  moneroStarting = $state(false);
  moneroError = $state<string | null>(null);

  torRouting = $state(false);
  torStarting = $state(false);
  torError = $state<string | null>(null);

  connected = $state(false);
  loading = $state(false);
  error = $state<string | null>(null);
  lastRefresh = $state<Date | null>(null);

  #timer: ReturnType<typeof setTimeout> | undefined;

  async loadAddresses() {
    try {
      const list = await ipc.listAddresses();
      this.addresses = Object.fromEntries(list.map((a) => [a.asset, a]));
    } catch (e) {
      this.error = (e as { message?: string }).message ?? String(e);
    }
  }

  /** Called once on unlock. Starts polling only if consent was already given. */
  start() {
    void this.loadAddresses();

    // Monero was left switched on, so bring its daemon back up rather than
    // showing a connection error for something the user already set up.
    // Stopping it clears the endpoint, so a deliberate stop stays stopped.
    if (settings.moneroReady) {
      void this.startMonero();
    }

    // Bring Tor up before the first lookup, so balances are never fetched in
    // the clear on a launch where routing was meant to be on.
    void this.#beginNetwork();
  }

  async #beginNetwork() {
    if (settings.torEnabled) {
      await this.startTor();
    }
    if (stored(CONSENT_KEY) === "yes") {
      this.connected = true;
      await this.refresh();
      this.#schedule();
    }
  }

  /** Brings Tor up and routes through it. Remembered across launches. */
  async startTor() {
    if (this.torStarting) return;
    this.torStarting = true;
    this.torError = null;
    try {
      const s = await ipc.torStart();
      this.torRouting = s.routing;
      settings.setTorEnabled(true);
      if (s.routing && this.connected) await this.refresh();
    } catch (e) {
      this.torRouting = false;
      this.torError = (e as { message?: string }).message ?? String(e);
    } finally {
      this.torStarting = false;
    }
  }

  async stopTor() {
    try {
      const s = await ipc.torStop();
      this.torRouting = s.routing;
    } catch (e) {
      this.torError = (e as { message?: string }).message ?? String(e);
    }
    settings.setTorEnabled(false);
  }

  /** Brings the Monero daemon up, adopting one that is already running. */
  async startMonero() {
    if (this.moneroStarting) return;
    this.moneroStarting = true;
    this.moneroError = null;
    try {
      const result = await ipc.moneroSetupRun(settings.moneroDaemon);
      this.moneroRunning = result.running;
      if (result.running && this.connected) await this.refresh();
    } catch (e) {
      this.moneroRunning = false;
      this.moneroError = (e as { message?: string }).message ?? String(e);
    } finally {
      this.moneroStarting = false;
    }
  }

  async stopMonero() {
    try {
      const result = await ipc.moneroStop();
      this.moneroRunning = result.running;
    } catch (e) {
      this.moneroError = (e as { message?: string }).message ?? String(e);
    }
    settings.setMoneroEndpoint("");
    this.moneroUnlocked = null;
  }

  stop() {
    clearTimeout(this.#timer);
    this.#timer = undefined;
  }

  // A self-rescheduling timeout rather than a fixed interval, so each gap is
  // drawn fresh. The refresh runs, then the next one is booked.
  #schedule() {
    clearTimeout(this.#timer);
    this.#timer = setTimeout(() => {
      void this.refresh().finally(() => {
        if (this.connected) this.#schedule();
      });
    }, nextDelay());
  }

  // Prices are quoted in one currency, so switching invalidates them all.
  // Clearing first stops the old numbers being relabelled with the new
  // symbol for the second or two before the refetch lands.
  async setCurrency(code: CurrencyCode) {
    this.prices = {};
    settings.setCurrency(code);
    await this.refresh();
  }

  async loadActivity() {
    if (this.activityLoading) return;
    this.activityLoading = true;
    this.activityError = null;
    try {
      this.activity = await ipc.fetchActivity();
    } catch (e) {
      this.activityError = (e as { message?: string }).message ?? String(e);
    } finally {
      this.activityLoading = false;
    }
  }

  async connect() {
    try {
      localStorage.setItem(CONSENT_KEY, "yes");
    } catch {
      /* the choice simply will not persist */
    }
    this.connected = true;
    await this.refresh();
    this.#schedule();
  }

  async refresh() {
    if (this.loading || !this.connected) return;
    this.loading = true;
    this.error = null;

    // Settled, not all: a price outage must not hide balances.
    const [b, p] = await Promise.allSettled([
      ipc.fetchBalances(),
      ipc.fetchPrices(settings.currency),
    ]);

    if (b.status === "fulfilled") {
      // A chain that failed this time keeps whatever it reported last time.
      // Replacing a real balance with "Unavailable" because one request
      // dropped is worse than showing a number that is a minute old, so the
      // error is recorded alongside the value rather than instead of it.
      const next: Record<string, AssetBalance> = {};
      for (const entry of b.value) {
        const key = balanceKey(entry);
        const previous = this.balances[key];
        if (entry.minor != null || previous?.minor == null) {
          next[key] = entry;
        } else {
          next[key] = { ...previous, error: entry.error };
        }
      }
      this.balances = next;
    } else {
      this.error = (b.reason as { message?: string }).message ?? String(b.reason);
    }
    if (p.status === "fulfilled") this.prices = p.value;

    // Monero cannot be read from an address, so it comes from the local
    // wallet daemon instead, when one has been configured.
    //
    // Skipped while the daemon is still coming up: asking too early would
    // report a connection failure for something that is simply not ready.
    if (settings.moneroReady && !this.moneroStarting) {
      try {
        const xmr = await ipc.xmrBalance(settings.moneroEndpoint);
        this.balances = {
          ...this.balances,
          XMR: { asset: "XMR", minor: xmr.totalMinor, error: null },
        };
        this.moneroUnlocked = xmr.unlockedMinor;
      } catch (e) {
        this.moneroUnlocked = null;
        this.balances = {
          ...this.balances,
          XMR: {
            asset: "XMR",
            minor: this.balances.XMR?.minor ?? null,
            error: (e as { message?: string }).message ?? String(e),
          },
        };
      }
    }

    this.lastRefresh = new Date();
    this.loading = false;
  }

  /** Whole units as a float, for cash conversion only. Displayed amounts are
   *  rendered from the decimal string so they stay exact. */
  #whole(minor: string, decimals: number) {
    return Number(minor) / 10 ** decimals;
  }

  value(asset: AssetId): number | null {
    const minor = this.balances[asset]?.minor;
    const quote = this.prices[asset];
    const meta = ASSETS.find((a) => a.id === asset);
    if (minor == null || quote == null || !meta) return null;
    return this.#whole(minor, meta.decimals) * quote.price;
  }

  /** A token's per-network balances, in display order. Empty for a native
   *  coin. The main `balances[asset]` entry holds the summed total. */
  networks(asset: AssetId): AssetBalance[] {
    const order: NetworkId[] = ["SOL", "ETH", "TRON"];
    return order
      .map((n) => this.balances[`${asset}:${n}`])
      .filter((b): b is AssetBalance => b != null);
  }

  get total() {
    return ASSETS.reduce((sum, a) => sum + (this.value(a.id) ?? 0), 0);
  }

  // Portfolio move over the day, weighted by what each holding is worth.
  // A coin you hold none of cannot swing the number.
  get totalChange24h(): number | null {
    let weighted = 0;
    let base = 0;
    for (const a of ASSETS) {
      const worth = this.value(a.id);
      const move = this.prices[a.id]?.change24h;
      if (worth == null || worth === 0 || move == null) continue;
      weighted += worth * move;
      base += worth;
    }
    return base === 0 ? null : weighted / base;
  }

  /** Assets holding something, which is all the send screen should offer. */
  get funded() {
    return ASSETS.filter((a) => {
      const minor = this.balances[a.id]?.minor;
      return minor != null && minor !== "0";
    });
  }

  get anyBalance() {
    return ASSETS.some((a) => this.balances[a.id]?.minor != null);
  }

  /** Balances read but priced at nothing, so quietly missing from the total. */
  get unpriced() {
    return ASSETS.filter(
      (a) => this.balances[a.id]?.minor != null && this.prices[a.id] == null,
    ).length;
  }

  // Chains that could not be read at all, so have nothing to show.
  get failures() {
    return ASSETS.filter(
      (a) => this.balances[a.id]?.error && this.balances[a.id]?.minor == null,
    ).length;
  }

  // Chains showing a value from an earlier refresh because the latest one
  // failed.
  get stale() {
    return ASSETS.filter(
      (a) => this.balances[a.id]?.error && this.balances[a.id]?.minor != null,
    ).length;
  }
}

export const wallet = new Wallet();
