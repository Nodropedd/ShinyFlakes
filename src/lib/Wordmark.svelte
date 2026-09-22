<script lang="ts">
  let { size = 40 }: { size?: number } = $props();
</script>

<span class="wordmark" style="font-size: {size}px">ShinyFlakes</span>

<style>

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
