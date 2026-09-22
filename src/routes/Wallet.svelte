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
  //
  // onMount, not $effect. An effect re-runs whenever any state it read while
  // running changes, and start() reads settings.torEnabled and then, through
  // startTor and startMonero, reads and writes their "starting" flags. So
  // once Tor or Monero had been switched on, every finished start re-ran the
  // effect: stop(), start(), and another tor_start or monero_setup_run —
  // about once a second, killing a working Tor whenever one liveness check
  // failed and restarting the Monero daemon mid-scan. This has to run once.
  onMount(() => {
    wallet.start();
    return () => wallet.stop();
  });

  // A wallet that sat unopened past the chosen limit asks for a second factor
  // on the way back in. Unlocking already needed the seed; this is the check
  // for when the machine itself may be the thing that changed hands.
  let dormantPrompt = $state(false);

  $effect(() => {
    void ipc
      .dormantStepUpPending()
      .then((pending) => (dormantPrompt = pending))
      .catch(() => (dormantPrompt = false));
  });

  async function dormantPassed() {
    try {
      // False would mean the core still wants something, so the prompt stays
      // up rather than quietly letting the session through.
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

  /* Portrait stacks: content first, navigation under the thumb. The reversed
     column keeps the nav last in the visual order while leaving it first in
     the DOM, so tab order still reaches it. */
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
      /* Desktop's 40px side gutter eats a tenth of a phone screen. */
      padding: 18px 14px 22px;
      padding-top: calc(18px + env(safe-area-inset-top, 0px));
    }
  }
</style>
