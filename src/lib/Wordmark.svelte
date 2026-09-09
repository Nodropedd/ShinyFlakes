<script lang="ts">
  let { size = 40 }: { size?: number } = $props();
</script>

<span class="wordmark" style="font-size: {size}px">ShinyFlakes</span>

<style>
  /* Flat gold script logotype. One colour, no gradient: a gradient across
     letterforms this thin muddies the strokes into brown at the ends.
     The face is bundled, not fetched: see the @font-face note in app.css. */
  .wordmark {
    font-family: var(--font-brand);
    font-weight: 400;
    color: var(--gold);
    letter-spacing: 0.005em;
    line-height: 1.45;
    padding-right: 0.12em;
    white-space: nowrap;
  }
</style>
