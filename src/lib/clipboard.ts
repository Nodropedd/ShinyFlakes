// Copy to the clipboard, then check nothing swapped it out.
//
// A known family of malware watches the clipboard and, the instant a crypto
// address is copied, replaces it with the attacker's. The victim pastes that
// into an exchange or another wallet and pays the thief. This copies, waits a
// moment for such a swap to happen, reads the clipboard back, and reports
// whether it still holds what was written.

export type CopyOutcome = "ok" | "mismatch" | "unknown";

/** How long to wait before reading back, to give a hijacker time to strike. */
const SETTLE_MS = 300;

export async function copyAndVerify(value: string): Promise<CopyOutcome> {
  let wrote = false;
  try {
    await navigator.clipboard.writeText(value);
    wrote = true;
  } catch {
    // The async API can be unavailable depending on webview settings; fall
    // back to the old execCommand path.
    wrote = legacyCopy(value);
  }

  if (!wrote) return "unknown";

  await new Promise((r) => setTimeout(r, SETTLE_MS));

  // Read the clipboard back. If it differs from what we just wrote, something
  // on this machine changed it, which for an address is a red flag.
  try {
    const now = await navigator.clipboard.readText();
    return now === value ? "ok" : "mismatch";
  } catch {
    // Reading back is not always permitted; then the copy stands but cannot
    // be confirmed.
    return "unknown";
  }
}

function legacyCopy(value: string): boolean {
  try {
    const el = document.createElement("textarea");
    el.value = value;
    el.setAttribute("readonly", "");
    el.style.position = "fixed";
    el.style.opacity = "0";
    document.body.appendChild(el);
    el.select();
    const ok = document.execCommand("copy");
    el.remove();
    return ok;
  } catch {
    return false;
  }
}
