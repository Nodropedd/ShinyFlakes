<script lang="ts">
  import AssetIcon from "../lib/AssetIcon.svelte";
  import CoinDetail from "../lib/CoinDetail.svelte";
  import ReceiveDialog from "../lib/ReceiveDialog.svelte";
  import SendDialog from "../lib/SendDialog.svelte";
  import { ASSETS, formatAmount } from "../lib/assets";
  import type { AssetId } from "../lib/ipc";
  import { settings } from "../lib/settings.svelte";
  import { wallet } from "../lib/wallet.svelte";

  let selected = $state<AssetId | null>(null);
  let receiving = $state(false);
  let sending = $state(false);

  function pct(value: number) {
    return `${value > 0 ? "+" : ""}${value.toFixed(1)}%`;
  }
</script>

{#if selected}
  <CoinDetail
    asset={selected}
    onback={() => (selected = null)}
    onsend={() => (sending = true)}
    onreceive={() => (receiving = true)}
  />
{:else}
  <div class="view">
    <header class="head">
      <div>
        <p class="label">Total balance</p>
        <div class="total-row">
          <p class="total">
            {#if !wallet.anyBalance}
              &mdash;
            {:else}
              {wallet.unpriced > 0 ? "~" : ""}{settings.money(wallet.total)}
            {/if}
          </p>
          {#if wallet.totalChange24h != null}
            {@const move = wallet.totalChange24h}
            <span class="day" class:up={move > 0} class:down={move < 0}>
              {move > 0 ? "+" : ""}{move.toFixed(2)}% today
            </span>
          {/if}
        </div>
        <p class="sub muted">
          {#if !wallet.connected}
            Not connected
          {:else if wallet.lastRefresh}
            Updated {wallet.lastRefresh.toLocaleTimeString()}{wallet.loading
              ? ", refreshing"
              : ""}{wallet.failures ? `, ${wallet.failures} unavailable` : ""}{wallet.unpriced
              ? `, ${wallet.unpriced} unpriced`
              : ""}
          {:else if wallet.loading}
            Loading
          {:else}
            Connected
          {/if}
        </p>
      </div>

      <div class="actions">
        <button class="btn" onclick={() => (receiving = true)}>Receive</button>
        <button class="btn" onclick={() => (sending = true)}>Send</button>
      </div>
    </header>

    {#if !wallet.connected}
      <div class="notice card">
        <p>
          Reading balances means asking public services what your addresses
          hold. They will see those addresses and this machine's IP, and can
          link them together. Nothing is contacted until you say so.
        </p>
        <p class="who muted">
          Bitcoin from mempool.space, Litecoin from litecoinspace.org, Solana
          from the public mainnet RPC, Tron from TronGrid, prices from
          CoinGecko.
        </p>
        <button class="btn btn-primary" onclick={() => wallet.connect()}>
          Connect and load balances
        </button>
      </div>
    {/if}

    {#if wallet.error}
      <p class="err">{wallet.error}</p>
    {/if}

    <section>
      <div class="section-head">
        <h2>Assets</h2>
        <button
          class="denom"
          onclick={() => settings.toggleDenomination()}
          title="Switch between coin amounts and {settings.meta.label}"
        >
          {settings.denominate === "coin" ? "Coin" : settings.meta.code.toUpperCase()}
        </button>
      </div>

      <ul class="assets">
        {#each ASSETS as asset (asset.id)}
          {@const bal = wallet.balances[asset.id]}
          {@const quote = wallet.prices[asset.id]}
          {@const worth = wallet.value(asset.id)}
          {@const move = quote?.change24h ?? null}
          <li>
            <button class="row" onclick={() => (selected = asset.id)}>
              <AssetIcon {asset} size={36} />
              <div class="name">
                <strong>{asset.name}</strong>
                <span class="price muted">
                  {#if quote}
                    {settings.money(quote.price)}
                    {#if move != null}
                      <span class:up={move > 0} class:down={move < 0}>{pct(move)}</span>
                    {/if}
                  {:else}
                    {asset.ticker}
                  {/if}
                </span>
              </div>

              <div class="amounts">
                {#if bal?.minor != null}
                  {#if settings.denominate === "coin"}
                    <span class="qty mono" class:stale={bal.error} title={bal.error ?? ""}>
                      {formatAmount(bal.minor, asset.decimals)}
                      {asset.ticker}
                    </span>
                    <span class="worth muted">
                      {worth != null ? settings.money(worth) : "—"}
                    </span>
                  {:else}
                    <span class="qty">
                      {worth != null ? settings.money(worth) : "—"}
                    </span>
                    <span class="worth muted mono">
                      {formatAmount(bal.minor, asset.decimals)}
                      {asset.ticker}
                    </span>
                  {/if}
                {:else if bal?.error}
                  <span class="qty unread" title={bal.error}>Unavailable</span>
                  <span class="worth muted">&mdash;</span>
                {:else}
                  <span class="qty muted">&mdash;</span>
                  <span class="worth">&nbsp;</span>
                {/if}
              </div>
            </button>
          </li>
        {/each}
      </ul>
    </section>
  </div>
{/if}

{#if receiving}
  <ReceiveDialog initial={selected} onclose={() => (receiving = false)} />
{/if}

{#if sending}
  <SendDialog
    initial={selected}
    onclose={() => (sending = false)}
    onsent={() => void wallet.refresh()}
  />
{/if}

<style>
  .view { display: flex; flex-direction: column; gap: 30px; }
  .head {
    display: flex; align-items: flex-start; justify-content: space-between;
    gap: 24px; flex-wrap: wrap;
  }
  .label { margin: 0; color: var(--text-muted); font-size: 13px; }
  .total-row { display: flex; align-items: baseline; gap: 12px; margin: 5px 0 4px; }
  .total {
    margin: 0; font-size: 46px; font-weight: 650;
    letter-spacing: -0.03em; font-variant-numeric: tabular-nums;
  }
  .day { font-size: 14px; font-weight: 600; }
  .sub { margin: 0; font-size: 12.5px; }
  .actions { display: flex; gap: 8px; padding-top: 8px; }
  .actions .btn { min-width: 104px; padding: 11px 20px; }

  .notice {
    padding: 16px 18px;
    border-color: color-mix(in srgb, var(--warn) 26%, var(--border));
    background: color-mix(in srgb, var(--warn) 7%, var(--card));
  }
  .notice p { margin: 0 0 8px; font-size: 13px; max-width: 78ch; }
  .notice .who { font-size: 12px; margin-bottom: 14px; }
  .err { margin: 0; color: var(--danger); font-size: 13px; }

  .section-head {
    display: flex; align-items: center; justify-content: space-between;
    margin-bottom: 10px;
  }
  h2 { margin: 0; font-size: 13px; font-weight: 600; color: var(--text-muted); }
  .denom {
    padding: 4px 11px; border-radius: 999px;
    border: 1px solid var(--border-strong);
    color: var(--text-muted); font-size: 11.5px; font-weight: 600;
    letter-spacing: 0.02em;
  }
  .denom:hover { color: var(--text); background: var(--card); }

  .assets {
    display: flex; flex-direction: column; gap: 9px;
    margin: 0; padding: 0; list-style: none;
  }
  .row {
    display: flex; align-items: center; gap: 15px; width: 100%;
    padding: 17px 18px; text-align: left;
    background: var(--card); border: 1px solid var(--border);
    border-radius: var(--radius);
    transition: background 110ms var(--ease), border-color 110ms var(--ease);
  }
  .row:hover { background: var(--card-hover); border-color: var(--border-strong); }

  .name { display: flex; flex-direction: column; gap: 3px; min-width: 0; }
  .name strong { font-size: 14px; }
  .price { font-size: 12px; display: flex; gap: 7px; align-items: baseline; }
  .up { color: var(--ok); }
  .down { color: var(--danger); }

  .amounts {
    display: flex; flex-direction: column; align-items: flex-end; gap: 3px;
    margin-left: auto; padding-left: 12px;
  }
  .qty { font-size: 14px; font-weight: 500; font-variant-numeric: tabular-nums; }
  .qty.unread { color: var(--text-faint); font-size: 12.5px; cursor: help; }

  .qty.stale { color: var(--text-muted); cursor: help; }
  .worth { font-size: 12px; font-variant-numeric: tabular-nums; }
</style>
