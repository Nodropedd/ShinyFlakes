<script lang="ts">
  import Login from "./routes/Login.svelte";
  import Wallet from "./routes/Wallet.svelte";
  import Wordmark from "./lib/Wordmark.svelte";
  import { session } from "./lib/session.svelte";

  session.refresh();
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
  <!-- Boot takes long enough on a phone to look like a crash: the vault is
       read, the credential store is asked for a key, and on a cold start the
       webview itself is still warming up. A blank panel for that long reads
       as a hang, so it says the name and shows that something is moving. -->
  <div class="boot">
    <div class="boot-mark"><Wordmark size={34} /></div>
    <div class="boot-bar"><span></span></div>
  </div>
{:else if session.screen === "wallet"}
  <Wallet />
{:else}
  <Login />
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
    /* Comes up rather than appearing, so a fast boot does not flash. */
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

  /* A travelling segment rather than a percentage: nothing here knows how
     far along it is, and a bar that claims 60% when it cannot tell is a
     worse lie than one that only says "still working". */
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
      opacity: 0;
      transform: translateY(6px);
    }
    to {
      opacity: 1;
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

  /* Respect the system setting: the sweep is decoration, not information. */
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
