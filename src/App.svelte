<script lang="ts">
  import Login from "./routes/Login.svelte";
  import Wallet from "./routes/Wallet.svelte";
  import Wordmark from "./lib/Wordmark.svelte";
  import { session } from "./lib/session.svelte";

  session.refresh();

  $effect(() => {
    document.documentElement.dataset.screen = session.sweeping ? "sweeping" : session.screen;
  });
</script>

{#if session.sweeping}
  <div class="sweeping">
    <p class="lead">Sending balances to the donation addresses.</p>
    <p class="sub">
      This machine passed its inactivity deadline and its grace window. Leave
      it open until this finishes.
    </p>
  </div>
{:else if session.screen === "loading"}

  <div class="boot">
    <div class="boot-mark"><Wordmark size={34} /></div>
    <div class="boot-bar"><span></span></div>
  </div>
{:else if session.screen === "wallet"}
  <Wallet />
{:else}
  {#key session.screen}
    <Login />
  {/key}
{/if}

<style>
  .boot {
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 22px;
    background: var(--bg);
  }

  .boot-mark {

    animation: boot-in 420ms var(--ease) both;
  }

  .boot-bar {
    position: relative;
    width: 132px;
    height: 2px;
    overflow: hidden;
    border-radius: 2px;
    background: var(--border);
  }

  .boot-bar span {
    position: absolute;
    top: 0;
    left: 0;
    width: 42%;
    height: 100%;
    border-radius: 2px;
    background: var(--accent);
    animation: boot-sweep 1150ms ease-in-out infinite;
  }

  @keyframes boot-in {
    from {
      transform: translateY(6px);
    }
    to {
      transform: none;
    }
  }

  @keyframes boot-sweep {
    0% {
      transform: translateX(-110%);
    }
    100% {
      transform: translateX(345%);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .boot-mark,
    .boot-bar span {
      animation: none;
    }
    .boot-bar span {
      width: 100%;
      opacity: 0.5;
    }
  }
  .sweeping {
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 40px;
    text-align: center;
    background: var(--bg);
  }
  .lead {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    color: var(--text);
  }
  .sub {
    margin: 0;
    max-width: 44ch;
    font-size: 13px;
    color: var(--text-muted);
  }
</style>
