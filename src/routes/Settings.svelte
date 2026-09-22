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
  import QrCode from "../lib/QrCode.svelte";
  import { BY_ID } from "../lib/assets";
  import {
    ipc,
    type MoneroKeys,
    type MoneroSetup,
    type MoneroStatus,
    type AssetId,
    type Donation,
    type Inactivity,
    type StepUpView,
    type TwoFactorState,
    type TotpSetup,
    type StaySignedIn,
  } from "../lib/ipc";
  import { session } from "../lib/session.svelte";
  import { updates } from "../lib/updates.svelte";
  import { untrack } from "svelte";
  import { DEFAULT_MONERO_ENDPOINT } from "../lib/settings.svelte";

  const REVEAL_WORD = "REVEAL";

  let twoFactorFor = $state<
    null | { purpose: string; run: () => Promise<void> }
  >(null);

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

    if (pending) await pending.run().catch(() => {});
  }

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

  $effect(() => () => {
    hideKeys();
    hideSeed();
    twoFactorFor = null;
  });

  let settingsBroken = $state(false);
  let resetting = $state(false);

  $effect(() => {
    void ipc
      .settingsUnreadable()
      .then((broken) => (settingsBroken = broken))
      .catch(() => (settingsBroken = false));
  });

  async function resetSettings() {
    if (resetting) return;
    resetting = true;
    try {
      await ipc.resetSettings();
      settingsBroken = false;
      await refreshStepUp();
    } finally {
      resetting = false;
    }
  }

  let twoFactorBusy = $state(false);
  let twoFactorError = $state<string | null>(null);

  let stepUp = $state<StepUpView | null>(null);
  let stepUpBusy = $state(false);
  let stepUpError = $state<string | null>(null);

  async function refreshStepUp() {
    try {
      stepUp = await ipc.stepUpSettings();
    } catch {
      stepUp = null;
    }
  }

  async function saveStepUp(next: Partial<StepUpView>) {
    if (!stepUp || stepUpBusy) return;
    stepUpBusy = true;
    stepUpError = null;
    const merged = { ...stepUp, ...next };
    try {
      stepUp = await ipc.setStepUpSettings(merged.dormantDays, merged.largeSend);
    } catch (e) {
      stepUpError = (e as { message?: string }).message ?? String(e);
    } finally {
      stepUpBusy = false;
    }
  }

  $effect(() => {
    void refreshStepUp();
  });

  let twoFactor = $state<TwoFactorState | null>(null);
  let totpSetup = $state<TotpSetup | null>(null);
  let totpCode = $state("");

  $effect(() => {
    ipc
      .twoFactorState()
      .then((s) => (twoFactor = s))
      .catch(() => {

      });
  });

  async function startTotp() {
    if (twoFactorBusy) return;
    twoFactorBusy = true;
    twoFactorError = null;
    try {
      totpSetup = await ipc.beginTotpSetup();
      totpCode = "";
    } catch (e) {
      twoFactorError = (e as { message?: string }).message ?? String(e);
    } finally {
      twoFactorBusy = false;
    }
  }

  async function confirmTotp() {
    if (twoFactorBusy || totpCode.trim().length === 0) return;
    twoFactorBusy = true;
    twoFactorError = null;
    try {
      const ok = await ipc.confirmTotp(totpCode.trim());
      if (ok) {
        totpSetup = null;
        totpCode = "";
        twoFactor = await ipc.twoFactorState();
      } else {
        twoFactorError =
          "That code didn't match. Make sure your phone's clock is right and enter the current code.";
      }
    } catch (e) {
      twoFactorError = (e as { message?: string }).message ?? String(e);
    } finally {
      twoFactorBusy = false;
    }
  }

  function cancelTotp() {
    totpSetup = null;
    totpCode = "";
    twoFactorError = null;
  }

  async function disableTotp() {
    if (twoFactorBusy) return;
    twoFactorBusy = true;
    twoFactorError = null;
    try {
      await ipc.disableTwoFactor();
      twoFactor = await ipc.twoFactorState();
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

  const accentValue = $derived(settings.accent ?? DEFAULT_ACCENT);

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

  $effect(() => {
    const allowed = wallet.connected;
    untrack(() => void updates.checkIfAllowed(allowed));
  });

  let staySignedIn = $state<StaySignedIn | null>(null);
  let stayBusy = $state(false);
  let stayError = $state<string | null>(null);

  $effect(() => {
    void hasPassphrase;
    ipc
      .staySignedInState()
      .then((s) => (staySignedIn = s))
      .catch(() => {

      });
  });

  async function doSetStaySignedIn(on: boolean) {
    stayError = null;
    stayBusy = true;
    try {
      staySignedIn = await ipc.setStaySignedIn(on);
    } catch (e) {
      if ((e as { kind?: string }).kind === "TwoFactorRequired") throw e;
      stayError = (e as { message?: string }).message ?? String(e);
    } finally {
      stayBusy = false;
    }
  }

  async function setStaySignedIn(on: boolean) {
    if (stayBusy) return;
    if (on) {
      await guarded("keep this device signed in", () => doSetStaySignedIn(true));
    } else {
      await doSetStaySignedIn(false);
    }
  }

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

  let donations = $state<Donation[]>([]);
  let tipping = $state<Donation | null>(null);

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

      });
  });

  let torInstalled = $state(false);

  $effect(() => {
    ipc
      .torState()
      .then((s) => (torInstalled = s.installed))
      .catch(() => {

      });
  });

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
    <h2>Updates</h2>
    {#if updates.info?.available}
      <p class="ok-note">
        Update available: version {updates.info.latest}. You have {updates.info.current}.
      </p>
      <div class="control">
        <button class="btn btn-primary" onclick={() => updates.openDownloads()}>Update</button>
      </div>
      <p class="hint-note">
        Opens the download page. Install it the way you installed this one; your
        wallet and settings stay as they are.
      </p>
    {:else if updates.info}
      <p class="muted">You have the latest version, {updates.info.current}.</p>
    {:else}
      <p class="muted">
        Asks GitHub whether a newer version is out, through Tor when that is on.
        {wallet.connected ? "" : "Once you connect, this happens on its own."}
      </p>
      <div class="control">
        <button class="btn" disabled={updates.checking} onclick={() => updates.check()}>
          {updates.checking ? "Checking" : "Check for updates"}
        </button>
      </div>
    {/if}
    {#if updates.error}<p class="kerr">{updates.error}</p>{/if}
  </section>

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
      Without one, the vault decrypts using a key in this machine's credential
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
    <h2>Stay signed in</h2>
    <p class="muted">
      Opening ShinyFlakes goes straight to the wallet instead of asking for the
      seed phrase. Closing the app, or the phone clearing it from memory, no
      longer signs you out.
    </p>
    <p class="warn-note">
      Anyone who can open the app on this device can then see your balances and
      send your funds, so the device's own screen lock becomes the thing
      protecting them. Lock still locks: after pressing it, the next start asks
      for the phrase again. With two-factor on, a wallet left unopened past its
      limit still asks for a code.
    </p>

    {#if staySignedIn && !staySignedIn.available}
      <p class="hint-note">
        Not available with a vault passphrase, which has to be typed on every
        open. Remove the passphrase above to use it.
      </p>
    {:else if staySignedIn?.enabled}
      <p class="ok-note">
        This device stays signed in.
        <button class="inline" disabled={stayBusy} onclick={() => setStaySignedIn(false)}>
          Turn off
        </button>
      </p>
    {:else}
      <div class="control">
        <button
          class="btn"
          disabled={stayBusy || !staySignedIn}
          onclick={() => setStaySignedIn(true)}
        >
          Stay signed in on this device
        </button>
      </div>
    {/if}

    {#if stayError}<p class="kerr">{stayError}</p>{/if}
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
      <p class="hint-note">
        Connecting to Tor.{torInstalled ? "" : " The first time also downloads it."}
      </p>
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
      {#if !torInstalled}
        <p class="hint-note">
          First time only: about 20 MB from torproject.org, checked against a hash
          built into this program. Connecting then takes a moment.
        </p>
      {/if}
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

  {#if settingsBroken}
    <section class="card danger-card">
      <h2>Saved settings unreadable</h2>
      <p class="muted">
        Your wallet and your coins are not affected. This file holds only
        your preferences and the two-factor secret — not your seed.
        It cannot be opened with the key on this machine, so the panels below
        will not work until it is reset.
      </p>
      <p class="hint-note">
        Resetting keeps the old file alongside, as
        <code class="mono">config.dat.unreadable</code>. Two-factor goes back
        to off and the authenticator has to be paired again.
      </p>
      <div class="crow">
        <button class="btn btn-primary" disabled={resetting} onclick={resetSettings}>
          {resetting ? "Resetting" : "Reset settings"}
        </button>
      </div>
    </section>
  {/if}

  <section class="card">
    <h2>When to ask again</h2>

    {#if stepUp}
      {#if !stepUp.totpAvailable}
        <p class="warn-note">
          Two-factor is off, so nothing is being asked for. Turn it on below.
        </p>
      {/if}

      <p class="muted">
        Always asked before showing the seed phrase or a private key.
      </p>

      <label class="toggle">
        <input
          type="checkbox"
          checked={stepUp.largeSend}
          disabled={stepUpBusy}
          onchange={(e) =>
            saveStepUp({ largeSend: e.currentTarget.checked })}
        />
        <span>
          Ask before sending more than ${stepUp.largeSendUsd.toFixed(0)} that is
          also at least {Math.round(stepUp.largeSendShare * 100)}% of everything
          here. Both have to be true, so routine payments stay quiet.
        </span>
      </label>

      <label class="pfield">
        <span>
          Ask on the first unlock after this many days unopened
          {#if stepUp.dormantDays === 0}<em class="stored">off</em>{/if}
        </span>
        <input
          class="mono"
          type="number"
          min="0"
          max="365"
          value={stepUp.dormantDays}
          disabled={stepUpBusy}
          onchange={(e) =>
            saveStepUp({ dormantDays: Number(e.currentTarget.value) || 0 })}
        />
      </label>

      {#if stepUpError}<p class="kerr">{stepUpError}</p>{/if}
    {/if}
  </section>

  <section class="card">
    <h2>Two-factor authentication</h2>
    <p class="muted">
      A six-digit code from an authenticator app (Google Authenticator, Aegis,
      and the like) before the seed phrase or the Monero keys can be shown. It
      is fully offline: no email, no server. The code lives on your phone, so
      someone at this machine cannot produce it. If you lose the phone, your
      seed phrase still works as a fallback.
    </p>

    {#if totpSetup}
      <p class="hint-note">
        Scan this in your authenticator app, then enter the code it shows to
        finish.
      </p>
      <div class="qrwrap">
        <QrCode value={totpSetup.uri} size={168} />
      </div>
      <p class="klabel">Or enter this secret by hand</p>
      <p class="mono kval selectable">{totpSetup.secret}</p>

      <label class="pfield">
        <span>Code from the app</span>
        <input
          class="mono"
          bind:value={totpCode}
          inputmode="numeric"
          maxlength="6"
          placeholder="000000"
          spellcheck="false"
        />
      </label>

      {#if twoFactorError}<p class="kerr">{twoFactorError}</p>{/if}

      <div class="crow">
        <button class="btn" onclick={cancelTotp} disabled={twoFactorBusy}>Cancel</button>
        <button
          class="btn btn-primary"
          onclick={confirmTotp}
          disabled={twoFactorBusy || totpCode.trim().length === 0}
        >
          {twoFactorBusy ? "Checking" : "Turn on two-factor"}
        </button>
      </div>
    {:else if twoFactor?.enabled}
      <p class="ok-note">On. Revealing your seed or keys asks for a code first.</p>
      {#if twoFactorError}<p class="kerr">{twoFactorError}</p>{/if}
      <div class="control">
        <button class="btn danger-btn" disabled={twoFactorBusy} onclick={disableTotp}>
          {twoFactorBusy ? "Working" : "Turn off two-factor"}
        </button>
      </div>
    {:else}
      <p class="hint-note">
        Off. Reveals are protected only by the typed confirmation.
      </p>
      {#if twoFactorError}<p class="kerr">{twoFactorError}</p>{/if}
      <div class="control">
        <button class="btn btn-primary" disabled={twoFactorBusy} onclick={startTotp}>
          {twoFactorBusy ? "Working" : "Set up two-factor"}
        </button>
      </div>
    {/if}
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
  .danger-card { border-color: var(--danger); }
  .toggle { display: flex; gap: 10px; align-items: flex-start; margin-top: 13px; }
  .toggle input { margin-top: 2px; flex: none; }
  .toggle span { font-size: 12.5px; line-height: 1.5; color: var(--text-muted); }
  .pfield { display: block; margin-top: 12px; }
  .pfield span { display: block; margin-bottom: 5px; font-size: 12.5px; color: var(--text-muted); }
  .pfield input { width: 100%; }
  .stored { font-style: normal; color: var(--text-faint); font-size: 11px; }
  .qrwrap {
    display: flex; justify-content: center; margin: 14px 0;
    padding: 14px; background: #fff; border-radius: var(--radius-sm);
    width: fit-content;
  }
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
