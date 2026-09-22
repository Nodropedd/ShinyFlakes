// Update-check state.

import { ipc, type UpdateInfo } from "./ipc";

class Updates {
  info = $state<UpdateInfo | null>(null);
  checking = $state(false);
  error = $state<string | null>(null);

  async check() {
    if (this.checking) return;
    this.checking = true;
    this.error = null;
    try {
      this.info = await ipc.checkForUpdate();
    } catch (e) {
      this.error = (e as { message?: string }).message ?? String(e);
    } finally {
      this.checking = false;
    }
  }

  async checkIfAllowed(networkAllowed: boolean) {
    if (!networkAllowed || this.info || this.checking || this.error) return;
    await this.check();
  }

  async openDownloads() {
    try {
      await ipc.openDownloadPage();
    } catch (e) {
      this.error = (e as { message?: string }).message ?? String(e);
    }
  }
}

export const updates = new Updates();
