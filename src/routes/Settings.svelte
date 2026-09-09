<script lang="ts">
  import {
    CURRENCIES,
    THEMES,
    settings,
    type CurrencyCode,
    type Theme,
  } from "../lib/settings.svelte";
  import { wallet } from "../lib/wallet.svelte";
  import AssetIcon from "../lib/AssetIcon.svelte";
  import SendDialog from "../lib/SendDialog.svelte";
  import { BY_ID } from "../lib/assets";
  import {
    ipc,
    type MoneroKeys,
    type MoneroSetup,
    type MoneroStatus,
    type AssetId,
    type Donation,
  } from "../lib/ipc";
  import { DEFAULT_MONERO_ENDPOINT } from "../lib/settings.svelte";

  // Monero keys are spending authority, so revealing them is deliberate:
  // typed confirmation, and they are dropped again on leaving the screen.
  const REVEAL_WORD = "REVEAL";
  let askingKeys = $state(false);
  let confirmWord = $state("");
  let moneroKeys = $state<MoneroKeys | null>(null);
  let keysError = $state<string | null>(null);

  async function revealKeys() {
    if (confirmWord.trim().toUpperCase() !== REVEAL_WORD) return;
    keysError = null;
    try {
      moneroKeys = await ipc.revealMoneroKeys();
      askingKeys = false;
      confirmWord = "";
    } catch (e) {
      keysError = (e as { message?: string }).message ?? String(e);
    }
  }

  function hideKeys() {
    moneroKeys = null;
    askingKeys = false;
    confirmWord = "";
    keysError = null;
  }

  $effect(() => () => hideKeys());

  const DEFAULT_ACCENT = "#f2c14e";

  async function pickCurrency(code: CurrencyCode) {
    await wallet.setCurrency(code);
  }

  // Reading the accent back for the colour input: when none is set the input
  // still needs a value, so it shows the dark-theme default.
  const accentValue = $derived(settings.accent ?? DEFAULT_ACCENT);

  // Tipping.
  let donations = $state<Donation[]>([]);
  let tipping = $state<Donation | null>(null);

  // Only chains this wallet can actually sign for. Monero is included once
  // its daemon is up, same as anywhere else.
  const SENDABLE_TIPS = $derived<AssetId[]>(
    settings.moneroReady
      ? ["BTC", "LTC", "ETH", "SOL", "XMR"]
      : ["BTC", "LTC", "ETH", "SOL"],
  );

  $effect(() => {
    ipc
      .donationAddresses()
      .then((list) => (donations = list))
      .catch(() => {
        /* the section simply stays empty */
      });
  });

  // One-button Monero setup.
  let setup = $state<MoneroSetup | null>(null);
  let settingUp = $state(false);
  let setupStep = $state("Working");
  let setupError = $state<string | null>(null);
  let daemonAddress = $state(settings.moneroDaemon);

  $effect(() => {
    ipc
      .moneroSetupState()
      .then((s) => (setup = s))
      .catch(() => {
        /* the button will report anything that matters */
      });
  });

  async function runSetup() {
    settingUp = true;
    setupError = null;
    setupStep = setup?.installed ? "Starting" : "Downloading Monero";
    try {
      settings.setMoneroDaemon(daemonAddress);
      const result = await ipc.moneroSetupRun(settings.moneroDaemon);
      setup = result;
      if (result.running) {
        // The bridge always listens here, so the endpoint is not a question
        // the user should have to answer.
        settings.setMoneroEndpoint(DEFAULT_MONERO_ENDPOINT);
        await wallet.refresh();
      }
    } catch (e) {
      setupError = (e as { message?: string }).message ?? String(e);
    } finally {
      settingUp = false;
    }
  }

  async function stopMonero() {
    await wallet.stopMonero();
    setup = await ipc.moneroSetupState();
  }

  // Manual bridge, kept for anyone already running their own daemon.
  let endpoint = $state(settings.moneroEndpoint || DEFAULT_MONERO_ENDPOINT);
  let nodeStatus = $state<MoneroStatus | null>(null);
  let nodeError = $state<string | null>(null);
  let checkingNode = $state(false);

  async function connectMonero() {
    checkingNode = true;
    nodeError = null;
    nodeStatus = null;
    try {
      const result = await ipc.xmrStatus(endpoint.trim());
      nodeStatus = result;
      // Only remember an endpoint that actually answered for this account.
      if (result.matchesWallet) {
        settings.setMoneroEndpoint(endpoint.trim());
        await wallet.refresh();
      }
    } catch (e) {
      nodeError = (e as { message?: string }).message ?? String(e);
    } finally {
      checkingNode = false;
    }
  }

  function disconnectMonero() {
    settings.setMoneroEndpoint("");
    nodeStatus = null;
    nodeError = null;
  }
</script>

<div class="view">
  <header>
    <h1>Settings</h1>
  </header>

  <section class="card">
    <h2>Appearance</h2>
    <p class="muted">
      Light and dark are applied immediately and remembered. Matching the
      system follows the operating system setting as it changes.
    </p>

    <div class="themes">
      {#each THEMES as option (option.value)}
        <button
          class="theme"
          class:active={settings.theme === option.value}
          onclick={() => settings.setTheme(option.value as Theme)}
        >
          {option.label}
        </button>
      {/each}
    </div>

    <div class="divider"></div>

    <p class="muted">
      The accent colour used on buttons, focus rings and the selected section.
      The ShinyFlakes mark stays gold in both themes.
    </p>
    <div class="control">
      <input
        type="color"
        value={accentValue}
        oninput={(e) =>
          settings.setAccent((e.currentTarget as HTMLInputElement).value)}
        aria-label="Accent colour"
      />
      <code class="mono selectable">
        {settings.accent ?? "theme default"}
      </code>
      <button
        class="btn"
        onclick={() => settings.setAccent(null)}
        disabled={settings.accent === null}
      >
        Reset
      </button>
    </div>
  </section>

  <section class="card">
    <h2>Currency</h2>
    <p class="muted">
      What balances are valued in. Prices are fetched in this currency rather
      than converted locally.
    </p>
    <div class="control">
      <select
        value={settings.currency}
        onchange={(e) =>
          pickCurrency((e.currentTarget as HTMLSelectElement).value as CurrencyCode)}
        aria-label="Display currency"
      >
        {#each CURRENCIES as c (c.code)}
          <option value={c.code}>{c.label} ({c.symbol})</option>
        {/each}
      </select>
    </div>
  </section>

  <section class="card">
    <h2>Monero wallet</h2>
    <p class="muted">
      Monero balances cannot be read from an address, and spending needs ring
      signatures. So this wallet runs Monero's own wallet daemon on this
      machine and talks to it over loopback. Your keys go straight from memory
      into that process: nothing is written to a file or shown on screen, and
      only local addresses are ever accepted.
    </p>

    {#if wallet.moneroStarting}
      <p class="hint-note">Starting Monero.</p>
    {:else if setup?.running || wallet.moneroRunning}
      <p class="ok-note">
        Running{setup ? ` Monero ${setup.version}` : ""}. Balances and sending are live.
        <button class="inline" onclick={stopMonero}>Stop</button>
      </p>
    {:else}
      <div class="control">
        <button class="btn btn-primary" onclick={runSetup} disabled={settingUp}>
          {settingUp
            ? setupStep
            : setup?.installed
              ? "Start Monero"
              : "Set up Monero"}
        </button>
      </div>
      {#if !setup?.installed}
        <p class="hint-note">
          First time only: this downloads about 90 MB from getmonero.org and
          checks it against a hash built into this program. Then it scans the
          chain, which takes a while.
        </p>
      {/if}
    {/if}

    {#if setupError}
      <p class="kerr">{setupError}</p>
    {/if}

    <details class="how">
      <summary>Use a different node, or one of your own</summary>
      <p>
        The chain is read through a public node by default. It never sees your
        keys and cannot spend anything, but it does see this machine's address
        and which parts of the chain are requested. Running your own removes
        that, at the cost of a long sync.
      </p>
      <div class="control">
        <input
          class="mono"
          bind:value={daemonAddress}
          spellcheck="false"
          aria-label="Monero node address"
        />
      </div>
      <p>
        Change this before pressing the button above. Point it at
        <code class="mono">127.0.0.1:18081</code> once your own node has caught up.
      </p>
    </details>
  </section>

  <section class="card pending" hidden>
    <h2>Manual bridge</h2>

    <div class="control">
      <input
        class="mono"
        bind:value={endpoint}
        spellcheck="false"
        aria-label="Monero wallet RPC address"
      />
      <button class="btn" onclick={connectMonero} disabled={checkingNode}>
        {checkingNode ? "Checking" : "Connect"}
      </button>
    </div>

    {#if nodeError}
      <p class="kerr">{nodeError}</p>
    {/if}

    {#if nodeStatus}
      {#if nodeStatus.matchesWallet}
        <p class="ok-note">
          Connected. Synced to block {nodeStatus.height.toLocaleString()}. Monero
          balances and sending are live.
        </p>
      {:else}
        <p class="kerr">
          That daemon is serving a different account. It reports
          <span class="mono">{nodeStatus.address.slice(0, 16)}…</span>, which is not the
          one this seed derives, so it was not saved. Restore the wallet from the keys
          below first.
        </p>
      {/if}
    {:else if settings.moneroReady}
      <p class="ok-note">
        Configured at <span class="mono">{settings.moneroEndpoint}</span>.
        <button class="inline" onclick={disconnectMonero}>Disconnect</button>
      </p>
    {/if}

    <details class="how">
      <summary>How to set this up</summary>
      <p>
        Install the Monero CLI tools, then restore this account from the keys below
        using <code class="mono">monero-wallet-cli --generate-from-keys</code>. It asks
        for the address, the spend key and the view key in that order.
      </p>
      <p>Then run the daemon and point this box at it:</p>
      <pre class="mono">monero-wallet-rpc --wallet-file YOUR_WALLET \
  --password YOUR_PASSWORD \
  --rpc-bind-port 18082 \
  --disable-rpc-login \
  --daemon-address YOUR_NODE:18081</pre>
      <p>
        The first scan after a restore can take a long time. Giving it the block
        height from when you first received Monero here makes it much faster.
      </p>
    </details>
  </section>

  <section class="card">
    <h2>Monero keys</h2>
    <p class="muted">
      Monero cannot be restored from your seed phrase, because mapping a BIP-39
      phrase to Monero keys is this wallet's own convention rather than a
      standard. These two keys are the only way to reach the account from the
      official Monero wallet or any other, so write them down alongside your
      phrase if you hold Monero here.
    </p>

    {#if moneroKeys}
      <div class="keys">
        <p class="klabel">Address</p>
        <p class="mono kval selectable">{moneroKeys.address}</p>

        <p class="klabel">Private spend key</p>
        <p class="mono kval selectable danger-text">{moneroKeys.spendKey}</p>

        <p class="klabel">Private view key</p>
        <p class="mono kval selectable">{moneroKeys.viewKey}</p>

        <p class="knote">
          The spend key is the money. Anyone who reads it can take the funds.
          The view key only lets someone watch the account, which is what a
          scanning node needs.
        </p>
        <p class="knote">
          To restore elsewhere, use the official wallet's restore from keys
          option. It will ask for a starting block height;
          {moneroKeys.restoreHeightHint} is the answer.
        </p>
        <button class="btn" onclick={hideKeys}>Hide</button>
      </div>
    {:else if askingKeys}
      <div class="confirm-box">
        <p class="muted">
          These will appear on screen. Make sure nobody is watching and that this
          machine is not being recorded. Type <strong>{REVEAL_WORD}</strong> to continue.
        </p>
        <input class="mono" bind:value={confirmWord} spellcheck="false" />
        {#if keysError}
          <p class="kerr">{keysError}</p>
        {/if}
        <div class="crow">
          <button class="btn" onclick={hideKeys}>Cancel</button>
          <button
            class="btn btn-primary"
            disabled={confirmWord.trim().toUpperCase() !== REVEAL_WORD}
            onclick={revealKeys}
          >
            Show keys
          </button>
        </div>
      </div>
    {:else}
      <div class="control">
        <button class="btn" onclick={() => (askingKeys = true)}>Show Monero keys</button>
      </div>
    {/if}
  </section>

  <section class="card pending">
    <h2>Email notifications <span class="tag">Not built</span></h2>
    <p class="muted">
      A direct SMTP connection to your own mail provider, with the app password
      stored encrypted on this machine. Sends a notice whenever the seed phrase
      is revealed, and carries two-factor codes.
    </p>
  </section>

  <section class="card pending">
    <h2>Two-factor authentication <span class="tag">Not built</span></h2>
    <p class="muted">
      Six digit codes by email, held in memory with a short expiry. Three wrong
      codes trigger a one minute lockout, five abandon the attempt and fall back
      to the seed phrase. Gates seed reveal, key reveal and seed rotation.
    </p>
  </section>

  <section class="card pending">
    <h2>Bucket rules <span class="tag">Not built</span></h2>
    <p class="muted">
      How incoming funds are split across buckets by default, and which bucket
      a shortfall is pulled from first.
    </p>
  </section>

  <section class="card">
    <h2>Tip the creator</h2>
    <p class="muted">
      These addresses are fixed and compiled into the program. Nothing is ever
      sent without you choosing an amount and confirming, exactly like any
      other payment.
    </p>

    {#if donations.length > 0}
      <ul class="tips">
        {#each donations as tip (tip.asset)}
          {@const meta = BY_ID[tip.asset]}
          {@const canSend = SENDABLE_TIPS.includes(tip.asset)}
          <li>
            <button
              class="tip"
              disabled={!canSend}
              title={canSend ? "" : `Sending ${meta.name} is not implemented yet`}
              onclick={() => (tipping = tip)}
            >
              <AssetIcon asset={meta} size={26} />
              <span class="tname">
                {meta.name}
                {#if tip.host}<span class="on">on {tip.host}</span>{/if}
              </span>
              <span class="taddr mono">{tip.address.slice(0, 10)}…{tip.address.slice(-6)}</span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
</div>

{#if tipping}
  <SendDialog
    initial={tipping.asset}
    presetTo={tipping.address}
    presetNote="The creator's {BY_ID[tipping.asset].name} address, built into this program."
    onclose={() => (tipping = null)}
    onsent={() => void wallet.refresh()}
  />
{/if}

<style>
  .view { display: flex; flex-direction: column; gap: 14px; max-width: 640px; }
  .tips { display: flex; flex-direction: column; gap: 6px; margin: 14px 0 0; padding: 0; list-style: none; }
  .tip {
    display: flex; align-items: center; gap: 11px; width: 100%;
    padding: 9px 11px; text-align: left;
    border: 1px solid var(--border); border-radius: var(--radius-sm);
    background: var(--bg-raised); font-size: 13px;
  }
  .tip:hover:not(:disabled) { background: var(--card-hover); }
  .tip:disabled { opacity: 0.45; cursor: not-allowed; }
  .tname { display: flex; align-items: baseline; gap: 6px; }
  .on {
    padding: 1px 6px; border-radius: 999px; border: 1px solid var(--border-strong);
    color: var(--text-faint); font-size: 10px; font-weight: 600;
  }
  .taddr { margin-left: auto; font-size: 11.5px; color: var(--text-faint); }
  h1 { margin: 0; font-size: 20px; font-weight: 650; }
  section { padding: 20px 22px; }
  h2 {
    display: flex; align-items: center; gap: 10px;
    margin: 0 0 6px; font-size: 14.5px; font-weight: 600;
  }
  section p { margin: 0; font-size: 13px; }
  .pending { opacity: 0.72; }
  .tag {
    padding: 2px 7px; border-radius: 999px;
    border: 1px solid var(--border-strong);
    color: var(--text-faint);
    font-size: 10.5px; font-weight: 600;
    letter-spacing: 0.03em; text-transform: uppercase;
  }

  .themes { display: flex; gap: 8px; margin-top: 14px; }
  .theme {
    flex: 1; padding: 9px 12px; border-radius: var(--radius-sm);
    border: 1px solid var(--border); background: var(--bg-raised);
    color: var(--text-muted); font-size: 13px; font-weight: 500;
    transition: border-color 120ms var(--ease), color 120ms var(--ease);
  }
  .theme:hover { color: var(--text); }
  .theme.active {
    border-color: var(--accent);
    color: var(--text);
    box-shadow: 0 0 0 1px var(--accent) inset;
  }

  .divider { height: 1px; background: var(--border); margin: 20px 0 16px; }

  .control { display: flex; align-items: center; gap: 12px; margin-top: 14px; }

  .keys { margin-top: 16px; }
  .klabel { margin: 12px 0 3px; font-size: 11.5px; color: var(--text-muted); }
  .klabel:first-child { margin-top: 0; }
  .kval {
    margin: 0; padding: 9px 11px; font-size: 12px; word-break: break-all;
    border: 1px solid var(--border); border-radius: var(--radius-sm);
    background: var(--bg-raised);
  }
  .danger-text { color: var(--danger); border-color: color-mix(in srgb, var(--danger) 35%, var(--border)); }
  .knote { margin: 12px 0 0; font-size: 12px; color: var(--text-muted); }
  .keys .btn { margin-top: 16px; }

  .confirm-box {
    margin-top: 16px; padding: 15px 16px;
    border: 1px solid color-mix(in srgb, var(--warn) 35%, var(--border));
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--warn) 6%, transparent);
  }
  .confirm-box p { margin: 0 0 10px; font-size: 12.5px; }
  .confirm-box input { width: 100%; }
  .crow { display: flex; gap: 8px; margin-top: 11px; }
  .crow .btn { flex: 1; }
  .kerr { margin: 8px 0 0 !important; color: var(--danger); font-size: 12px; }
  .ok-note { margin: 10px 0 0 !important; color: var(--ok); font-size: 12px; }
  .inline { color: var(--text-muted); font-size: 12px; padding: 0 0 0 6px; text-decoration: underline; }
  .inline:hover { color: var(--text); }
  .how { margin-top: 14px; }
  .how summary { font-size: 12.5px; color: var(--text-muted); cursor: pointer; }
  .how summary:hover { color: var(--text); }
  .how p { margin: 10px 0 0 !important; font-size: 12.5px; color: var(--text-muted); }
  .hint-note { margin: 10px 0 0 !important; font-size: 12px; color: var(--text-faint); }
  .how pre {
    margin: 8px 0 0; padding: 11px 12px; overflow-x: auto;
    border: 1px solid var(--border); border-radius: var(--radius-sm);
    background: var(--bg-raised); font-size: 11.5px; line-height: 1.6;
  }
  code { font-size: 12px; }
  select {
    flex: 1;
    background: var(--bg-raised); color: var(--text);
    border: 1px solid var(--border); border-radius: var(--radius-sm);
    padding: 9px 11px; font: inherit;
  }
  input[type="color"] {
    width: 42px; height: 32px; padding: 2px;
    border-radius: var(--radius-sm); cursor: pointer;
  }
  code { font-size: 12.5px; color: var(--text-muted); }
  .control .btn { margin-left: auto; padding: 7px 14px; }
</style>
