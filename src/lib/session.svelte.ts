// Which screen we belong on.

import { ipc, type Inactivity, type VaultStatus } from "./ipc";

export type Screen = "loading" | "cold-start" | "locked" | "wallet";

class Session {
  status = $state<VaultStatus>({
    initialized: false,
    unlocked: false,
    needsPassphrase: false,
    keyMissing: false,
  });

  clearedByInactivity = $state(false);

  sweptByInactivity = $state(false);

  sweeping = $state(false);
  screen = $state<Screen>("loading");
  error = $state<string | null>(null);

  async refresh() {
    this.error = null;
    try {

      const inactivity: Inactivity = await ipc.inactivityCheck();
      if (inactivity.wiped) this.clearedByInactivity = true;

      if (inactivity.sweepDue) {
        this.sweeping = true;
        try {
          await ipc.inactivitySweep();
          this.sweptByInactivity = true;
        } catch {

        } finally {
          this.sweeping = false;
        }
      }

      this.status = await ipc.vaultStatus();

      if (
        this.status.initialized &&
        !this.status.unlocked &&
        !this.status.keyMissing &&
        !this.status.needsPassphrase &&
        (await ipc.autoUnlock())
      ) {
        this.status = await ipc.vaultStatus();
      }

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
