<script lang="ts">
  // Cross-chain swaps through Trocador. The wallet quotes a rate, creates a
  // trade to get a deposit address, then pays a normal local-signed send to it
  // with no creator fee. The proceeds land at an address this wallet owns.
  // Nothing here custodies funds; Trocador is one more endpoint, Tor-routed
  // like the rest.
  import AssetIcon from "../lib/AssetIcon.svelte";
  import Address from "../lib/Address.svelte";
  import { ASSETS, BY_ID, formatAmount, toMinor } from "../lib/assets";
  import { ipc, type AssetId, type SwapConfig, type SwapQuote, type SwapTrade } from "../lib/ipc";
  import { settings } from "../lib/settings.svelte";
  import { wallet } from "../lib/wallet.svelte";

  let config = $state<SwapConfig | null>(null);

  $effect(() => {
    ipc
      .swapConfig()
      .then((c) => (config = c))
      .catch(() => (config = { configured: false, hasKey: false, markup: 0 }));
  });

  // Only coins this wallet can actually send may be the source. Monero counts
  // only once its daemon is up, since funding it needs the local bridge.
  const FROM_ASSETS = $derived<AssetId[]>([
    "BTC",
    "LTC",
    "ETH",
    "SOL",
    ...(settings.moneroReady ? (["XMR"] as AssetId[]) : []),
  ]);

  let from = $state<AssetId>("SOL");
  let to = $state<AssetId>("LTC");
  let amountInput = $state("");

  // Keep the two sides distinct, and keep `from` within what can be sent.
  $effect(() => {
    if (!FROM_ASSETS.includes(from)) from = FROM_ASSETS[0];
    if (to === from) to = ASSETS.find((a) => a.id !== from)?.id ?? "BTC";
  });

  const fromMeta = $derived(BY_ID[from]);
  const toMeta = $derived(BY_ID[to]);
  const fromBalance = $derived(wallet.balances[from]?.minor ?? null);

  let quote = $state<SwapQuote | null>(null);
  let quoting = $state(false);
  let quoteError = $state<string | null>(null);

  let trade = $state<SwapTrade | null>(null);
  let creating = $state(false);

  let funding = $state(false);
  let txid = $state<string | null>(null);
  let fundError = $state<string | null>(null);

  let statusText = $state<string | null>(null);
  let polling = $state(false);

  // Any change to the terms invalidates a quote or a trade built on the old
  // ones, so a stale deposit address can never be paid.
  function reset() {
    quote = null;
    trade = null;
    quoteError = null;
    fundError = null;
    txid = null;
    statusText = null;
  }

  function amountMinorOrNull(): string | null {
    try {
      return toMinor(amountInput, fromMeta.decimals);
    } catch {
      return null;
    }
  }

  async function getQuote() {
    const minor = amountMinorOrNull();
    if (!minor || minor === "0") {
      quoteError = "Enter an amount to swap.";
      return;
    }
    reset();
    quoting = true;
    try {
      quote = await ipc.swapQuote(from, to, minor);
    } catch (e) {
      quoteError = (e as { message?: string }).message ?? String(e);
    } finally {
      quoting = false;
    }
  }

  async function review() {
    const minor = amountMinorOrNull();
    if (!minor) return;
    creating = true;
    fundError = null;
    try {
      trade = await ipc.swapCreate(from, to, minor);
    } catch (e) {
      quoteError = (e as { message?: string }).message ?? String(e);
    } finally {
      creating = false;
    }
  }

  const memoBlocks = $derived(!!trade && trade.depositMemo.trim().length > 0);

  async function confirmFund() {
    if (!trade || memoBlocks) return;
    funding = true;
    fundError = null;
    try {
      txid = await ipc.swapFund(
        trade.from,
        trade.depositAddress,
        trade.depositAmountMinor,
        trade.depositMemo || null,
        trade.from === "XMR" ? settings.moneroEndpoint : null,
      );
      statusText = trade.status || "waiting";
      void wallet.refresh();
    } catch (e) {
      fundError = (e as { message?: string }).message ?? String(e);
    } finally {
      funding = false;
    }
  }

  // Once funded, poll the trade until it reaches a terminal state.
  const TERMINAL = ["finished", "expired", "failed", "refunded"];

  $effect(() => {
    if (!txid || !trade) return;
    const id = trade.id;
    let stop = false;

    const tick = async () => {
      if (stop) return;
      polling = true;
      try {
        const s = await ipc.swapStatus(id);
        statusText = s;
        if (TERMINAL.includes(s.toLowerCase())) stop = true;
      } catch {
        /* leave the last known status; try again next tick */
      } finally {
        polling = false;
      }
    };

    void tick();
    const t = setInterval(() => {
      if (stop) {
        clearInterval(t);
        return;
      }
      void tick();
    }, 15000);
    return () => {
      stop = true;
      clearInterval(t);
    };
  });

  function startOver() {
    reset();
    amountInput = "";
  }
</script>

<div class="view">
  <header>
    <h1>Swap</h1>
  </header>

  {#if config && !config.configured}
    <section class="card">
      <h2>Connect Trocador</h2>
      <p class="muted">
        Swaps go through Trocador, a non-custodial aggregator: no account, no
        KYC, and your keys never leave this machine. It needs a free API key,
        which also lets you set a markup you earn on every swap.
      </p>
      <p class="hint">
        Get a key at <span class="mono">trocador.app</span>, then add it under
        Settings → Swaps.
      </p>
    </section>
  {:else if txid}
    <section class="card">
      <h2>Swap in progress</h2>
      <p class="muted">
        Your {fromMeta.name} is on its way to the exchange. The
        {toMeta.name} will arrive at your address once it confirms. You can leave
        this screen; the swap continues on the network.
      </p>

      <div class="line">
        <span class="k">Status</span>
        <span class="v">{statusText ?? "waiting"}{polling ? " …" : ""}</span>
      </div>
      <div class="line">
        <span class="k">You sent</span>
        <span class="v mono">{formatAmount(trade!.depositAmountMinor, fromMeta.decimals)} {fromMeta.ticker}</span>
      </div>
      <div class="line">
        <span class="k">Expected</span>
        <span class="v mono">≈ {formatAmount(trade!.amountToMinor, toMeta.decimals)} {toMeta.ticker}</span>
      </div>
      <div class="line">
        <span class="k">Sent tx</span>
        <span class="v"><Address value={txid} /></span>
      </div>
      <div class="line">
        <span class="k">Trade id</span>
        <span class="v mono selectable">{trade!.id}</span>
      </div>

      <p class="hint">
        Keep the trade id. If anything stalls, it is how Trocador looks the swap
        up.
      </p>
      <button class="btn" onclick={startOver}>New swap</button>
    </section>
  {:else if trade}
    <section class="card">
      <h2>Confirm swap</h2>
      <p class="muted">
        Trocador locked in this trade through {trade.provider || "an exchange"}.
        Confirming sends your {fromMeta.name} now. This is a real, irreversible
        transfer.
      </p>

      <div class="line">
        <span class="k">You send</span>
        <span class="v mono">{formatAmount(trade.depositAmountMinor, fromMeta.decimals)} {fromMeta.ticker}</span>
      </div>
      <div class="line">
        <span class="k">You receive</span>
        <span class="v mono">≈ {formatAmount(trade.amountToMinor, toMeta.decimals)} {toMeta.ticker}</span>
      </div>
      <div class="line">
        <span class="k">To your</span>
        <span class="v mono selectable">{trade.payoutAddress.slice(0, 12)}…{trade.payoutAddress.slice(-8)}</span>
      </div>
      <div class="line">
        <span class="k">Deposit to</span>
        <span class="v mono selectable">{trade.depositAddress.slice(0, 12)}…{trade.depositAddress.slice(-8)}</span>
      </div>

      {#if memoBlocks}
        <p class="kerr">
          This route needs a deposit memo this wallet cannot attach. Go back and
          try a different amount or pair.
        </p>
      {/if}
      {#if fundError}
        <p class="kerr">{fundError}</p>
      {/if}

      <div class="crow">
        <button class="btn" onclick={() => (trade = null)} disabled={funding}>Back</button>
        <button
          class="btn btn-primary"
          onclick={confirmFund}
          disabled={funding || memoBlocks}
        >
          {funding ? "Sending" : `Send ${fromMeta.ticker}`}
        </button>
      </div>
    </section>
  {:else}
    <section class="card">
      <h2>Trade one coin for another</h2>
      <p class="muted">
        A rate is fetched from Trocador. Your coin is sent on-chain to the
        exchange, and the other coin comes straight back to your own address.
      </p>

      <div class="swapbox">
        <label class="field">
          <span>From</span>
          <select bind:value={from} onchange={reset}>
            {#each FROM_ASSETS as id (id)}
              <option value={id}>{BY_ID[id].name} ({BY_ID[id].ticker})</option>
            {/each}
          </select>
        </label>

        <label class="field">
          <span>To</span>
          <select bind:value={to} onchange={reset}>
            {#each ASSETS.filter((a) => a.id !== from) as a (a.id)}
              <option value={a.id}>{a.name} ({a.ticker})</option>
            {/each}
          </select>
        </label>
      </div>

      <label class="field amount">
        <span>
          Amount in {fromMeta.ticker}
          {#if fromBalance != null}
            <button
              class="bal"
              onclick={() => {
                amountInput = formatAmount(fromBalance, fromMeta.decimals).replace(/,/g, "");
                reset();
              }}
            >
              balance {formatAmount(fromBalance, fromMeta.decimals)}
            </button>
          {/if}
        </span>
        <input
          class="mono"
          bind:value={amountInput}
          oninput={reset}
          inputmode="decimal"
          placeholder="0.0"
          spellcheck="false"
        />
      </label>

      {#if quoteError}
        <p class="kerr">{quoteError}</p>
      {/if}

      {#if quote}
        <div class="quote">
          <div class="qtop">
            <AssetIcon asset={toMeta} size={26} />
            <span class="qamt mono">
              ≈ {formatAmount(quote.amountToMinor, toMeta.decimals)} {toMeta.ticker}
            </span>
          </div>
          <p class="hint">
            Estimated{quote.provider ? ` via ${quote.provider}` : ""}. The final
            amount can move a little until the trade is locked in.
          </p>
        </div>
      {/if}

      <div class="crow">
        {#if quote}
          <button class="btn" onclick={getQuote} disabled={quoting}>
            {quoting ? "…" : "Refresh"}
          </button>
          <button class="btn btn-primary" onclick={review} disabled={creating}>
            {creating ? "Preparing" : "Review swap"}
          </button>
        {:else}
          <button class="btn btn-primary wide" onclick={getQuote} disabled={quoting}>
            {quoting ? "Getting rate" : "Get quote"}
          </button>
        {/if}
      </div>
    </section>
  {/if}
</div>

<style>
  .view { display: flex; flex-direction: column; gap: 14px; max-width: 560px; }
  h1 { margin: 0; font-size: 20px; font-weight: 650; }
  section { padding: 20px 22px; }
  h2 { margin: 0 0 6px; font-size: 14.5px; font-weight: 600; }
  section p { margin: 0; font-size: 13px; }

  .swapbox { display: flex; gap: 10px; margin-top: 16px; }
  .swapbox .field { flex: 1; }
  .field { display: block; }
  .field span {
    display: flex; align-items: center; justify-content: space-between; gap: 8px;
    margin-bottom: 5px; font-size: 12.5px; color: var(--text-muted);
  }
  .field select, .field input { width: 100%; }
  select {
    background: var(--bg-raised); color: var(--text);
    border: 1px solid var(--border); border-radius: var(--radius-sm);
    padding: 9px 11px; font: inherit;
  }
  .amount { margin-top: 14px; }
  .bal {
    color: var(--text-faint); font-size: 11.5px; text-decoration: underline;
    font-variant-numeric: tabular-nums;
  }
  .bal:hover { color: var(--text); }

  .quote {
    margin-top: 16px; padding: 14px 15px;
    border: 1px solid var(--border); border-radius: var(--radius-sm);
    background: var(--bg-raised);
  }
  .qtop { display: flex; align-items: center; gap: 10px; }
  .qamt { font-size: 16px; font-weight: 600; }

  .line {
    display: flex; align-items: baseline; justify-content: space-between; gap: 12px;
    padding: 8px 0; border-bottom: 1px solid var(--border); font-size: 13px;
  }
  .line:first-of-type { border-top: 1px solid var(--border); margin-top: 14px; }
  .k { color: var(--text-muted); }
  .v { text-align: right; word-break: break-all; }
  .v.mono { font-variant-numeric: tabular-nums; }

  .crow { display: flex; gap: 8px; margin-top: 18px; }
  .crow .btn { flex: 1; }
  .wide { width: 100%; }
  .hint { margin: 12px 0 0 !important; font-size: 12px; color: var(--text-faint); }
  .kerr { margin: 12px 0 0 !important; color: var(--danger); font-size: 12.5px; }
  .card .btn { margin-top: 16px; }
  .crow .btn { margin-top: 0; }
</style>
