<script lang="ts">
  import QRCode from "qrcode";

  let { value, size = 180 }: { value: string; size?: number } = $props();

  let dataUrl = $state<string | null>(null);
  let error = $state(false);

  $effect(() => {
    error = false;
    dataUrl = null;
    QRCode.toDataURL(value, {
      errorCorrectionLevel: "M",
      margin: 1,
      width: size * 2,
      color: { dark: "#000000ff", light: "#ffffffff" },
    })
      .then((url) => (dataUrl = url))
      .catch(() => (error = true));
  });
</script>

<div class="qr" style="width: {size}px; height: {size}px">
  {#if dataUrl}
    <img src={dataUrl} alt="QR code for {value}" width={size} height={size} />
  {:else if error}
    <span class="err">Could not draw the code.</span>
  {/if}
</div>

<style>
  .qr {
    display: grid;
    place-items: center;
    padding: 10px;
    border-radius: var(--radius-sm);
    background: #fff;
    box-sizing: content-box;
  }
  .qr img {
    display: block;
    image-rendering: pixelated;
  }
  .err {
    font-size: 11.5px;
    color: var(--danger);
    text-align: center;
  }
</style>
