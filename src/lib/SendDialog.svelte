<script lang="ts">
  import { untrack } from "svelte";

  import AssetIcon from "./AssetIcon.svelte";
  import { BY_ID, formatAmount, toMinor } from "./assets";
  import {
    ipc,
    type AssetId,
    type SendLimits,
    type SendQuote,
    type Spendable,
  } from "./ipc";
  import { settings } from "./settings.svelte";
  import { wallet } from "./wallet.svelte";

  let {
    initial,
    onclose,
    onsent,
  }: { initial: AssetId | null; onclose: () => void; onsent: () => void } = $props();

  // Chains whose signing is implemented. The rest appear but cannot be
  // picked, so the gap is visible rather than hidden behind an empty list.
  // Monero only becomes sendable once the local wallet daemon is configured,
  // because the signing happens there rather than here.
  const SENDABLE = $derived<AssetId[]>(
    settings.moneroReady
      ? ["SOL", "BTC", "LTC", "ETH", "XMR"]
      : ["SOL", "BTC", "LTC", "ETH"],
  );

  // Mounted fresh on each open, so this is a starting point rather than a
  // binding. Skipping straight to the form only makes sense when the asset
  // can actually be sent.
  let chosen = $state<AssetId | null>(
    untrack(() => (initial && SENDABLE.includes(initial) ? initial : null)),
  );
  let to = $state("");
  let amount = $state("");

  // Coin control. Only Bitcoin-style chains hold discrete outputs to choose
  // between; account chains have a single balance and nothing to pick.
  const HAS_COINS: AssetId[] = ["BTC", "LTC"];

  let coins = $state<Spendable[]>([]);
  let picked = $state<Set<string>>(new Set());
  let showCoins = $state(false);

  let limits = $state<SendLimits | null>(null);
  let quote = $state<SendQuote | null>(null);
  let signature = $state<string | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);

  const meta = $derived(chosen ? BY_ID[chosen] : null);
  const price = $derived(chosen ? (wallet.prices[chosen]?.price ?? null) : null);

  // The amount box can take either the coin or a cash figure. Whichever is
  // typed, the transaction is always built from a coin amount, so the cash
  // case is converted here and the result shown before anything is signed.
  let denom = $state<"coin" | "fiat">("coin");

  const coinAmount = $derived.by(() => {
    const typed = amount.trim();
    if (denom === "coin" || typed === "") return typed;

    const value = Number(typed);
    if (price == null || price <= 0 || !Number.isFinite(value)) return "";
    // Eight places is past the point where a cash-entered amount means
    // anything, and keeps the conversion clear of floating point noise.
    return (value / price).toFixed(Math.min(meta?.decimals ?? 8, 8));
  });

  const converted = $derived.by(() => {
    if (!meta || price == null) return null;
    const coins = Number(coinAmount);
    if (!Number.isFinite(coins) || coins <= 0) return null;
    return denom === "coin" ? coins * price : coins;
  });

  const ready = $derived(
    to.trim().length > 0 && coinAmount !== "" && Number(coinAmount) > 0,
  );

  function swapDenomination() {
    if (!meta) return;
    // Carry the typed value across rather than clearing it.
    if (price != null && price > 0 && amount.trim() !== "") {
      const value = Number(amount);
      if (Number.isFinite(value)) {
        amount =
          denom === "coin"
            ? (value * price).toFixed(2)
            : (value / price).toFixed(Math.min(meta.decimals, 8));
      }
    }
    denom = denom === "coin" ? "fiat" : "coin";
    reset();
  }

  // Only what the wallet actually holds is worth offering.
  const options = $derived(wallet.funded);

  const pickedCoins = $derived(coins.filter((c) => picked.has(c.outpoint)));

  const pickedTotal = $derived(
    pickedCoins.reduce((sum, c) => sum + BigInt(c.valueMinor), 0n).toString(),
  );

  // Spending outputs from two sub-wallets in one transaction proves on chain
  // that the same person owns both. Worth saying before, not after.
  const wouldLink = $derived(
    new Set(pickedCoins.map((c) => c.keyIndex)).size > 1,
  );

  function toggleCoin(outpoint: string) {
    const next = new Set(picked);
    if (next.has(outpoint)) next.delete(outpoint);
    else next.add(outpoint);
    picked = next;
    reset();
  }

  $effect(() => {
    const current = chosen;
    coins = [];
    picked = new Set();
    showCoins = false;
    if (!current || !HAS_COINS.includes(current)) return;

    ipc
      .listSpendable(current)
      .then((list) => {
        if (chosen === current) coins = list;
      })
      .catch(() => {
        /* choosing coins is optional; the wallet will select for you */
      });
  });

  $effect(() => {
    const current = chosen;
    limits = null;
    if (!current || !SENDABLE.includes(current)) return;
    ipc
      .sendLimits(current)
      .then((l) => {
        if (chosen === current) limits = l;
      })
      .catch(() => {
        /* the preview reports the real problem */
      });
  });

  function reset() {
    quote = null;
    signature = null;
    error = null;
  }

  /** Plain decimal for an input box: no grouping separators. */
  function plain(minor: string, decimals: number) {
    return formatAmount(minor, decimals).replace(/,/g, "");
  }

  function useMax() {
    if (!limits || !meta) return;
    const coins = plain(limits.maxMinor, meta.decimals);
    amount =
      denom === "coin" || price == null
        ? coins
        : (Number(coins) * price).toFixed(2);
    reset();
  }

  async function preview() {
    if (!ready || busy || !chosen || !meta) return;
    busy = true;
    reset();
    try {
      const minor = toMinor(coinAmount, meta.decimals);

      if (chosen === "XMR") {
        // The Monero wallet prices a transfer by building the real thing and
        // discarding it, so the fee here is the actual fee.
        const priced = await ipc.xmrPreview(settings.moneroEndpoint, to.trim(), minor);
        quote = {
          asset: "XMR",
          to: to.trim(),
          amountMinor: priced.amountMinor,
          feeMinor: priced.feeMinor,
          totalMinor: (BigInt(priced.amountMinor) + BigInt(priced.feeMinor)).toString(),
          simulated: false,
        };
      } else {
        quote = await ipc.sendPreview(
          chosen,
          to.trim(),
          minor,
          pickedCoins.length > 0 ? pickedCoins.map((c) => c.outpoint) : null,
        );
      }
    } catch (e) {
      error = (e as { message?: string }).message ?? String(e);
    } finally {
      busy = false;
    }
  }

  async function confirm() {
    if (!quote || busy || !chosen) return;
    busy = true;
    error = null;
    try {
      if (chosen === "XMR") {
        const sent = await ipc.xmrSend(settings.moneroEndpoint, quote.to, quote.amountMinor);
        signature = sent.txHash;
      } else {
        signature = await ipc.sendExecute(
          chosen,
          quote.to,
          quote.amountMinor,
          pickedCoins.length > 0 ? pickedCoins.map((c) => c.outpoint) : null,
        );
      }
      onsent();
    } catch (e) {
      error = (e as { message?: string }).message ?? String(e);
    } finally {
      busy = false;
    }
  }
</script>

<div
  class="scrim"
  role="button"
  tabindex="-1"
  onclick={onclose}
  onkeydown={(e) => e.key === "Escape" && onclose()}
>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="panel card" onclick={(e) => e.stopPropagation()}>
    <header>
      {#if chosen && !signature}
        <button
          class="back"
          onclick={() => {
            chosen = null;
            reset();
          }}
          aria-label="Back"
        >
          <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor"
               stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
            <path d="M15 5l-7 7 7 7" />
          </svg>
        </button>
      {/if}
      <h2>{chosen ? `Send ${meta?.name}` : "Select asset"}</h2>
      <button class="x" onclick={onclose} aria-label="Close">&times;</button>
    </header>

    {#if signature}
      <p class="ok">Sent.</p>
      <p class="label">Transaction id</p>
      <p class="mono wrap selectable">{signature}</p>
      <button class="btn btn-primary wide" onclick={onclose}>Done</button>
    {:else if !chosen}
      {#if options.length === 0}
        <p class="note">Nothing to send. Every balance is zero or unread.</p>
      {:else}
        <ul class="list">
          {#each options as asset (asset.id)}
            {@const bal = wallet.balances[asset.id]}
            {@const worth = wallet.value(asset.id)}
            {@const can = SENDABLE.includes(asset.id)}
            <li>
              <button
                class="row"
                disabled={!can}
                title={can ? "" : "Sending this chain is not implemented yet"}
                onclick={() => (chosen = asset.id)}
              >
                <AssetIcon {asset} size={32} />
                <span class="name">
                  {asset.name}
                  {#if !can}<span class="soon">not implemented</span>{/if}
                </span>
                <span class="held">
                  <span class="mono"
                    >{formatAmount(bal!.minor!, asset.decimals)} {asset.ticker}</span
                  >
                  <span class="muted">{worth != null ? settings.money(worth) : ""}</span>
                </span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    {:else}
      <label class="field">
        <span>Recipient address</span>
        <input class="mono" bind:value={to} oninput={reset} spellcheck="false" />
      </label>

      <label class="field">
        <span class="amount-label">
          <span class="unit">
            Amount in
            <button class="swap" onclick={swapDenomination} type="button" disabled={price == null}>
              {denom === "coin" ? meta?.ticker : settings.meta.code.toUpperCase()}
              <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor"
                   stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M7 4v13M7 17l-3-3M17 20V7M17 7l3 3" />
              </svg>
            </button>
          </span>
          {#if limits && meta}
            <button class="max" onclick={useMax} type="button">Max</button>
          {/if}
        </span>
        <input class="mono" bind:value={amount} oninput={reset} placeholder="0.0" />
        {#if converted != null && meta}
          <span class="hint">
            {denom === "coin"
              ? settings.money(converted)
              : `${coinAmount} ${meta.ticker}`}
          </span>
        {/if}
        {#if limits && meta}
          <span class="hint">
            Balance {plain(limits.balanceMinor, meta.decimals)}
            {meta.ticker}, less a {plain(limits.feeMinor, meta.decimals)} fee.
          </span>
        {/if}
      </label>

      {#if coins.length > 0}
        <div class="coins">
          <button class="disclose" type="button" onclick={() => (showCoins = !showCoins)}>
            {showCoins ? "Hide" : "Choose"} which coins to spend
            <span class="muted">
              {picked.size === 0
                ? "wallet decides"
                : `${picked.size} chosen, ${formatAmount(pickedTotal, meta?.decimals ?? 8)} ${meta?.ticker}`}
            </span>
          </button>

          {#if showCoins}
            <ul class="coinlist">
              {#each coins as coin (coin.outpoint)}
                <li>
                  <label class="coin">
                    <input
                      type="checkbox"
                      checked={picked.has(coin.outpoint)}
                      onchange={() => toggleCoin(coin.outpoint)}
                    />
                    <span class="mono amt">
                      {formatAmount(coin.valueMinor, meta?.decimals ?? 8)}
                    </span>
                    <span class="where muted">
                      {coin.keyIndex === 0 ? "main" : `sub-wallet ${coin.keyIndex}`}
                    </span>
                  </label>
                </li>
              {/each}
            </ul>

            {#if picked.size > 0}
              <button class="clear" type="button" onclick={() => { picked = new Set(); reset(); }}>
                Clear selection
              </button>
            {/if}

            {#if wouldLink}
              <p class="link-warn">
                These coins sit on different sub-wallets. Spending them together publicly
                proves the same person owns both, which undoes the split.
              </p>
            {/if}
          {/if}
        </div>
      {/if}

      {#if error}
        <p class="err">{error}</p>
      {/if}

      {#if quote && meta}
        <div class="quote">
          <div class="line">
            <span>Sending</span>
            <strong class="mono"
              >{formatAmount(quote.amountMinor, meta.decimals)} {meta.ticker}</strong
            >
          </div>
          <div class="line">
            <span>Network fee</span>
            <strong class="mono"
              >{formatAmount(quote.feeMinor, meta.decimals)} {meta.ticker}</strong
            >
          </div>
          <div class="line total">
            <span>Leaves your wallet</span>
            <strong class="mono"
              >{formatAmount(quote.totalMinor, meta.decimals)} {meta.ticker}</strong
            >
          </div>
          <p class="warn">This cannot be undone once sent.</p>
        </div>

        <button class="btn btn-primary wide" disabled={busy} onclick={confirm}>
          {busy ? "Sending" : "Confirm and send"}
        </button>
      {:else}
        <button class="btn btn-primary wide" disabled={!ready || busy} onclick={preview}>
          {busy ? "Checking" : "Preview"}
        </button>
      {/if}
    {/if}
  </div>
</div>

<style>
  .scrim {
    position: fixed; inset: 0; display: grid; place-items: center;
    padding: 24px; background: rgba(0, 0, 0, 0.55); z-index: 50; border: 0;
  }
  .panel {
    width: 100%; max-width: 440px; padding: 20px 22px 22px;
    box-shadow: var(--shadow); text-align: left; cursor: default;
  }
  header { display: flex; align-items: center; gap: 10px; margin-bottom: 14px; }
  h2 { margin: 0; font-size: 15.5px; font-weight: 650; }
  .x { margin-left: auto; color: var(--text-muted); font-size: 20px; line-height: 1; padding: 0 4px; }
  .x:hover { color: var(--text); }
  .back { color: var(--text-muted); display: flex; padding: 0; }
  .back:hover { color: var(--text); }

  .list {
    display: flex; flex-direction: column; gap: 6px;
    margin: 0; padding: 0; list-style: none; max-height: 340px; overflow-y: auto;
  }
  .row {
    display: flex; align-items: center; gap: 12px; width: 100%;
    padding: 11px 13px; text-align: left;
    border: 1px solid var(--border); border-radius: var(--radius-sm);
    background: var(--bg-raised); transition: background 110ms var(--ease);
  }
  .row:hover:not(:disabled) { background: var(--card-hover); }
  .row:disabled { opacity: 0.45; cursor: not-allowed; }
  .name { font-size: 13.5px; font-weight: 500; display: flex; flex-direction: column; gap: 2px; }
  .soon { font-size: 10.5px; color: var(--text-faint); font-weight: 500; }
  .held {
    margin-left: auto; display: flex; flex-direction: column; align-items: flex-end;
    gap: 2px; font-size: 12.5px; font-variant-numeric: tabular-nums;
  }
  .held .muted { font-size: 11.5px; }

  .field { display: block; margin-bottom: 13px; }
  .field span { display: block; margin-bottom: 5px; font-size: 12.5px; color: var(--text-muted); }
  .amount-label { display: flex !important; align-items: baseline; justify-content: space-between; gap: 10px; }
  .max { color: var(--accent); font-size: 12px; font-weight: 600; font-family: var(--font-mono); padding: 0; }
  .unit { display: inline-flex !important; align-items: center; gap: 6px; margin-bottom: 0 !important; }
  .swap {
    display: inline-flex; align-items: center; gap: 4px;
    padding: 2px 8px; border-radius: 999px;
    border: 1px solid var(--border-strong);
    color: var(--text); font-size: 11.5px; font-weight: 600;
  }
  .swap:hover:not(:disabled) { background: var(--card-hover); }
  .swap:disabled { opacity: 0.5; cursor: not-allowed; }
  .max:hover { text-decoration: underline; }
  .hint { display: block; margin: 5px 0 0; font-size: 11.5px; color: var(--text-faint); }
  .field input { width: 100%; }

  .coins { margin: 4px 0 12px; }
  .disclose {
    display: flex; align-items: baseline; justify-content: space-between;
    width: 100%; padding: 8px 11px; gap: 10px;
    border: 1px solid var(--border); border-radius: var(--radius-sm);
    background: var(--bg-raised); color: var(--text);
    font-size: 12.5px; text-align: left;
  }
  .disclose:hover { background: var(--card-hover); }
  .disclose .muted { font-size: 11.5px; }
  .coinlist {
    display: flex; flex-direction: column; gap: 2px;
    margin: 8px 0 0; padding: 0; list-style: none;
    max-height: 170px; overflow-y: auto;
  }
  .coin {
    display: flex; align-items: center; gap: 10px;
    padding: 6px 9px; border-radius: var(--radius-sm);
    font-size: 12.5px; cursor: pointer;
  }
  .coin:hover { background: var(--bg-raised); }
  .amt { font-variant-numeric: tabular-nums; }
  .where { margin-left: auto; font-size: 11.5px; }
  .clear { margin-top: 8px; color: var(--text-muted); font-size: 11.5px; padding: 0; }
  .clear:hover { color: var(--text); }
  .link-warn { margin: 9px 0 0; font-size: 11.5px; color: var(--warn); }

  .quote {
    margin: 16px 0 0; padding: 14px 15px;
    border: 1px solid var(--border); border-radius: var(--radius-sm);
    background: var(--bg-raised);
  }
  .line { display: flex; align-items: baseline; gap: 9px; font-size: 13px; margin-bottom: 7px; }
  .line strong { margin-left: auto; font-variant-numeric: tabular-nums; }
  .total { padding-top: 7px; border-top: 1px solid var(--border); margin-bottom: 0; }
  .warn { margin: 10px 0 0; font-size: 12px; color: var(--warn); }
  .err { margin: 4px 0 0; color: var(--danger); font-size: 12.5px; }
  .ok { margin: 0 0 14px; color: var(--ok); font-size: 14px; font-weight: 600; }
  .label { margin: 0 0 4px; font-size: 12px; color: var(--text-muted); }
  .wrap { word-break: break-all; font-size: 12px; margin: 0 0 16px; }
  .note { margin: 0; font-size: 13px; color: var(--text-muted); }
  .wide { width: 100%; margin-top: 16px; }
</style>
