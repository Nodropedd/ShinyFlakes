// Session state for the UI shell. Deliberately holds no key material: the
// Rust side is the only thing that ever sees a derived key, and this store
// only tracks which screen we belong on.

import { ipc, type VaultStatus } from "./ipc";

export type Screen = "loading" | "cold-start" | "locked" | "wallet";

class Session {
  status = $state<VaultStatus>({ initialized: false, unlocked: false });
  screen = $state<Screen>("loading");
  error = $state<string | null>(null);

  async refresh() {
    try {
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
