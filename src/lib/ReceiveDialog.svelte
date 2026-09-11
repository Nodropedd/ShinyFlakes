<script lang="ts">
  import { untrack } from "svelte";

  import Address from "./Address.svelte";
  import AssetIcon from "./AssetIcon.svelte";
  import QrCode from "./QrCode.svelte";
  import { ASSETS, BY_ID, formatAmount } from "./assets";
  import { ipc, type AssetId, type ReceiveAddress } from "./ipc";
  import { settings } from "./settings.svelte";
  import { wallet } from "./wallet.svelte";

  let { initial, onclose }: { initial: AssetId | null; onclose: () => void } = $props();

  // The dialog is mounted fresh each time it opens, so the asset it starts on
  // is genuinely a starting point and should not track later changes.
  let chosen = $state<AssetId | null>(untrack(() => initial));
  let filter = $state("");

  const meta = $derived(chosen ? BY_ID[chosen] : null);
  const entry = $derived(chosen ? wallet.addresses[chosen] : null);

  // Reusing one address links every payment ever sent to it. On chains that
  // support it the wallet walks forward to one that has never been seen, and
  // falls back to the account address when that lookup fails.
  let fresh = $state<ReceiveAddress | null>(null);
  let finding = $state(false);
  let freshError = $state<string | null>(null);

  $effect(() => {
    const asset = chosen;
    fresh = null;
    freshError = null;
    if (!asset) return;

    finding = true;
    ipc
      .nextReceiveAddress(asset)
      .then((next) => {
        if (chosen === asset) fresh = next;
      })
      .catch((e) => {
        if (chosen === asset) {
          freshError = (e as { message?: string }).message ?? String(e);
        }
      })
      .finally(() => {
        if (chosen === asset) finding = false;
      });
  });

  // The rotated address when there is one, otherwise the account address.
  const shownAddress = $derived(fresh?.address ?? entry?.address ?? null);

  const matches = $derived(
    ASSETS.filter((a) => {
      const q = filter.trim().toLowerCase();
      return !q || a.name.toLowerCase().includes(q) || a.ticker.toLowerCase().includes(q);
    }),
  );
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
      {#if chosen}
        <button class="back" onclick={() => (chosen = null)} aria-label="Back">
          <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor"
               stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
            <path d="M15 5l-7 7 7 7" />
          </svg>
        </button>
      {/if}
      <h2>{chosen ? `Receive ${meta?.name}` : "Select asset"}</h2>
      <button class="x" onclick={onclose} aria-label="Close">&times;</button>
    </header>

    {#if !chosen}
      <input class="search" placeholder="Search" bind:value={filter} />
      <ul class="list">
        {#each matches as asset (asset.id)}
          {@const bal = wallet.balances[asset.id]}
          {@const worth = wallet.value(asset.id)}
          <li>
            <button class="row" onclick={() => (chosen = asset.id)}>
              <AssetIcon {asset} size={32} />
              <span class="name">{asset.name}</span>
              <span class="held">
                {#if bal?.minor != null && bal.minor !== "0"}
                  <span class="mono">{formatAmount(bal.minor, asset.decimals)} {asset.ticker}</span>
                  <span class="muted">{worth != null ? settings.money(worth) : ""}</span>
                {:else}
                  <span class="muted ticker">{asset.ticker}</span>
                {/if}
              </span>
            </button>
          </li>
        {/each}
      </ul>
    {:else if finding && !shownAddress}
      <p class="note">Finding an unused address.</p>
    {:else if shownAddress}
      <p class="lede muted">
        Send only {meta?.name} to this address. Anything else sent here is lost.
      </p>
      <div class="qrwrap">
        <QrCode value={shownAddress} size={168} />
      </div>
      <div class="addrbox">
        <span class="mono full selectable">{shownAddress}</span>
      </div>
      <div class="copyrow">
        <Address value={shownAddress} />
      </div>

      {#if fresh?.rotates}
        <p class="note">
          A fresh address, never used before. Each payment you receive gets its own,
          so they cannot be tied to each other by the address alone.
        </p>
      {:else if fresh && !fresh.rotates}
        <p class="note">
          {meta?.name} uses one account rather than a new address per payment, so this
          one repeats. Payments to it are linkable.
        </p>
      {/if}

      {#if freshError}
        <p class="note">
          Could not check for an unused address, so this is the main one:
          {freshError}
        </p>
      {/if}

      {#if entry?.host}
        <p class="note">
          {meta?.name} is a token on {entry.host} and shares that chain's address.
        </p>
      {/if}
      <button class="btn btn-primary wide" onclick={onclose}>Done</button>
    {:else}
      <p class="note">{entry?.unsupported ?? "No address available for this asset."}</p>
      <button class="btn wide" onclick={() => (chosen = null)}>Back</button>
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

  .search { width: 100%; margin-bottom: 12px; }
  .list {
    display: flex; flex-direction: column; gap: 6px;
    margin: 0; padding: 0; list-style: none;
    max-height: 340px; overflow-y: auto;
  }
  .row {
    display: flex; align-items: center; gap: 12px; width: 100%;
    padding: 11px 13px; text-align: left;
    border: 1px solid var(--border); border-radius: var(--radius-sm);
    background: var(--bg-raised);
    transition: background 110ms var(--ease);
  }
  .row:hover { background: var(--card-hover); }
  .name { font-size: 13.5px; font-weight: 500; }
  .held {
    margin-left: auto; display: flex; flex-direction: column;
    align-items: flex-end; gap: 2px; font-size: 12.5px;
    font-variant-numeric: tabular-nums;
  }
  .held .muted { font-size: 11.5px; }
  .ticker { font-size: 12px !important; }

  .lede { margin: 0 0 12px; font-size: 12.5px; }
  .qrwrap { display: flex; justify-content: center; margin: 4px 0 14px; }
  .addrbox {
    padding: 13px 14px; border-radius: var(--radius-sm);
    border: 1px solid var(--border); background: var(--bg-raised);
  }
  .full { font-size: 12.5px; word-break: break-all; line-height: 1.6; }
  .copyrow { margin-top: 10px; }
  .note { margin: 10px 0 0; font-size: 12px; color: var(--text-faint); }
  .wide { width: 100%; margin-top: 16px; }
</style>
