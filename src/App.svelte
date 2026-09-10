<script lang="ts">
  import Login from "./routes/Login.svelte";
  import Wallet from "./routes/Wallet.svelte";
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
  <div class="boot"></div>
{:else if session.screen === "wallet"}
  <Wallet />
{:else}
  <Login />
{/if}

<style>
  .boot {
    height: 100%;
    background: var(--bg);
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
