<script lang="ts">
  import AssetIcon from "../lib/AssetIcon.svelte";
  import { BY_ID, formatAmount } from "../lib/assets";
  import type { ActivityEntry } from "../lib/ipc";
  import { settings } from "../lib/settings.svelte";
  import { wallet } from "../lib/wallet.svelte";

  $effect(() => {
    if (wallet.activity.length === 0) void wallet.loadActivity();
  });

  function dayLabel(ts: number | null) {
    if (ts == null) return "Pending";
    const date = new Date(ts * 1000);
    const today = new Date();
    const yesterday = new Date(today);
    yesterday.setDate(today.getDate() - 1);

    const same = (a: Date, b: Date) => a.toDateString() === b.toDateString();
    if (same(date, today)) return "Today";
    if (same(date, yesterday)) return "Yesterday";

    return date.toLocaleDateString(undefined, {
      day: "numeric",
      month: "short",
      year: date.getFullYear() === today.getFullYear() ? undefined : "numeric",
    });
  }

  function time(ts: number | null) {
    if (ts == null) return "";
    return new Date(ts * 1000).toLocaleTimeString(undefined, {
      hour: "numeric",
      minute: "2-digit",
    });
  }

  function worth(entry: ActivityEntry) {
    const meta = BY_ID[entry.asset];
    const quote = wallet.prices[entry.asset];
    if (!meta || !quote) return null;
    return (Number(entry.amountMinor) / 10 ** meta.decimals) * quote.price;
  }

  // Entries arrive newest first, so grouping in order preserves that.
  const grouped = $derived(
    wallet.activity.reduce<{ label: string; items: ActivityEntry[] }[]>((acc, entry) => {
      const label = dayLabel(entry.timestamp);
      const last = acc[acc.length - 1];
      if (last && last.label === label) last.items.push(entry);
      else acc.push({ label, items: [entry] });
      return acc;
    }, []),
  );
</script>

<div class="view">
  <header>
    <div>
      <h1>Activity</h1>
      <p class="muted">
        Transactions this wallet sent or received, newest first. Monero is
        absent because its history needs a view-key scan, and token transfers
        are not tracked yet.
      </p>
    </div>
    <button
      class="btn"
      onclick={() => wallet.loadActivity()}
      disabled={wallet.activityLoading}
    >
      {wallet.activityLoading ? "Loading" : "Refresh"}
    </button>
  </header>

  {#if wallet.activityError}
    <p class="err">{wallet.activityError}</p>
  {/if}

  {#if wallet.activityLoading && wallet.activity.length === 0}
    <p class="muted">Reading the chains.</p>
  {:else if wallet.activity.length === 0}
    <div class="empty card">
      <h2>Nothing yet</h2>
      <p class="muted">
        No transactions found for these addresses. If you have just sent
        something, give it a moment and refresh.
      </p>
    </div>
  {:else}
    {#each grouped as group (group.label)}
      <section>
        <h2>{group.label}</h2>
        <ul>
          {#each group.items as entry (entry.asset + entry.id)}
            {@const meta = BY_ID[entry.asset]}
            {@const value = worth(entry)}
            <li class="row">
              <span class="icon">
                <AssetIcon asset={meta} size={34} />
                <span class="arrow" class:in={entry.direction === "in"}>
                  {entry.direction === "in" ? "↓" : "↑"}
                </span>
              </span>

              <div class="what">
                <strong class="mono">
                  {entry.direction === "in" ? "+" : "-"}{formatAmount(
                    entry.amountMinor,
                    meta.decimals,
                  )}
                  {meta.ticker}
                </strong>
                <span class="when muted">
                  {time(entry.timestamp)}{entry.confirmed ? "" : " · unconfirmed"}
                </span>
              </div>

              <span class="value" class:in={entry.direction === "in"}>
                {#if value != null}
                  {entry.direction === "in" ? "+" : "-"}{settings.money(value)}
                {:else}
                  &mdash;
                {/if}
              </span>
            </li>
          {/each}
        </ul>
      </section>
    {/each}
  {/if}
</div>

<style>
  .view { display: flex; flex-direction: column; gap: 22px; max-width: 760px; }
  header { display: flex; align-items: flex-start; gap: 20px; }
  header .btn { margin-left: auto; flex: none; }
  h1 { margin: 0 0 6px; font-size: 20px; font-weight: 650; }
  header p { margin: 0; max-width: 64ch; font-size: 13px; }
  .err { margin: 0; color: var(--danger); font-size: 13px; }

  .empty { padding: 26px; max-width: 62ch; }
  .empty h2 { margin: 0 0 8px; font-size: 15px; }
  .empty p { margin: 0; font-size: 13.5px; }

  section h2 {
    margin: 0 0 9px; font-size: 12px; font-weight: 600;
    color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.04em;
  }
  ul {
    display: flex; flex-direction: column; margin: 0; padding: 0; list-style: none;
    border: 1px solid var(--border); border-radius: var(--radius);
    background: var(--card); overflow: hidden;
  }
  .row { display: flex; align-items: center; gap: 14px; padding: 13px 16px; }
  .row + .row { border-top: 1px solid var(--border); }
  .row:hover { background: var(--card-hover); }

  .icon { position: relative; flex: none; display: flex; }
  .arrow {
    position: absolute; right: -3px; bottom: -3px;
    width: 16px; height: 16px; border-radius: 50%;
    display: grid; place-items: center;
    font-size: 10px; font-weight: 700; line-height: 1;
    background: var(--bg-raised); border: 1px solid var(--border);
    color: var(--danger);
  }
  .arrow.in { color: var(--ok); }

  .what { display: flex; flex-direction: column; gap: 2px; }
  .what strong { font-size: 13.5px; font-variant-numeric: tabular-nums; }
  .when { font-size: 11.5px; }

  .value {
    margin-left: auto; font-size: 13.5px; font-weight: 500;
    font-variant-numeric: tabular-nums; color: var(--text);
  }
  .value.in { color: var(--ok); }
</style>
