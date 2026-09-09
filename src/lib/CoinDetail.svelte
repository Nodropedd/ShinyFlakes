<script lang="ts">
  import Address from "./Address.svelte";
  import AssetIcon from "./AssetIcon.svelte";
  import { BY_ID, formatAmount } from "./assets";
  import type { AssetId } from "./ipc";
  import { settings } from "./settings.svelte";
  import { wallet } from "./wallet.svelte";

  let {
    asset,
    onback,
    onsend,
    onreceive,
  }: {
    asset: AssetId;
    onback: () => void;
    onsend: () => void;
    onreceive: () => void;
  } = $props();

  const meta = $derived(BY_ID[asset]);
  const bal = $derived(wallet.balances[asset]);
  const quote = $derived(wallet.prices[asset]);
  const entry = $derived(wallet.addresses[asset]);
  const worth = $derived(wallet.value(asset));
  const move = $derived(quote?.change24h ?? null);
</script>

<div class="view">
  <button class="back" onclick={onback}>
    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor"
         stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
      <path d="M15 5l-7 7 7 7" />
    </svg>
    Portfolio
  </button>

  <header class="head">
    <AssetIcon asset={meta} size={44} />
    <div>
      <div class="title">
        <h1>{meta.name}</h1>
        {#if move != null}
          <span class="move" class:up={move > 0} class:down={move < 0}>
            {move > 0 ? "+" : ""}{move.toFixed(1)}%
          </span>
        {/if}
      </div>
      <p class="price">
        {quote ? settings.money(quote.price) : "Price unavailable"}
      </p>
    </div>
  </header>

  <div class="holding card">
    <div>
      <p class="label">Balance</p>
      <p class="amount mono">
        {#if bal?.minor != null}
          {formatAmount(bal.minor, meta.decimals)}
          {meta.ticker}
        {:else}
          &mdash;
        {/if}
      </p>
    </div>
    <div class="right">
      <p class="label">Value</p>
      <p class="amount">{worth != null ? settings.money(worth) : "—"}</p>
    </div>
  </div>

  {#if bal?.error}
    <p class="note">{bal.error}</p>
  {/if}

  <div class="actions">
    <button class="btn" onclick={onreceive}>Receive</button>
    <button class="btn" onclick={onsend}>Send</button>
  </div>

  <section>
    <h2>Receiving address</h2>
    {#if entry?.address}
      <div class="addr card">
        <span class="mono selectable full">{entry.address}</span>
        <Address value={entry.address} path={entry.path} />
      </div>
      {#if entry.host}
        <p class="note">
          {meta.name} is a token on {entry.host}, so it shares that chain's address.
        </p>
      {/if}
    {:else}
      <p class="note">{entry?.unsupported ?? "No address derived."}</p>
    {/if}
  </section>
</div>

<style>
  .view { display: flex; flex-direction: column; gap: 20px; max-width: 720px; }
  .back {
    display: inline-flex; align-items: center; gap: 6px;
    color: var(--text-muted); font-size: 13px; padding: 0;
    align-self: flex-start;
  }
  .back:hover { color: var(--text); }

  .head { display: flex; align-items: center; gap: 15px; }
  .title { display: flex; align-items: baseline; gap: 10px; }
  h1 { margin: 0; font-size: 21px; font-weight: 650; }
  .move { font-size: 13px; font-weight: 600; }
  .up { color: var(--ok); }
  .down { color: var(--danger); }
  .price {
    margin: 3px 0 0; font-size: 15px; color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .holding { display: flex; gap: 24px; padding: 18px 20px; }
  .holding .right { margin-left: auto; text-align: right; }
  .label { margin: 0 0 4px; font-size: 12px; color: var(--text-muted); }
  .amount { margin: 0; font-size: 20px; font-weight: 600; font-variant-numeric: tabular-nums; }

  .actions { display: flex; gap: 8px; }
  .actions .btn { min-width: 110px; }

  h2 { margin: 0 0 9px; font-size: 12.5px; font-weight: 600; color: var(--text-muted); }
  .addr {
    display: flex; align-items: center; gap: 12px;
    padding: 12px 14px;
  }
  .full { font-size: 12.5px; word-break: break-all; }
  .note { margin: 8px 0 0; font-size: 12px; color: var(--text-faint); max-width: 62ch; }
</style>
