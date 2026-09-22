<script lang="ts">
  import { untrack } from "svelte";

  let {
    pieces,
    status,
    direction = "split",
  }: {
    pieces: number;

    status: "working" | "done" | "failed";
    direction?: "split" | "form";
  } = $props();

  const shown = $derived(Math.max(2, Math.min(pieces, 8)));
  const shards = $derived(Array.from({ length: shown }, (_, i) => i));

  type Stage = "raise" | "strike" | "scatter" | "bagged";

  let stage = $state<Stage>(untrack(() => (direction === "form" ? "bagged" : "raise")));

  $effect(() => {
    const timers: ReturnType<typeof setTimeout>[] = [];
    if (direction === "split") {
      timers.push(setTimeout(() => (stage = "strike"), 260));
      timers.push(setTimeout(() => (stage = "scatter"), 620));
      timers.push(setTimeout(() => (stage = "bagged"), 1400));
    } else {

      timers.push(setTimeout(() => (stage = "scatter"), 500));
      timers.push(setTimeout(() => (stage = "raise"), 1300));
    }
    return () => timers.forEach(clearTimeout);
  });

  const bagWidth = $derived(250 / shown);

  function bagCentre(i: number) {
    return 35 + bagWidth * i + bagWidth / 2;
  }

  const inBags = $derived(stage === "bagged");
  const whole = $derived(direction === "form" && stage === "raise");
</script>

<div class="scene" class:failed={status === "failed"}>
  <svg viewBox="0 0 320 200" role="img"
       aria-label={direction === "split"
         ? `Splitting the balance into ${pieces} pieces`
         : `Reassembling ${pieces} pieces into one`}>

    {#if direction === "split"}
      <g class="hammer {stage}">
        <rect x="150" y="-6" width="9" height="52" rx="4" fill="var(--text-faint)" />
        <rect x="132" y="-16" width="46" height="20" rx="4" fill="var(--text-muted)" />
      </g>
    {/if}

    {#if stage === "raise" || stage === "strike"}
      <g class="block" class:hit={stage === "strike"} class:formed={whole}>
        <rect x="112" y="58" width="96" height="44" rx="6"
              fill="color-mix(in srgb, var(--accent) 30%, transparent)"
              stroke="var(--accent)" stroke-width="1.5" />
        <line x1="124" y1="68" x2="142" y2="92" stroke="var(--accent)"
              stroke-width="1" opacity="0.5" />
      </g>
    {/if}

    <g class="bags" class:ready={inBags}>
      {#each shards as i (i)}
        {@const x = 35 + bagWidth * i + bagWidth * 0.12}
        {@const w = bagWidth * 0.76}
        <g style="--d: {i * 45}ms">
          <path d="M{x} 146 h{w} v26 a5 5 0 0 1 -5 5 h{-(w - 10)} a5 5 0 0 1 -5 -5 Z"
                fill="var(--bg-raised)" stroke="var(--border-strong)" stroke-width="1.2" />
        </g>
      {/each}
    </g>

    {#if stage !== "raise" || direction === "form"}
      {#each shards as i (i)}
        <polygon
          class="shard"
          class:scattered={stage === "scatter"}
          class:bagged={inBags}
          class:gone={whole}
          points="0,-8 7,0 2,8 -6,3"
          fill="color-mix(in srgb, var(--accent) 60%, transparent)"
          stroke="var(--accent)" stroke-width="1"
          style="--cx: {bagCentre(i)}px; --d: {i * 45}ms"
        />
      {/each}
    {/if}

    <g class="bags front" class:ready={inBags}>
      {#each shards as i (i)}
        {@const x = 35 + bagWidth * i + bagWidth * 0.12}
        {@const w = bagWidth * 0.76}
        <g style="--d: {i * 45}ms">
          <path d="M{x} 160 h{w} v12 a5 5 0 0 1 -5 5 h{-(w - 10)} a5 5 0 0 1 -5 -5 Z"
                fill="color-mix(in srgb, var(--bg-raised) 78%, transparent)"
                stroke="var(--border-strong)" stroke-width="1" />
          <rect x={x + w * 0.28} y="141" width={w * 0.44} height="6" rx="3"
                fill="var(--border-strong)" />
        </g>
      {/each}
    </g>
  </svg>

  <p class="caption">
    {#if status === "failed"}
      That did not go through.
    {:else if status === "done"}
      {direction === "split" ? `Split into ${pieces} pieces.` : "Combined into one."}
    {:else}
      {direction === "split" ? "Splitting." : "Combining."}
    {/if}
  </p>
</div>

<style>
  .scene { display: flex; flex-direction: column; align-items: center; gap: 6px; }
  svg { width: 100%; max-width: 320px; height: auto; overflow: visible; }

  .hammer {
    transform-origin: 155px 46px;
    transform: rotate(-58deg);
    transition: transform 260ms cubic-bezier(0.4, 0, 1, 1);
  }
  .hammer.strike { transform: rotate(4deg); }
  .hammer.scatter,
  .hammer.bagged { transform: rotate(-72deg); transition: transform 420ms var(--ease); }

  .block { transition: transform 120ms var(--ease), opacity 300ms var(--ease); }
  .block.hit { transform: translateY(3px) scaleY(0.94); transform-origin: 160px 102px; }

  .block.formed { animation: reform 420ms var(--ease); }

  @keyframes reform {
    from { opacity: 0; transform: scale(0.8); }
    to { opacity: 1; transform: scale(1); }
  }

  .shard {
    transform: translate(160px, 80px);
    transition:
      transform 700ms cubic-bezier(0.35, 0.9, 0.4, 1),
      opacity 260ms var(--ease);
    transition-delay: var(--d);
  }

  .shard.scattered { transform: translate(var(--cx), 96px) rotate(150deg); }

  .shard.bagged { transform: translate(var(--cx), 163px) rotate(200deg); }

  .shard.gone { transform: translate(160px, 80px) scale(0.4); opacity: 0; }

  .bags g { opacity: 0.4; transition: opacity 300ms var(--ease); transition-delay: var(--d); }
  .bags.ready g { opacity: 1; }

  .caption { margin: 0; font-size: 12.5px; color: var(--text-muted); }
  .failed .caption { color: var(--danger); }

  @media (prefers-reduced-motion: reduce) {
    .hammer, .block, .shard, .bags g {
      animation: none !important;
      transition: none !important;
    }
  }
</style>
