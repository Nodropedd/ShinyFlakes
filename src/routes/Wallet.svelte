<script lang="ts">
  import { onMount } from "svelte";
  import Sidebar, { type View } from "../lib/Sidebar.svelte";
  import Portfolio from "./Portfolio.svelte";
  import Swap from "./Swap.svelte";
  import Utxo from "./Utxo.svelte";
  import Activity from "./Activity.svelte";
  import Settings from "./Settings.svelte";
  import TwoFactorPrompt from "../lib/TwoFactorPrompt.svelte";
  import { ipc } from "../lib/ipc";
  import { session } from "../lib/session.svelte";
  import { wallet } from "../lib/wallet.svelte";

  const VIEWS: View[] = ["portfolio", "swap", "utxo", "activity", "settings"];

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

  onMount(() => {
    wallet.start();
    return () => wallet.stop();
  });

  let dormantPrompt = $state(false);

  $effect(() => {
    void ipc
      .dormantStepUpPending()
      .then((pending) => (dormantPrompt = pending))
      .catch(() => (dormantPrompt = false));
  });

  async function dormantPassed() {
    try {

      if (await ipc.clearDormantStepUp()) dormantPrompt = false;
    } catch {
      dormantPrompt = false;
    }
  }

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
    {:else if view === "swap"}
      <Swap />
    {:else if view === "utxo"}
      <Utxo />
    {:else if view === "activity"}
      <Activity />
    {:else}
      <Settings />
    {/if}
  </main>
</div>

{#if dormantPrompt}
  <TwoFactorPrompt
    purpose="open a wallet that had been shut for a while"
    action="dormant"
    onpassed={dormantPassed}
    oncancel={lock}
  />
{/if}

<style>
  .shell {
    display: flex;
    height: 100%;
  }

  @media (max-width: 720px) {
    .shell {
      flex-direction: column-reverse;
    }
  }

  main {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 34px 40px 44px;
  }

  @media (max-width: 720px) {
    main {

      padding: 18px 14px 22px;
      padding-top: calc(18px + env(safe-area-inset-top, 0px));
    }
  }
</style>
