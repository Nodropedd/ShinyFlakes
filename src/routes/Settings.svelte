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
  import TwoFactorPrompt from "../lib/TwoFactorPrompt.svelte";
  import { BY_ID } from "../lib/assets";
  import {
    ipc,
    type MoneroKeys,
    type MoneroSetup,
    type MoneroStatus,
    type AssetId,
    type Donation,
    type Inactivity,
    type EmailConfig,
  } from "../lib/ipc";
  import { session } from "../lib/session.svelte";
  import { DEFAULT_MONERO_ENDPOINT } from "../lib/settings.svelte";

  // Sensitive reveals are deliberate: a typed confirmation, then two-factor
  // when it is on, and the secret is dropped again on leaving the screen.
  const REVEAL_WORD = "REVEAL";

  // A reveal waiting on two-factor. When set, the prompt is shown; passing it
  // re-runs the stored action, which now finds a valid pass in the core.
  let twoFactorFor = $state<
    null | { purpose: string; run: () => Promise<void> }
  >(null);

  // Runs a reveal, and if the core answers that two-factor is needed, opens the
  // prompt instead of failing. Every other error the run itself reports.
  async function guarded(purpose: string, run: () => Promise<void>) {
    try {
      await run();
    } catch (e) {
      if ((e as { kind?: string }).kind === "TwoFactorRequired") {
        twoFactorFor = { purpose, run };
      } else {
        throw e;
      }
    }
  }

  async function twoFactorPassed() {
    const pending = twoFactorFor;
    twoFactorFor = null;
    // The run reports its own non-2FA errors; a repeat 2FA prompt would be an
    // odd edge, so it is simply swallowed and the user can retry.
    if (pending) await pending.run().catch(() => {});
  }

  // Monero keys.
  let askingKeys = $state(false);
  let confirmWord = $state("");
  let moneroKeys = $state<MoneroKeys | null>(null);
  let keysError = $state<string | null>(null);

  async function doRevealKeys() {
    keysError = null;
    try {
      moneroKeys = await ipc.revealMoneroKeys();
      askingKeys = false;
      confirmWord = "";
    } catch (e) {
      if ((e as { kind?: string }).kind === "TwoFactorRequired") throw e;
      keysError = (e as { message?: string }).message ?? String(e);
    }
  }

  async function revealKeys() {
    if (confirmWord.trim().toUpperCase() !== REVEAL_WORD) return;
    await guarded("reveal your Monero keys", doRevealKeys);
  }

  function hideKeys() {
    moneroKeys = null;
    askingKeys = false;
    confirmWord = "";
    keysError = null;
  }

  // Seed phrase.
  let askingSeed = $state(false);
  let seedWord = $state("");
  let seedPhrase = $state<string | null>(null);
  let seedError = $state<string | null>(null);

  async function doRevealSeed() {
    seedError = null;
    try {
      seedPhrase = await ipc.revealSeed();
      askingSeed = false;
      seedWord = "";
    } catch (e) {
      if ((e as { kind?: string }).kind === "TwoFactorRequired") throw e;
      seedError = (e as { message?: string }).message ?? String(e);
    }
  }

  async function revealSeed() {
    if (seedWord.trim().toUpperCase() !== REVEAL_WORD) return;
    await guarded("reveal your seed phrase", doRevealSeed);
  }

  function hideSeed() {
    seedPhrase = null;
    askingSeed = false;
    seedWord = "";
    seedError = null;
  }

  // Nothing sensitive lingers when the screen is left.
  $effect(() => () => {
    hideKeys();
    hideSeed();
    twoFactorFor = null;
  });

  // Email and two-factor.
  let email = $state<EmailConfig | null>(null);
  let smtpHost = $state("");
  let smtpPort = $state(587);
  let smtpUser = $state("");
  let smtpPass = $state("");
  let smtpFrom = $state("");
  let emailBusy = $state(false);
  let testing = $state(false);
  let emailError = $state<string | null>(null);
  let emailOk = $state<string | null>(null);
  let twoFactorBusy = $state(false);
  let twoFactorError = $state<string | null>(null);

  $effect(() => {
    ipc
      .emailConfig()
      .then((c) => {
        email = c;
        smtpHost = c.host;
        smtpPort = c.port;
        smtpUser = c.username;
        smtpFrom = c.from;
      })
      .catch(() => {
        /* the section shows its empty state */
      });
  });

  async function saveEmail() {
    if (emailBusy) return;
    emailBusy = true;
    emailError = null;
    emailOk = null;
    try {
      email = await ipc.setEmailConfig(
        smtpHost,
        smtpPort,
        smtpUser,
        smtpPass.length ? smtpPass : null,
        smtpFrom,
      );
      smtpPass = "";
      emailOk = "Saved.";
    } catch (e) {
      emailError = (e as { message?: string }).message ?? String(e);
    } finally {
      emailBusy = false;
    }
  }

  async function testEmail() {
    if (testing) return;
    testing = true;
    emailError = null;
    emailOk = null;
    try {
      await ipc.sendTestEmail();
      emailOk = "Test message sent. Check your inbox.";
    } catch (e) {
      emailError = (e as { message?: string }).message ?? String(e);
    } finally {
      testing = false;
    }
  }

  async function toggleTwoFactor() {
    if (twoFactorBusy) return;
    twoFactorBusy = true;
    twoFactorError = null;
    try {
      email = await ipc.setTwoFactor(!(email?.twoFactor ?? false));
    } catch (e) {
      twoFactorError = (e as { message?: string }).message ?? String(e);
    } finally {
      twoFactorBusy = false;
    }
  }

  const DEFAULT_ACCENT = "#f2c14e";

  async function pickCurrency(code: CurrencyCode) {
    await wallet.setCurrency(code);
  }

  // Reading the accent back for the colour input: when none is set the input
  // still needs a value, so it shows the dark-theme default.
  const accentValue = $derived(settings.accent ?? DEFAULT_ACCENT);

  // Vault passphrase, a second factor on top of the seed.
  let curPass = $state("");
  let newPass = $state("");
  let confirmPass = $state("");
  let passBusy = $state(false);
  let passError = $state<string | null>(null);
  let passDone = $state<string | null>(null);

  const hasPassphrase = $derived(session.status.needsPassphrase);

  async function savePassphrase(remove: boolean) {
    if (passBusy) return;
    passError = null;
    passDone = null;
    if (!remove && newPass !== confirmPass) {
      passError = "The two new entries do not match.";
      return;
    }
    passBusy = true;
    try {
      await ipc.setVaultPassphrase(
        hasPassphrase ? curPass : null,
        remove ? null : newPass,
      );
      curPass = "";
      newPass = "";
      confirmPass = "";
      passDone = remove
        ? "Passphrase removed."
        : "Passphrase set. You will need it next time you unlock.";
      await session.refresh();
    } catch (e) {
      passError = (e as { message?: string }).message ?? String(e);
    } finally {
      passBusy = false;
    }
  }

  // Clearing this machine after a long absence.
  const PERIODS: { months: number; label: string }[] = [
    { months: 0, label: "Never" },
    { months: 3, label: "3 months" },
    { months: 6, label: "6 months" },
    { months: 12, label: "12 months" },
    { months: 24, label: "24 months" },
  ];

  let inactivity = $state<Inactivity | null>(null);

  $effect(() => {
    ipc
      .inactivityCheck()
      .then((i) => (inactivity = i))
      .catch(() => {
        /* the section stays hidden rather than guessing */
      });
  });

  async function setPeriod(months: number) {
    try {
      inactivity = await ipc.inactivitySetMonths(months);
    } catch (e) {
      deleteError = (e as { message?: string }).message ?? String(e);
    }
  }

  async function setAction(action: "delete" | "donate") {
    try {
      inactivity = await ipc.inactivitySetAction(action);
    } catch (e) {
      deleteError = (e as { message?: string }).message ?? String(e);
    }
  }

  // Deleting this wallet by hand.
  const DELETE_WORD = "DELETE";
  let askingDelete = $state(false);
  let deleteWord = $state("");
  let deleteError = $state<string | null>(null);

  async function deleteWallet() {
    if (deleteWord.trim().toUpperCase() !== DELETE_WORD) return;
    deleteError = null;
    try {
      await ipc.forgetWallet();
      askingDelete = false;
      deleteWord = "";
      await session.refresh();
    } catch (e) {
      deleteError = (e as { message?: string }).message ?? String(e);
    }
  }

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
    <h2>Vault passphrase</h2>
    <p class="muted">
      Without one, the vault decrypts using a key in the Windows credential
      store, so anyone with this OS account can read your seed. A passphrase
      mixes into that key, so decrypting then needs both this account and
      something only you know.
    </p>
    <p class="warn-note">
      It is not recoverable. Forget it and this machine's wallet is unreadable;
      only your seed phrase brings the funds back. Setting one also turns off
      the donate-on-inactivity switch, since an unattended sweep cannot ask for
      it.
    </p>

    {#if hasPassphrase}
      <label class="pfield">
        <span>Current passphrase</span>
        <input class="mono" type="password" bind:value={curPass} spellcheck="false" />
      </label>
    {/if}
    <label class="pfield">
      <span>{hasPassphrase ? "New passphrase" : "Passphrase"}</span>
      <input class="mono" type="password" bind:value={newPass} spellcheck="false" />
    </label>
    <label class="pfield">
      <span>Confirm</span>
      <input class="mono" type="password" bind:value={confirmPass} spellcheck="false" />
    </label>

    {#if passError}<p class="kerr">{passError}</p>{/if}
    {#if passDone}<p class="ok-note">{passDone}</p>{/if}

    <div class="crow">
      <button
        class="btn btn-primary"
        disabled={passBusy || newPass.length === 0}
        onclick={() => savePassphrase(false)}
      >
        {hasPassphrase ? "Change passphrase" : "Set passphrase"}
      </button>
      {#if hasPassphrase}
        <button class="btn" disabled={passBusy} onclick={() => savePassphrase(true)}>
          Remove
        </button>
      {/if}
    </div>
  </section>

  <section class="card">
    <h2>Route through Tor</h2>
    <p class="muted">
      Every balance, price and history lookup goes to a public server that
      otherwise sees this machine's address alongside the wallet addresses it
      asks about. Tor hides the first half: the server sees a Tor exit, not you.
      It does nothing about what the addresses reveal on chain, which is a
      separate matter, but it is the largest network leak.
    </p>

    {#if wallet.torStarting}
      <p class="hint-note">Connecting to Tor. The first time also downloads it.</p>
    {:else if wallet.torRouting}
      <p class="ok-note">
        Routing through Tor. Lookups no longer reveal your address.
        <button class="inline" onclick={() => wallet.stopTor()}>Turn off</button>
      </p>
    {:else}
      <div class="control">
        <button class="btn btn-primary" onclick={() => wallet.startTor()}>
          Route through Tor
        </button>
      </div>
      <p class="hint-note">
        First time only: about 20 MB from torproject.org, checked against a hash
        built into this program. Connecting then takes a moment.
      </p>
    {/if}

    {#if wallet.torError}
      <p class="kerr">{wallet.torError}</p>
    {/if}
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

  <section class="card">
    <h2>Email notifications</h2>
    <p class="muted">
      A direct connection to your own mail provider over TLS. The app password
      is stored encrypted on this machine and goes nowhere but your server.
      Mail is sent to this same address: a notice whenever the seed phrase or a
      key is revealed, and the two-factor codes below.
    </p>
    <p class="hint-note">
      Use an app-specific password, not your main one. For Gmail the server is
      <code class="mono">smtp.gmail.com</code> on port 587.
    </p>

    <label class="pfield">
      <span>SMTP server</span>
      <input class="mono" bind:value={smtpHost} spellcheck="false" placeholder="smtp.gmail.com" />
    </label>
    <div class="pair">
      <label class="pfield">
        <span>Port</span>
        <input class="mono" type="number" bind:value={smtpPort} />
      </label>
      <label class="pfield grow">
        <span>Username</span>
        <input class="mono" bind:value={smtpUser} spellcheck="false" placeholder="you@gmail.com" />
      </label>
    </div>
    <label class="pfield">
      <span>Your email address</span>
      <input class="mono" bind:value={smtpFrom} spellcheck="false" placeholder="you@gmail.com" />
    </label>
    <label class="pfield">
      <span>App password {#if email?.hasPassword}<em class="stored">stored — leave blank to keep</em>{/if}</span>
      <input class="mono" type="password" bind:value={smtpPass} spellcheck="false" />
    </label>

    {#if emailError}<p class="kerr">{emailError}</p>{/if}
    {#if emailOk}<p class="ok-note">{emailOk}</p>{/if}

    <div class="crow">
      <button class="btn btn-primary" disabled={emailBusy} onclick={saveEmail}>
        {emailBusy ? "Saving" : "Save settings"}
      </button>
      <button
        class="btn"
        disabled={testing || !email?.configured}
        onclick={testEmail}
      >
        {testing ? "Sending" : "Send test"}
      </button>
    </div>
  </section>

  <section class="card">
    <h2>Two-factor authentication</h2>
    <p class="muted">
      A six-digit code by email before the seed phrase or the Monero keys can be
      shown. The code lasts five minutes; three wrong tries lock it for a minute,
      and five abandon it and fall back to entering your seed phrase.
    </p>

    {#if email?.twoFactor}
      <p class="ok-note">
        On. Revealing your seed or keys asks for an emailed code first.
      </p>
    {:else}
      <p class="hint-note">
        Off. Reveals are protected only by the typed confirmation. Set up and
        save your email above before turning this on.
      </p>
    {/if}

    {#if twoFactorError}<p class="kerr">{twoFactorError}</p>{/if}

    <div class="control">
      <button
        class="btn btn-primary"
        disabled={twoFactorBusy || (!email?.twoFactor && !email?.configured)}
        onclick={toggleTwoFactor}
      >
        {twoFactorBusy
          ? "Working"
          : email?.twoFactor
            ? "Turn off two-factor"
            : "Turn on two-factor"}
      </button>
    </div>
  </section>

  <section class="card">
    <h2>Seed phrase</h2>
    <p class="muted">
      Your twelve words are the wallet. Anyone who reads them owns every coin
      here and can restore it anywhere, so reveal them only to write down a
      backup, somewhere private and offline.
    </p>

    {#if seedPhrase}
      <div class="keys">
        <p class="mono kval selectable danger-text">{seedPhrase}</p>
        <p class="knote">
          Write these down on paper and store them safely. Never type them into
          a website or share them with anyone, including anyone claiming to be
          support.
        </p>
        <button class="btn" onclick={hideSeed}>Hide</button>
      </div>
    {:else if askingSeed}
      <div class="confirm-box">
        <p class="muted">
          These will appear on screen. Make sure nobody is watching and that this
          machine is not being recorded. Type <strong>{REVEAL_WORD}</strong> to continue.
        </p>
        <input class="mono" bind:value={seedWord} spellcheck="false" />
        {#if seedError}
          <p class="kerr">{seedError}</p>
        {/if}
        <div class="crow">
          <button class="btn" onclick={hideSeed}>Cancel</button>
          <button
            class="btn btn-primary"
            disabled={seedWord.trim().toUpperCase() !== REVEAL_WORD}
            onclick={revealSeed}
          >
            Show seed phrase
          </button>
        </div>
      </div>
    {:else}
      <div class="control">
        <button class="btn" onclick={() => (askingSeed = true)}>Show seed phrase</button>
      </div>
    {/if}
  </section>

  <section class="card pending">
    <h2>Bucket rules <span class="tag">Not built</span></h2>
    <p class="muted">
      How incoming funds are split across buckets by default, and which bucket
      a shortfall is pulled from first.
    </p>
  </section>

  <section class="card">
    <h2>Clearing this machine</h2>
    <p class="muted">
      If this wallet is not opened for the period below, a switch fires the
      next time it starts. What it does is your choice.
    </p>

    <div class="periods">
      <button
        class="theme"
        class:active={inactivity?.action === "delete"}
        onclick={() => setAction("delete")}
      >
        Delete only
      </button>
      <button
        class="theme"
        class:active={inactivity?.action === "donate"}
        onclick={() => setAction("donate")}
      >
        Send to donations
      </button>
    </div>

    {#if inactivity?.action === "donate"}
      <p class="warn-note">
        Armed. After the period, and a 14-day grace in which opening the wallet
        cancels it, the balances are swept to the creator's fixed donation
        addresses and then the local wallet is deleted. This sends real money
        and cannot be undone. Tron, USDC, USDT and Monero cannot be swept and
        are left for your seed phrase to recover.
      </p>
    {:else}
      <p class="muted">
        Deletes the encrypted vault and its key. Nothing on any chain is
        touched, so the coins stay where they are and your seed phrase brings
        them back.
      </p>
    {/if}

    <p class="warn-note">
      Either way, your seed phrase is the only way back. Without it written
      down, this puts the funds out of reach for good.
    </p>

    <div class="periods">
      {#each PERIODS as period (period.months)}
        <button
          class="theme"
          class:active={inactivity?.months === period.months}
          onclick={() => setPeriod(period.months)}
        >
          {period.label}
        </button>
      {/each}
    </div>

    {#if inactivity}
      <p class="hint-note">
        {#if inactivity.months === 0}
          This machine will never clear itself.
        {:else if inactivity.daysRemaining != null}
          Last opened {inactivity.daysSince === 0
            ? "today"
            : `${inactivity.daysSince} days ago`}. Clears in
          {inactivity.daysRemaining} days if left untouched. Opening the wallet
          resets it.
        {/if}
      </p>
    {/if}

    <div class="divider"></div>

    <p class="muted">
      Or remove it now. This deletes the same two things, immediately.
    </p>

    {#if askingDelete}
      <div class="confirm-box">
        <p class="muted">
          This deletes the wallet stored on this machine and the key that
          decrypts it. If its seed phrase is not written down, the funds it
          holds are gone for good. Type <strong>{DELETE_WORD}</strong> to continue.
        </p>
        <input class="mono" bind:value={deleteWord} spellcheck="false" />
        {#if deleteError}
          <p class="kerr">{deleteError}</p>
        {/if}
        <div class="crow">
          <button
            class="btn"
            onclick={() => {
              askingDelete = false;
              deleteWord = "";
            }}
          >
            Cancel
          </button>
          <button
            class="btn danger-btn"
            disabled={deleteWord.trim().toUpperCase() !== DELETE_WORD}
            onclick={deleteWallet}
          >
            Delete this wallet
          </button>
        </div>
      </div>
    {:else}
      <div class="control">
        <button class="btn danger-btn" onclick={() => (askingDelete = true)}>
          Delete this wallet
        </button>
      </div>
    {/if}
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

{#if twoFactorFor}
  <TwoFactorPrompt
    purpose={twoFactorFor.purpose}
    onpassed={twoFactorPassed}
    oncancel={() => (twoFactorFor = null)}
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
  .pfield { display: block; margin-top: 12px; }
  .pfield span { display: block; margin-bottom: 5px; font-size: 12.5px; color: var(--text-muted); }
  .pfield input { width: 100%; }
  .pair { display: flex; gap: 10px; align-items: flex-end; }
  .pair .grow { flex: 1; }
  .stored { font-style: normal; color: var(--text-faint); font-size: 11px; }
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
  .warn-note { margin: 10px 0 0 !important; font-size: 12.5px; color: var(--warn); }
  .periods { display: flex; gap: 6px; margin-top: 14px; flex-wrap: wrap; }
  .periods .theme { flex: 1; min-width: 84px; }
  .danger-btn {
    color: var(--danger);
    border-color: color-mix(in srgb, var(--danger) 45%, var(--border));
  }
  .danger-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--danger) 10%, transparent);
  }
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
