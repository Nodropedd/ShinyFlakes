<script lang="ts">
  import Login from "./routes/Login.svelte";
  import Wallet from "./routes/Wallet.svelte";
  import { session } from "./lib/session.svelte";

  session.refresh();
</script>

{#if session.screen === "loading"}
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
</style>
