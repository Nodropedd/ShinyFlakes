// Clipboard helper.

export type CopyOutcome = "ok" | "mismatch" | "unknown";

const SETTLE_MS = 300;

export async function copyAndVerify(value: string): Promise<CopyOutcome> {
  let wrote = false;
  try {
    await navigator.clipboard.writeText(value);
    wrote = true;
  } catch {

    wrote = legacyCopy(value);
  }

  if (!wrote) return "unknown";

  await new Promise((r) => setTimeout(r, SETTLE_MS));

  try {
    const now = await navigator.clipboard.readText();
    return now === value ? "ok" : "mismatch";
  } catch {

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
