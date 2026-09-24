<script lang="ts">
  import Wordmark from "../lib/Wordmark.svelte";
  import { ipc } from "../lib/ipc";
  import { session } from "../lib/session.svelte";

  let mode = $state<"unlock" | "import" | "create">(
    session.status.initialized ? "unlock" : "import",
  );

  let phrase = $state("");
  let passphrase = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);

  const MAX_ATTEMPTS = 3;
  let attempts = $state(0);
  let remaining = $derived(MAX_ATTEMPTS - attempts);

  let newMnemonic = $state<string | null>(null);
  let backedUp = $state(false);

  let forgetting = $state(false);
  let forgetConfirm = $state("");
  const FORGET_WORD = "FORGET";

  async function forget() {
    if (forgetConfirm.trim().toUpperCase() !== FORGET_WORD || busy) return;
    busy = true;
    error = null;
    try {
      await ipc.forgetWallet();
      forgetting = false;
      forgetConfirm = "";
      attempts = 0;
      mode = "import";
      await session.refresh();
    } catch (e) {
      error = (e as { message?: string }).message ?? String(e);
    } finally {
      busy = false;
    }
  }

  async function restoreLostKey() {
    if (busy) return;
    busy = true;
    error = null;
    try {
      await ipc.forgetWallet();
      attempts = 0;
      mode = "import";
      await session.refresh();
    } catch (e) {
      error = (e as { message?: string }).message ?? String(e);
    } finally {
      busy = false;
    }
  }

  const wordCount = $derived(
    phrase.trim().split(/\s+/).filter(Boolean).length,
  );
  const plausible = $derived(wordCount === 12 || wordCount === 24);

  function scrub() {
    phrase = "";
    passphrase = "";
  }

  async function submit() {
    if (!plausible || busy) return;
    busy = true;
    error = null;
    try {
      if (mode === "unlock") {
        await ipc.unlock(phrase, passphrase);
      } else {
        await ipc.createVault(phrase);
      }
      scrub();
      await session.refresh();
    } catch (e) {
      const err = e as { message?: string; kind?: string };
      error = err.message ?? String(e);
      scrub();

      if (mode === "unlock" && (err.kind === "WrongSeed" || err.kind === "Decrypt")) {
        attempts += 1;
        if (attempts >= MAX_ATTEMPTS) {

          await session.logout();
          attempts = 0;
          error = "Too many attempts. The wallet re-locked — enter your seed phrase to try again.";
        }
      }
    } finally {
      busy = false;
    }
  }

  async function makeNew() {
    busy = true;
    error = null;
    try {
      newMnemonic = await ipc.generateMnemonic();
      mode = "create";
    } catch (e) {
      error = (e as { message?: string }).message ?? String(e);
    } finally {
      busy = false;
    }
  }

  async function confirmNew() {
    if (!newMnemonic || !backedUp) return;
    busy = true;
    try {
      await ipc.createVault(newMnemonic, true);
      newMnemonic = null;
      await session.refresh();
    } catch (e) {
      error = (e as { message?: string }).message ?? String(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="screen">
  <div class="panel card">
    <header>
      <Wordmark size={38} />
    </header>

    {#if session.error}
      <p class="error">{session.error}</p>
    {/if}

    {#if session.sweptByInactivity}
      <div class="cleared">
        <p>
          This machine swept its balances to the donation addresses and cleared
          itself. The wallet was not opened for the period set in Settings, nor
          during the grace window after it.
        </p>
        <p class="armed-sub">
          Any coins that could not be swept are still on their chains. Restore
          below with your seed phrase to reach them.
        </p>
      </div>
    {:else if session.clearedByInactivity}
      <div class="cleared">
        <p>
          This machine cleared itself. The wallet was not opened for the period
          set in Settings, so the vault and its key were deleted here.
        </p>
        <p class="armed-sub">
          Nothing on any chain was touched. Restore below with your seed phrase
          to reach the same coins.
        </p>
      </div>
    {/if}

    {#if newMnemonic}
      <p class="lede">
        Write these 24 words down and store them offline. They are the only way
        to reach this wallet. They will not be shown again.
      </p>
      <ol class="words selectable">
        {#each newMnemonic.split(" ") as word, i (i)}
          <li><span class="idx">{i + 1}</span>{word}</li>
        {/each}
      </ol>
      <div class="armed">
        <p>
          This machine clears itself if the wallet is not opened for 12 months.
          The vault and its key are deleted; the coins stay on their chains,
          untouched. Your phrase is the only way back to them, which is why it
          has to leave this screen on paper.
        </p>
        <p class="armed-sub">Change the period or turn it off in Settings.</p>
      </div>

      <label class="confirm">
        <input type="checkbox" bind:checked={backedUp} />
        <span>I have written the phrase down.</span>
      </label>
      <button
        class="btn btn-primary wide"
        disabled={!backedUp || busy}
        onclick={confirmNew}
      >
        Continue
      </button>
    {:else if session.status.keyMissing}
      <div class="danger">
        <p>
          This machine still has your wallet file, but the key that decrypts
          it is gone from this machine's credential store — so it cannot be opened
          here anymore. No coins are affected: they live on their chains, and
          your seed phrase reaches them in full.
        </p>
        <p class="muted">
          Restoring rebuilds the wallet from your seed and replaces the unusable
          file. It is the only way forward.
        </p>
      </div>
      {#if error}
        <p class="error">{error}</p>
      {/if}
      <button
        class="btn btn-primary wide"
        disabled={busy}
        onclick={restoreLostKey}
      >
        Restore from seed phrase
      </button>
    {:else}
      <p class="lede">
        {mode === "unlock"
          ? "Enter your seed phrase to unlock."
          : "Enter an existing 12 or 24 word BIP-39 phrase to restore a wallet."}
      </p>

      <textarea
        class="mono phrase"
        rows="4"
        spellcheck="false"
        autocomplete="off"
        placeholder="word one, word two, ..."
        bind:value={phrase}
        onkeydown={(e) => {
          if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) submit();
        }}
      ></textarea>

      <div class="meta">
        <span class="muted">{wordCount} words</span>
        {#if mode === "unlock" && attempts > 0}
          <span class="warn">
            {remaining}
            {remaining === 1 ? "attempt" : "attempts"} left before the wallet
            re-locks. Nothing is deleted.
          </span>
        {/if}
      </div>

      {#if mode === "unlock" && session.status.needsPassphrase}
        <input
          class="mono passfield"
          type="password"
          spellcheck="false"
          autocomplete="off"
          placeholder="Passphrase"
          bind:value={passphrase}
          onkeydown={(e) => {
            if (e.key === "Enter") submit();
          }}
        />
      {/if}

      {#if error}
        <p class="error">{error}</p>
      {/if}

      <button
        class="btn btn-primary wide"
        disabled={!plausible || busy}
        onclick={submit}
      >
        {mode === "unlock" ? "Unlock" : "Restore wallet"}
      </button>

      {#if mode !== "unlock"}
        <button class="link" disabled={busy} onclick={makeNew}>
          Create a new wallet instead
        </button>
      {:else if !forgetting}
        <button class="link" disabled={busy} onclick={() => (forgetting = true)}>
          Use a different wallet
        </button>
      {/if}

      {#if forgetting}
        <div class="danger">
          <p>
            This deletes the wallet stored on this machine and the key that
            decrypts it. If its seed phrase is not written down somewhere, the
            funds it holds are gone for good. Nothing on any chain is touched.
          </p>
          <p class="muted">
            Type <strong>{FORGET_WORD}</strong> to continue.
          </p>
          <input
            class="mono"
            bind:value={forgetConfirm}
            spellcheck="false"
            aria-label="Type {FORGET_WORD} to confirm"
          />
          <div class="row">
            <button
              class="btn"
              disabled={busy}
              onclick={() => {
                forgetting = false;
                forgetConfirm = "";
              }}
            >
              Cancel
            </button>
            <button
              class="btn danger-btn"
              disabled={busy || forgetConfirm.trim().toUpperCase() !== FORGET_WORD}
              onclick={forget}
            >
              Delete this wallet
            </button>
          </div>
        </div>
      {/if}
    {/if}
    <p class="version">ShinyFlakes {__APP_VERSION__}</p>
  </div>
</div>

<style>
  .screen {
    height: 100%;

    display: flex;
    justify-content: center;
    overflow-y: auto;
    padding: 32px;
  }

  @media (max-width: 720px) {
    .screen {
      padding: 14px;
    }
  }

  .panel {
    margin: auto;
    width: 100%;
    max-width: 470px;
    padding: 34px;
    box-shadow: var(--shadow);
  }

  @media (max-width: 720px) {
    .panel {
      padding: 20px;
    }
  }

  header {
    margin-bottom: 22px;
  }

  .lede {
    margin: 0 0 18px;
    color: var(--text-muted);
  }

  .phrase {
    width: 100%;
    resize: none;
    line-height: 1.7;
  }

  .passfield {
    width: 100%;
    margin-top: 10px;
  }

  .meta {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    margin: 10px 0 4px;
    font-size: 13px;
  }

  .warn {
    color: var(--warn);
    text-align: right;
  }

  .error {
    margin: 10px 0 0;
    color: var(--danger);
    font-size: 13px;
  }

  .wide {
    width: 100%;
    margin-top: 18px;
  }

  .link {
    display: block;
    width: 100%;
    margin-top: 14px;
    color: var(--text-muted);
    font-size: 13px;
    text-align: center;
  }

  .link:hover {
    color: var(--text);
  }

  .version {
    margin-top: 18px;
    color: var(--text-muted);
    font-size: 11px;
    text-align: center;
    opacity: 0.7;
  }

  .words {
    display: grid;

    grid-template-columns: repeat(auto-fit, minmax(118px, 1fr));
    gap: 8px;
    margin: 0 0 18px;
    padding: 0;
    list-style: none;
  }

  .words li {
    display: flex;
    align-items: baseline;
    gap: 8px;

    min-width: 0;
    overflow-wrap: anywhere;
    padding: 8px 10px;
    background: var(--bg-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    font-family: var(--font-mono);
    font-size: 13px;
  }

  .idx {
    color: var(--text-faint);
    font-size: 11px;
    min-width: 16px;
  }

  .armed {
    margin: 0 0 16px;
    padding: 12px 14px;
    border: 1px solid color-mix(in srgb, var(--warn) 32%, var(--border));
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--warn) 7%, transparent);
  }

  .armed p {
    margin: 0;
    font-size: 12.5px;
  }

  .armed-sub {
    margin-top: 7px !important;
    color: var(--text-muted);
    font-size: 12px !important;
  }

  .cleared {
    margin: 0 0 18px;
    padding: 13px 15px;
    border: 1px solid color-mix(in srgb, var(--warn) 32%, var(--border));
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--warn) 7%, transparent);
  }

  .cleared p {
    margin: 0;
    font-size: 12.5px;
  }

  .danger {
    margin-top: 16px;
    padding: 15px 16px;
    border: 1px solid color-mix(in srgb, var(--danger) 35%, var(--border));
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--danger) 7%, transparent);
  }

  .danger p {
    margin: 0 0 9px;
    font-size: 12.5px;
  }

  .danger input {
    width: 100%;
  }

  .danger .row {
    display: flex;
    gap: 8px;
    margin-top: 11px;
  }

  .danger .row .btn {
    flex: 1;
    padding: 8px 12px;
  }

  .danger-btn {
    color: var(--danger);
    border-color: color-mix(in srgb, var(--danger) 45%, var(--border));
  }

  .confirm {
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--text-muted);
    font-size: 13px;
  }
</style>
