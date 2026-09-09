<script lang="ts">
  import AssetIcon from "../lib/AssetIcon.svelte";
  import FragmentAnimation from "../lib/FragmentAnimation.svelte";
  import SendDialog from "../lib/SendDialog.svelte";
  import { ASSETS, BY_ID, formatAmount, toMinor } from "../lib/assets";
  import {
    ipc,
    type AssetId,
    type FragmentQuote,
    type SendQuote,
    type UtxoEntry,
    type UtxoState,
  } from "../lib/ipc";
  import { settings } from "../lib/settings.svelte";
  import { wallet } from "../lib/wallet.svelte";

  // Only true UTXO chains belong here. Monero has outputs too, but ring
  // signatures mean its privacy does not depend on how they are arranged.
  const CHAINS = ASSETS.filter((a) => a.utxo);

  const CHAIN_KEY = "shinyflakes.utxoChain";

  function rememberedChain(): AssetId {
    try {
      const saved = localStorage.getItem(CHAIN_KEY);
      if (saved && CHAINS.some((c) => c.id === saved)) return saved as AssetId;
    } catch {
      /* fall through to the default */
    }
    return CHAINS[0].id;
  }

  // Persisted so a remount does not silently drop back to Bitcoin while the
  // screen is showing Litecoin figures.
  let selected = $state<AssetId>(rememberedChain());

  function pickChain(id: AssetId) {
    selected = id;
    try {
      localStorage.setItem(CHAIN_KEY, id);
    } catch {
      /* the choice simply will not persist */
    }
  }
  let layout = $state<UtxoState | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let sending = $state(false);

  let amount = $state("");
  let pieces = $state(4);
  let quote = $state<FragmentQuote | null>(null);
  let combineQuote = $state<SendQuote | null>(null);
  let checking = $state(false);
  let quoteError = $state<string | null>(null);

  let running = $state<null | "split" | "form">(null);
  let outcome = $state<"working" | "done" | "failed">("working");
  let txid = $state<string | null>(null);

  const meta = $derived(BY_ID[selected]);

  // Quotes carry the asset they were built for. Rendering against that rather
  // than the locally selected chain means a mismatch can never show one
  // chain's ticker against another chain's amounts.
  const layoutMeta = $derived(layout ? BY_ID[layout.asset] : meta);
  const splitMeta = $derived(quote ? BY_ID[quote.asset] : meta);
  const combineMeta = $derived(combineQuote ? BY_ID[combineQuote.asset] : meta);
  const price = $derived(wallet.prices[selected]?.price ?? null);

  async function load() {
    // Switching chains starts a second lookup while the first is still in
    // flight. Whichever returns last would otherwise win, which is how
    // Bitcoin figures ended up under a Litecoin heading. Every response is
    // checked against the chain still selected and dropped if it is stale.
    const forChain = selected;
    loading = true;
    error = null;
    try {
      const next = await ipc.utxoState(forChain);
      if (forChain !== selected) return;
      layout = next;
    } catch (e) {
      if (forChain !== selected) return;
      error = message(e);
    } finally {
      if (forChain === selected) loading = false;
    }
  }

  $effect(() => {
    const chain = selected;
    layout = null;
    reset();
    void load();
    return () => {
      void chain;
    };
  });

  function message(e: unknown) {
    return (e as { message?: string }).message ?? String(e);
  }

  function reset() {
    quote = null;
    combineQuote = null;
    quoteError = null;
    txid = null;
    running = null;
  }

  function plain(minor: string) {
    return formatAmount(minor, meta.decimals).replace(/,/g, "");
  }

  function cash(minor: string) {
    if (price == null) return null;
    return (Number(minor) / 10 ** meta.decimals) * price;
  }

  function useAll() {
    if (!layout) return;
    amount = plain(layout.totalMinor);
    reset();
  }

  async function previewSplit() {
    if (!amount.trim() || checking) return;
    checking = true;
    quote = null;
    combineQuote = null;
    quoteError = null;
    try {
      const forChain = selected;
      const next = await ipc.fragmentPreview(
        forChain,
        toMinor(amount, meta.decimals),
        pieces,
      );
      if (forChain === selected) quote = next;
    } catch (e) {
      quoteError = message(e);
    } finally {
      checking = false;
    }
  }

  async function previewCombine() {
    if (checking) return;
    checking = true;
    quote = null;
    combineQuote = null;
    quoteError = null;
    try {
      const forChain = selected;
      const next = await ipc.consolidatePreview(forChain);
      if (forChain === selected) combineQuote = next;
    } catch (e) {
      quoteError = message(e);
    } finally {
      checking = false;
    }
  }

  async function run(kind: "split" | "form") {
    if (running) return;
    running = kind;
    outcome = "working";
    txid = null;
    try {
      txid =
        kind === "split"
          ? await ipc.fragmentExecute(selected, quote!.amountMinor, quote!.pieces)
          : await ipc.consolidateExecute(selected);
      outcome = "done";
      await load();
      void wallet.refresh();
    } catch (e) {
      quoteError = message(e);
      outcome = "failed";
    }
  }

  // Grouping by sub-wallet is what makes a split visible: one group before,
  // several after.
  const grouped = $derived(
    (layout?.outputs ?? []).reduce<Record<number, UtxoEntry[]>>((acc, o) => {
      (acc[o.keyIndex] ??= []).push(o);
      return acc;
    }, {}),
  );

  const spread = $derived(Object.keys(grouped).length > 1);
  const sliderMax = $derived(Math.max(2, Math.min(layout?.maxPieces ?? 2, 500)));
</script>

<div class="view">
  <header>
    <h1>UTXO fragmentation</h1>
    <p class="muted">
      Bitcoin and Litecoin hold value as discrete outputs rather than one
      balance. Splitting a large output into smaller ones on separate
      addresses changes what moves when you spend. It costs a fee now, and
      every extra output costs again to spend later.
    </p>
  </header>

  <div class="tabs">
    {#each CHAINS as chain (chain.id)}
      <button class="tab" class:active={selected === chain.id} onclick={() => pickChain(chain.id)}>
        <AssetIcon asset={chain} size={20} />
        {chain.name}
      </button>
    {/each}
  </div>

  {#if error}
    <p class="err">{error}</p>
  {/if}

  {#if !layout && loading}
    <p class="muted">Reading the chain.</p>
  {:else if layout}
    <div class="summary card">
      <div>
        <p class="label">Spendable{loading ? ", refreshing" : ""}</p>
        <p class="big mono">
          {formatAmount(layout.totalMinor, layoutMeta.decimals)} {layoutMeta.ticker}
        </p>
        {#if cash(layout.totalMinor) != null}
          <p class="sub muted">{settings.money(cash(layout.totalMinor)!)}</p>
        {/if}
      </div>
      <div class="mid">
        <p class="label">Outputs</p>
        <p class="big">{layout.outputs.length}</p>
        <p class="sub muted">
          across {layout.sources}
          {layout.sources === 1 ? "sub-wallet" : "sub-wallets"}
        </p>
      </div>
      <div class="acts">
        <button class="btn" onclick={() => (sending = true)}>Send</button>
        <button class="btn" onclick={() => void load()} disabled={loading}>Refresh</button>
      </div>
    </div>

    {#if layout.outputs.length === 0}
      <div class="empty card">
        <h2>Nothing to split</h2>
        <p class="muted">This chain holds no confirmed outputs. Receive something first.</p>
      </div>
    {:else}
      <section>
        <h2>Current layout</h2>
        <ul class="wallets">
          {#each Object.entries(grouped) as [index, outs] (index)}
            <li class="wallet card">
              <div class="whead">
                <strong>{Number(index) === 0 ? "Main address" : `Sub-wallet ${index}`}</strong>
                <span class="mono">
                  {formatAmount(
                    outs.reduce((s, o) => s + BigInt(o.valueMinor), 0n).toString(),
                    meta.decimals,
                  )}
                  {meta.ticker}
                </span>
              </div>
              <div class="chips">
                {#each outs as o (o.txid + o.vout)}
                  <span class="chip mono" title="{o.txid}:{o.vout}">
                    {formatAmount(o.valueMinor, meta.decimals)}
                  </span>
                {/each}
              </div>
            </li>
          {/each}
        </ul>
      </section>

      {#if running}
        <section class="running card">
          <FragmentAnimation
            pieces={running === "split" ? (quote?.pieces ?? pieces) : layout.outputs.length}
            status={outcome}
            direction={running}
          />
          {#if txid}
            <p class="label">Transaction id</p>
            <p class="mono wrap selectable">{txid}</p>
            <button class="btn" onclick={() => { reset(); amount = ""; }}>Done</button>
          {:else if outcome === "failed"}
            <p class="err">{quoteError}</p>
            <button class="btn" onclick={() => (running = null)}>Back</button>
          {/if}
        </section>
      {:else}
        <section>
          <h2>Split</h2>
          <div class="controls card">
            <label class="field">
              <span class="row">
                How much to split
                <button class="link" onclick={useAll}>All {plain(layout.totalMinor)}</button>
              </span>
              <input class="mono" bind:value={amount} oninput={reset} placeholder="0.0" />
            </label>

            <label class="field">
              <span class="row">
                Into how many pieces
                <span class="muted">{pieces}</span>
              </span>
              <input type="range" min="2" max={sliderMax} bind:value={pieces} oninput={reset} />
              <span class="hint">
                Up to {sliderMax} with this balance. Two limits apply: each piece must clear
                the dust limit of {plain(layout.dustMinor)} {meta.ticker}, and the whole
                transaction has to stay under the size every node will relay, which caps it
                near two thousand however much you hold.
              </span>
            </label>

            {#if quoteError}
              <p class="err">{quoteError}</p>
            {/if}

            {#if quote}
              <div class="quote">
                <div class="line">
                  <span>Each piece</span>
                  <strong class="mono">
                    {formatAmount(quote.perPieceMinor, splitMeta.decimals)} {splitMeta.ticker}
                  </strong>
                </div>
                <div class="line">
                  <span>Network fee</span>
                  <strong class="mono">
                    {formatAmount(quote.feeMinor, splitMeta.decimals)} {splitMeta.ticker}
                  </strong>
                </div>
                <div class="line">
                  <span>Left on the main address</span>
                  <strong class="mono">
                    {formatAmount(quote.changeMinor, splitMeta.decimals)} {splitMeta.ticker}
                  </strong>
                </div>
                {#if cash(quote.feeMinor) != null}
                  <p class="cost muted">
                    The fee is {settings.money(cash(quote.feeMinor)!)}, and spending these
                    {quote.pieces} pieces later will cost more than spending one would have.
                  </p>
                {/if}
                {#if quote.sources > 1}
                  <p class="cost muted">
                    One sub-wallet was not enough, so this draws from {quote.sources} of them.
                  </p>
                {/if}
              </div>
              <button class="btn btn-primary wide" onclick={() => run("split")}>
                Split into {quote.pieces}
              </button>
            {:else}
              <button
                class="btn btn-primary wide"
                disabled={!amount.trim() || checking}
                onclick={previewSplit}
              >
                {checking ? "Checking" : "Preview split"}
              </button>
            {/if}
          </div>
        </section>

        <section>
          <h2>Form it back</h2>
          <div class="controls card">
            <p class="muted">
              Sweeps every output, across every sub-wallet, back into a single one on the
              main address. This is what to use before moving the balance somewhere else.
            </p>

            {#if combineQuote}
              <div class="quote">
                <div class="line">
                  <span>Comes back as one {combineMeta.ticker} output</span>
                  <strong class="mono">
                    {formatAmount(combineQuote.amountMinor, combineMeta.decimals)}
                    {combineMeta.ticker}
                  </strong>
                </div>
                <div class="line">
                  <span>Network fee</span>
                  <strong class="mono">
                    {formatAmount(combineQuote.feeMinor, combineMeta.decimals)}
                    {combineMeta.ticker}
                  </strong>
                </div>
              </div>
              <button class="btn btn-primary wide" onclick={() => run("form")}>
                Combine into one
              </button>
            {:else}
              <button
                class="btn wide"
                disabled={checking || !spread && layout.outputs.length < 2}
                onclick={previewCombine}
              >
                {checking ? "Checking" : "Preview combine"}
              </button>
              {#if !spread && layout.outputs.length < 2}
                <p class="hint">Already a single output, so there is nothing to combine.</p>
              {/if}
            {/if}
          </div>
        </section>
      {/if}
    {/if}
  {/if}
</div>

{#if sending}
  <SendDialog
    initial={selected}
    onclose={() => (sending = false)}
    onsent={() => {
      void load();
      void wallet.refresh();
    }}
  />
{/if}

<style>
  .view { display: flex; flex-direction: column; gap: 20px; max-width: 720px; }
  h1 { margin: 0 0 6px; font-size: 20px; font-weight: 650; }
  header p { margin: 0; max-width: 68ch; font-size: 13px; }
  .err { margin: 0; color: var(--danger); font-size: 13px; }

  .tabs { display: flex; gap: 6px; }
  .tab {
    display: flex; align-items: center; gap: 8px;
    padding: 7px 13px 7px 8px; border-radius: 999px;
    border: 1px solid var(--border); color: var(--text-muted);
    font-size: 13px; font-weight: 500;
  }
  .tab:hover { color: var(--text); background: var(--card); }
  .tab.active { color: var(--text); background: var(--card); border-color: var(--border-strong); }

  .summary { display: flex; align-items: flex-start; gap: 28px; padding: 18px 20px; }
  .summary .mid { flex: none; }
  .summary .acts { margin-left: auto; display: flex; gap: 8px; padding-top: 2px; }
  .label { margin: 0 0 4px; font-size: 12px; color: var(--text-muted); }
  .big { margin: 0; font-size: 20px; font-weight: 600; font-variant-numeric: tabular-nums; }
  .sub { margin: 3px 0 0; font-size: 12px; }

  .empty { padding: 24px; }
  .empty h2 { margin: 0 0 6px; font-size: 15px; }
  .empty p { margin: 0; font-size: 13.5px; }

  h2 { margin: 0 0 9px; font-size: 12.5px; font-weight: 600; color: var(--text-muted); }
  .wallets { display: flex; flex-direction: column; gap: 8px; margin: 0; padding: 0; list-style: none; }
  .wallet { padding: 13px 15px; }
  .whead { display: flex; align-items: baseline; justify-content: space-between; font-size: 13px; }
  .whead span { font-variant-numeric: tabular-nums; }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 9px; }
  .chip {
    padding: 3px 9px; border-radius: 999px;
    border: 1px solid var(--border); background: var(--bg-raised);
    font-size: 11.5px; color: var(--text-muted); cursor: help;
  }

  .controls { padding: 18px 20px; }
  .controls > p { margin: 0 0 4px; font-size: 13px; }
  .field { display: block; margin-bottom: 16px; }
  .field .row {
    display: flex; align-items: baseline; justify-content: space-between;
    margin-bottom: 6px; font-size: 12.5px; color: var(--text-muted);
  }
  .field input:not([type="range"]) { width: 100%; }
  .field input[type="range"] { width: 100%; accent-color: var(--accent); }
  .link { color: var(--accent); font-size: 12px; font-weight: 600; font-family: var(--font-mono); padding: 0; }
  .link:hover { text-decoration: underline; }
  .hint { display: block; margin-top: 6px; font-size: 11.5px; color: var(--text-faint); }

  .quote {
    margin: 12px 0 0; padding: 14px 15px;
    border: 1px solid var(--border); border-radius: var(--radius-sm);
    background: var(--bg-raised);
  }
  .line { display: flex; align-items: baseline; font-size: 13px; margin-bottom: 7px; }
  .line:last-of-type { margin-bottom: 0; }
  .line strong { margin-left: auto; font-variant-numeric: tabular-nums; }
  .cost { margin: 10px 0 0; font-size: 12px; }
  .wide { width: 100%; margin-top: 16px; }

  .running { padding: 22px 20px; text-align: center; }
  .running .label { margin-top: 14px; }
  .wrap { word-break: break-all; font-size: 12px; margin: 0 0 14px; }
</style>
