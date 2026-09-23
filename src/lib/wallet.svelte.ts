// Wallet state and startup.

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

function balanceKey(e: { asset: string; network?: NetworkId | null }): string {
  return e.network ? `${e.asset}:${e.network}` : e.asset;
}

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

  start() {
    void this.loadAddresses();

    if (settings.moneroReady) {
      void this.startMonero();
    }

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

  #schedule() {
    clearTimeout(this.#timer);
    this.#timer = setTimeout(() => {
      void this.refresh().finally(() => {
        if (this.connected) this.#schedule();
      });
    }, nextDelay());
  }

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

    }
    this.connected = true;
    await this.refresh();
    this.#schedule();
  }

  async refresh() {
    if (this.loading || !this.connected) return;
    this.loading = true;
    this.error = null;

    const [b, p] = await Promise.allSettled([
      ipc.fetchBalances(),
      ipc.fetchPrices(settings.currency),
    ]);

    if (b.status === "fulfilled") {

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
    void this.loadAddresses();

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

  networks(asset: AssetId): AssetBalance[] {
    const order: NetworkId[] = ["SOL", "ETH", "TRON"];
    return order
      .map((n) => this.balances[`${asset}:${n}`])
      .filter((b): b is AssetBalance => b != null);
  }

  get total() {
    return ASSETS.reduce((sum, a) => sum + (this.value(a.id) ?? 0), 0);
  }

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

  get funded() {
    return ASSETS.filter((a) => {
      const minor = this.balances[a.id]?.minor;
      return minor != null && minor !== "0";
    });
  }

  get anyBalance() {
    return ASSETS.some((a) => this.balances[a.id]?.minor != null);
  }

  get unpriced() {
    return ASSETS.filter(
      (a) => this.balances[a.id]?.minor != null && this.prices[a.id] == null,
    ).length;
  }

  get failures() {
    return ASSETS.filter(
      (a) => this.balances[a.id]?.error && this.balances[a.id]?.minor == null,
    ).length;
  }

  get stale() {
    return ASSETS.filter(
      (a) => this.balances[a.id]?.error && this.balances[a.id]?.minor != null,
    ).length;
  }
}

export const wallet = new Wallet();
