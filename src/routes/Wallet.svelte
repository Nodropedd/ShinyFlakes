<script lang="ts">
  import Sidebar, { type View } from "../lib/Sidebar.svelte";
  import Portfolio from "./Portfolio.svelte";
  import Buckets from "./Buckets.svelte";
  import Utxo from "./Utxo.svelte";
  import Activity from "./Activity.svelte";
  import Settings from "./Settings.svelte";
  import { ipc } from "../lib/ipc";
  import { session } from "../lib/session.svelte";
  import { wallet } from "../lib/wallet.svelte";

  const VIEWS: View[] = ["portfolio", "buckets", "utxo", "activity", "settings"];

  // The section lives in the URL fragment. Users never see it in a Tauri
  // window, but it gives back and forward navigation for free and makes a
  // given screen reachable directly while developing.
  function fromHash(): View {
    const h = location.hash.replace(/^#\/?/, "");
    return (VIEWS as string[]).includes(h) ? (h as View) : "portfolio";
  }

  let view = $state<View>(fromHash());

  function go(next: View) {
    view = next;
    location.hash = next;
  }

  $effect(() => {
    const sync = () => (view = fromHash());
    addEventListener("hashchange", sync);
    return () => removeEventListener("hashchange", sync);
  });

  // Addresses load immediately; balances poll once the user has consented.
  $effect(() => {
    wallet.start();
    return () => wallet.stop();
  });

  async function lock() {
    await ipc.lock();
    await session.refresh();
  }
</script>

<div class="shell">
  <Sidebar current={view} onselect={go} onlock={lock} />

  <main>
    {#if view === "portfolio"}
      <Portfolio />
    {:else if view === "buckets"}
      <Buckets />
    {:else if view === "utxo"}
      <Utxo />
    {:else if view === "activity"}
      <Activity />
    {:else}
      <Settings />
    {/if}
  </main>
</div>

<style>
  .shell {
    display: flex;
    height: 100%;
  }

  main {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 34px 40px 44px;
  }
</style>
