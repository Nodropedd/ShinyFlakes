<script lang="ts">
  import AssetIcon from "../lib/AssetIcon.svelte";
  import { BY_ID, formatAmount } from "../lib/assets";
  import { ipc, type Bucket } from "../lib/ipc";

  let buckets = $state<Bucket[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);

  $effect(() => {
    ipc
      .listBuckets()
      .then((b) => (buckets = b))
      .catch((e) => (error = (e as { message?: string }).message ?? String(e)))
      .finally(() => (loading = false));
  });
</script>

<div class="view">
  <header>
    <h1>Buckets</h1>
    <p class="muted">
      Named groups of funds inside this one wallet. A payment draws from the
      bucket you pick, and if that bucket is short the app asks before pulling
      the difference from another one.
    </p>
  </header>

  {#if loading}
    <p class="muted">Loading.</p>
  {:else if error}
    <p class="err">{error}</p>
  {:else if buckets.length === 0}
    <div class="empty card">
      <h2>No buckets yet</h2>
      <p class="muted">
        Creating a bucket means deriving addresses for it, so this waits on
        chain connectivity. On Bitcoin and Litecoin a bucket is a set of
        tagged outputs. On Monero it is a subaddress account. On the
        account-based chains it is a separate derived address.
      </p>
      <button class="btn" disabled title="Needs chain connectivity">
        New bucket
      </button>
    </div>
  {:else}
    <ul class="list">
      {#each buckets as b (b.id)}
        {@const meta = BY_ID[b.asset]}
        <li class="row">
          {#if meta}
            <AssetIcon asset={meta} size={30} />
            <div class="name">
              <strong>{b.name}</strong>
              <span class="muted">{meta.name} &middot; {b.addressCount} addresses</span>
            </div>
            <span class="qty mono selectable">
              {formatAmount(b.balanceMinor, meta.decimals)}
              {meta.ticker}
            </span>
          {:else}
            <!-- A vault written by a build that knew an asset this one does
                 not must not take the whole list down with it. -->
            <div class="name">
              <strong>{b.name}</strong>
              <span class="muted">Unrecognised asset &ldquo;{b.asset}&rdquo;</span>
            </div>
            <span class="qty mono selectable">{b.balanceMinor}</span>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .view { display: flex; flex-direction: column; gap: 20px; }
  h1 { margin: 0 0 6px; font-size: 20px; font-weight: 650; }
  header p { margin: 0; max-width: 62ch; font-size: 13.5px; }
  .empty { padding: 26px; max-width: 62ch; }
  .empty h2 { margin: 0 0 8px; font-size: 15px; }
  .empty p { margin: 0 0 16px; font-size: 13.5px; }
  .err { color: var(--danger); font-size: 13px; }
  .list {
    display: flex; flex-direction: column; margin: 0; padding: 0;
    list-style: none; border: 1px solid var(--border);
    border-radius: var(--radius); overflow: hidden; background: var(--card);
  }
  .row { display: flex; align-items: center; gap: 12px; padding: 13px 16px; }
  .row + .row { border-top: 1px solid var(--border); }
  .row:hover { background: var(--card-hover); }
  .name { display: flex; flex-direction: column; gap: 1px; font-size: 13.5px; }
  .name span { font-size: 12px; }
  .qty { margin-left: auto; font-size: 14px; font-variant-numeric: tabular-nums; }
</style>
