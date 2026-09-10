// Session state for the UI shell. Deliberately holds no key material: the
// Rust side is the only thing that ever sees a derived key, and this store
// only tracks which screen we belong on.

import { ipc, type Inactivity, type VaultStatus } from "./ipc";

export type Screen = "loading" | "cold-start" | "locked" | "wallet";

class Session {
  status = $state<VaultStatus>({
    initialized: false,
    unlocked: false,
    needsPassphrase: false,
  });

  /** Set when the inactivity switch cleared the wallet, so the cold start can
   *  explain itself rather than looking like data loss. */
  clearedByInactivity = $state(false);
  /** Set when the switch swept balances to the donation addresses. */
  sweptByInactivity = $state(false);
  /** True while a due sweep is being broadcast. */
  sweeping = $state(false);
  screen = $state<Screen>("loading");
  error = $state<string | null>(null);

  async refresh() {
    try {
      // Before anything else, since it may clear the vault this is about to
      // ask after. It needs no keys, so it works while locked out.
      const inactivity: Inactivity = await ipc.inactivityCheck();
      if (inactivity.wiped) this.clearedByInactivity = true;

      // Donate mode past its grace: send the balances, then the core deletes
      // the local wallet. Runs before unlock, deliberately, because the whole
      // point is that nobody is here to unlock. A returning owner never
      // reaches this: unlocking on any earlier launch resets the clock.
      if (inactivity.sweepDue) {
        this.sweeping = true;
        try {
          await ipc.inactivitySweep();
          this.sweptByInactivity = true;
        } catch {
          // A failed sweep leaves the wallet in place to retry next launch.
        } finally {
          this.sweeping = false;
        }
      }

      this.status = await ipc.vaultStatus();
      this.screen = !this.status.initialized
        ? "cold-start"
        : this.status.unlocked
          ? "wallet"
          : "locked";
    } catch (e) {
      this.error = (e as { message?: string }).message ?? String(e);
      this.screen = "cold-start";
    }
  }

  async logout() {
    await ipc.logout();
    await this.refresh();
  }
}

export const session = new Session();
