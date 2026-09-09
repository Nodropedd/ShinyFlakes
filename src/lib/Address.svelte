<script lang="ts">
  // The derivation path used to appear in the tooltip. It means nothing to
  // anyone reading an address and only added noise, so only the address is
  // shown now.
  let { value }: { value: string; path?: string | null } = $props();

  let copied = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;

  // Without this, navigating away within the feedback window leaves a timer
  // holding a reference to a destroyed component.
  $effect(() => () => clearTimeout(timer));

  // Middle-truncated so both ends stay visible. The ends are what people
  // actually check when comparing an address against another screen.
  const short = $derived(
    value.length > 24 ? `${value.slice(0, 12)}\u2026${value.slice(-8)}` : value,
  );

  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
    } catch {
      // Clipboard API can be unavailable depending on webview settings.
      const el = document.createElement("textarea");
      el.value = value;
      el.setAttribute("readonly", "");
      el.style.position = "fixed";
      el.style.opacity = "0";
      document.body.appendChild(el);
      el.select();
      try {
        document.execCommand("copy");
      } finally {
        el.remove();
      }
    }
    copied = true;
    clearTimeout(timer);
    timer = setTimeout(() => (copied = false), 1400);
  }
</script>

<button
  class="addr mono"
  onclick={copy}
  title={value}
  aria-label="Copy address {value}"
>
  <span class="text">{short}</span>
  {#if copied}
    <span class="flag">Copied</span>
  {:else}
    <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor"
         stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
      <path d="M9 9h10v10H9zM5 15V5h10" />
    </svg>
  {/if}
</button>

<style>
  .addr {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 6px 2px 0;
    border-radius: 6px;
    color: var(--text-faint);
    font-size: 11.5px;
    transition: color 110ms var(--ease);
  }

  .addr:hover {
    color: var(--text-muted);
  }

  .flag {
    color: var(--ok);
    font-size: 10.5px;
    font-weight: 600;
  }
</style>
