<script lang="ts">
  import type { AssetMeta } from "./assets";

  let { asset, size = 34 }: { asset: AssetMeta; size?: number } = $props();
</script>

<img
  class="logo"
  src={asset.logo}
  alt=""
  width={size}
  height={size}
  style="width: {size}px; height: {size}px"
  aria-hidden="true"
/>

<style>
  .logo {
    flex: none;
    border-radius: 50%;
    display: block;
  }
</style>
