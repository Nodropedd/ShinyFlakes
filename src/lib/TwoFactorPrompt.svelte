<script lang="ts">
  // The email two-factor gate, shown before a sensitive reveal when two-factor
  // is on. It sends a code on open, checks what the user types against the
  // attempt ladder in the core, and offers the seed phrase as a fallback for
  // when the code cannot be received. On success it calls onpassed; the caller
  // then re-runs the reveal, which now finds a valid pass waiting.
  import { onMount } from "svelte";
  import { ipc } from "./ipc";

  let {
    purpose,
    onpassed,
    oncancel,
  }: {
    /** A short phrase for the copy, e.g. "reveal your seed phrase". */
    purpose: string;
    onpassed: () => void;
    oncancel: () => void;
  } = $props();

  let mode = $state<"code" | "seed">("code");
  let code = $state("");
  let seedInput = $state("");
  let sending = $state(false);
  let verifying = $state(false);
  let message = $state<string | null>(null);
  let error = $state<string | null>(null);
  let lockedUntil = $state<number | null>(null);
  let now = $state(Math.floor(Date.now() / 1000));

  // A clock, running only while a lockout counts down, so the button can
  // re-enable itself the moment the minute is up.
  $effect(() => {
    if (lockedUntil == null) return;
    const t = setInterval(() => (now = Math.floor(Date.now() / 1000)), 500);
    return () => clearInterval(t);
  });

  const lockRemaining = $derived(
    lockedUntil != null ? Math.max(0, lockedUntil - now) : 0,
  );
  const locked = $derived(lockRemaining > 0);

  async function sendCode() {
    if (sending) return;
    sending = true;
    error = null;
    message = null;
    try {
      const res = await ipc.request2fa();
      message = `A code was sent to ${res.sentTo}. It is good for five minutes.`;
    } catch (e) {
      // If the code cannot be sent at all, the seed fallback is the way in.
      error =
        ((e as { message?: string }).message ?? String(e)) +
        " You can use your seed phrase instead.";
    } finally {
      sending = false;
    }
  }

  onMount(sendCode);

  async function submitCode() {
    if (verifying || locked || code.trim().length === 0) return;
    verifying = true;
    error = null;
    message = null;
    try {
      const res = await ipc.verify2fa(code.trim());
      switch (res.status) {
        case "ok":
          onpassed();
          return;
        case "wrong":
          error = `That code is wrong. ${res.remaining ?? 0} ${
            res.remaining === 1 ? "try" : "tries"
          } left.`;
          code = "";
          break;
        case "lockedOut":
          lockedUntil = res.lockedUntil;
          now = Math.floor(Date.now() / 1000);
          error =
            "Too many wrong tries. Wait a minute, then the same code still works.";
          code = "";
          break;
        case "expired":
          error = "That code expired. Send a new one.";
          break;
        case "abandoned":
          error = "That code is used up. Use your seed phrase instead.";
          mode = "seed";
          break;
        case "none":
          error = "No code is waiting. Send one first.";
          break;
      }
    } catch (e) {
      error = (e as { message?: string }).message ?? String(e);
    } finally {
      verifying = false;
    }
  }

  async function submitSeed() {
    if (verifying || seedInput.trim().length === 0) return;
    verifying = true;
    error = null;
    try {
      const ok = await ipc.verify2faSeed(seedInput.trim());
      seedInput = "";
      if (ok) {
        onpassed();
      } else {
        error = "That phrase does not match this wallet.";
      }
    } catch (e) {
      error = (e as { message?: string }).message ?? String(e);
    } finally {
      verifying = false;
    }
  }
</script>

<div
  class="scrim"
  role="button"
  tabindex="-1"
  onclick={oncancel}
  onkeydown={(e) => e.key === "Escape" && oncancel()}
>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="panel card" onclick={(e) => e.stopPropagation()}>
    <header>
      <h2>Confirm it's you</h2>
      <button class="x" onclick={oncancel} aria-label="Close">&times;</button>
    </header>

    <p class="lede muted">
      Two-factor is on, so to {purpose} you need the code sent to your inbox.
    </p>

    {#if mode === "code"}
      <label class="field">
        <span>Six-digit code</span>
        <input
          class="mono"
          bind:value={code}
          inputmode="numeric"
          maxlength="6"
          placeholder="000000"
          spellcheck="false"
          onkeydown={(e) => e.key === "Enter" && submitCode()}
        />
      </label>

      {#if locked}
        <p class="hint">Locked for {lockRemaining}s.</p>
      {/if}
      {#if error}<p class="err">{error}</p>{/if}
      {#if message && !error}<p class="ok">{message}</p>{/if}

      <div class="row">
        <button class="btn" onclick={sendCode} disabled={sending}>
          {sending ? "Sending" : "Resend code"}
        </button>
        <button
          class="btn btn-primary"
          onclick={submitCode}
          disabled={verifying || locked || code.trim().length === 0}
        >
          {verifying ? "Checking" : "Confirm"}
        </button>
      </div>

      <button
        class="link"
        onclick={() => {
          mode = "seed";
          error = null;
        }}
      >
        Can't get the code? Use your seed phrase
      </button>
    {:else}
      <label class="field">
        <span>Seed phrase</span>
        <textarea
          class="mono"
          rows="3"
          bind:value={seedInput}
          spellcheck="false"
          placeholder="your twelve or twenty-four words"
        ></textarea>
      </label>
      <p class="hint">
        The seed is the wallet, so proving it is as strong as the code. It is
        checked on this machine and goes nowhere.
      </p>
      {#if error}<p class="err">{error}</p>{/if}

      <div class="row">
        <button
          class="btn"
          onclick={() => {
            mode = "code";
            error = null;
          }}
        >
          Back to code
        </button>
        <button
          class="btn btn-primary"
          onclick={submitSeed}
          disabled={verifying || seedInput.trim().length === 0}
        >
          {verifying ? "Checking" : "Confirm"}
        </button>
      </div>
    {/if}
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    display: grid;
    place-items: center;
    padding: 24px;
    background: rgba(0, 0, 0, 0.55);
    z-index: 60;
    border: 0;
  }
  .panel {
    width: 100%;
    max-width: 400px;
    padding: 20px 22px 22px;
    box-shadow: var(--shadow);
    text-align: left;
    cursor: default;
  }
  header {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 12px;
  }
  h2 {
    margin: 0;
    font-size: 15.5px;
    font-weight: 650;
  }
  .x {
    margin-left: auto;
    color: var(--text-muted);
    font-size: 20px;
    line-height: 1;
    padding: 0 4px;
  }
  .x:hover {
    color: var(--text);
  }
  .lede {
    margin: 0 0 14px;
    font-size: 12.5px;
  }
  .field {
    display: block;
    margin-bottom: 4px;
  }
  .field span {
    display: block;
    margin-bottom: 5px;
    font-size: 12.5px;
    color: var(--text-muted);
  }
  .field input,
  .field textarea {
    width: 100%;
    resize: vertical;
  }
  .field input {
    letter-spacing: 0.35em;
    font-size: 16px;
    text-align: center;
  }
  .hint {
    margin: 8px 0 0;
    font-size: 12px;
    color: var(--text-faint);
  }
  .err {
    margin: 8px 0 0;
    font-size: 12px;
    color: var(--danger);
  }
  .ok {
    margin: 8px 0 0;
    font-size: 12px;
    color: var(--ok);
  }
  .row {
    display: flex;
    gap: 8px;
    margin-top: 14px;
  }
  .row .btn {
    flex: 1;
  }
  .link {
    display: block;
    margin: 12px auto 0;
    color: var(--text-muted);
    font-size: 12px;
    text-decoration: underline;
  }
  .link:hover {
    color: var(--text);
  }
</style>
