<script lang="ts">
  import Wordmark from "./Wordmark.svelte";

  export type View = "portfolio" | "swap" | "utxo" | "activity" | "settings";

  let {
    current,
    onselect,
    onlock,
  }: {
    current: View;
    onselect: (v: View) => void;
    onlock: () => void;
  } = $props();

  const ITEMS: { id: View; label: string; path: string }[] = [

    { id: "portfolio", label: "Portfolio", path: "M3 7h18v11a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1zM3 7l2-3h14l2 3M16 13h2" },

    { id: "swap", label: "Swap", path: "M7 4v13M7 4L4 7M7 4l3 3M17 20V7M17 20l-3-3M17 20l3-3" },

    { id: "utxo", label: "UTXO", path: "M4 5h6v6H4zM14 5h6v3h-6zM4 15h4v4H4zM12 12h8v7h-8z" },

    { id: "activity", label: "Activity", path: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 7v5l3 2" },

    { id: "settings", label: "Settings", path: "M4 7h10M18 7h2M4 17h4M12 17h8M16 5v4M8 15v4" },
  ];
</script>

<nav class="sidebar">
  <div class="brand"><Wordmark size={21} /></div>

  <ul>
    {#each ITEMS as item (item.id)}
      <li>
        <button
          class="item"
          class:active={current === item.id}
          aria-current={current === item.id ? "page" : undefined}
          onclick={() => onselect(item.id)}
        >
          <svg viewBox="0 0 24 24" width="17" height="17" fill="none"
               stroke="currentColor" stroke-width="1.6"
               stroke-linecap="round" stroke-linejoin="round">
            <path d={item.path} />
          </svg>
          <span>{item.label}</span>
        </button>
      </li>
    {/each}
  </ul>

  <button class="item lock" onclick={onlock}>
    <svg viewBox="0 0 24 24" width="17" height="17" fill="none"
         stroke="currentColor" stroke-width="1.6"
         stroke-linecap="round" stroke-linejoin="round">
      <path d="M6 11h12v9H6zM9 11V8a3 3 0 0 1 6 0v3" />
    </svg>
    <span>Lock</span>
  </button>
</nav>

<style>
  .sidebar {
    width: 208px;
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 18px 12px 14px;
    background: var(--bg-raised);
    border-right: 1px solid var(--border);
  }

  .brand {
    padding: 0 8px 18px;
  }

  ul {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .item {
    display: flex;
    align-items: center;
    gap: 11px;
    width: 100%;
    padding: 9px 10px;
    border-radius: var(--radius-sm);
    color: var(--text-muted);
    font-size: 13.5px;
    font-weight: 500;
    text-align: left;
    transition: background 120ms var(--ease), color 120ms var(--ease);
  }

  .item:hover {
    background: var(--card);
    color: var(--text);
  }

  .item.active {
    background: var(--card);
    color: var(--text);
  }

  .item.active svg {
    color: var(--accent);
  }

  .lock {
    margin-top: auto;
  }

  @media (max-width: 720px) {
    .sidebar {
      width: 100%;
      flex-direction: row;
      gap: 0;
      padding: 0;
      border-right: none;
      border-top: 1px solid var(--border);

      padding-bottom: max(10px, env(safe-area-inset-bottom, 0px));
    }

    .brand {
      display: none;
    }

    ul {
      flex: 1;
      flex-direction: row;
      gap: 0;
    }

    li {
      flex: 1;
      min-width: 0;
    }

    .item {
      flex-direction: column;
      gap: 3px;
      padding: 9px 2px;
      border-radius: 0;
      text-align: center;
      font-size: 10.5px;
    }

    .item.active {
      background: none;
      box-shadow: inset 0 2px 0 var(--accent);
    }

    .item:hover {
      background: none;
    }

    .lock {
      flex: none;
      margin-top: 0;
      width: 62px;
      border-left: 1px solid var(--border);
    }
  }
</style>
