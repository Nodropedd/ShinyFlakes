<script lang="ts">
  import jsQR from "jsqr";

  let { onresult, onclose }: { onresult: (text: string) => void; onclose: () => void } =
    $props();

  let video: HTMLVideoElement | null = $state(null);
  let error = $state<string | null>(null);
  let scanning = $state(false);
  let fileInput: HTMLInputElement | null = $state(null);

  let stream: MediaStream | null = null;
  let raf: number | undefined;
  const canvas = document.createElement("canvas");

  // Wallet QR codes are often a payment URI like "bitcoin:bc1...?amount=1".
  // The send box wants the bare address, so the scheme and any query are
  // stripped off.
  function extractAddress(raw: string): string {
    let text = raw.trim();
    const colon = text.indexOf(":");
    // Only treat it as a scheme when what precedes the colon looks like one,
    // never for an address that happens to contain a colon.
    if (colon > 0 && colon < 12 && /^[a-zA-Z]+$/.test(text.slice(0, colon))) {
      text = text.slice(colon + 1);
    }
    const q = text.indexOf("?");
    if (q >= 0) text = text.slice(0, q);
    return text.trim();
  }

  function finish(raw: string) {
    stop();
    onresult(extractAddress(raw));
  }

  function decodeFrame() {
    if (!video || video.readyState !== video.HAVE_ENOUGH_DATA) {
      raf = requestAnimationFrame(decodeFrame);
      return;
    }
    canvas.width = video.videoWidth;
    canvas.height = video.videoHeight;
    const ctx = canvas.getContext("2d", { willReadFrequently: true });
    if (!ctx) return;
    ctx.drawImage(video, 0, 0, canvas.width, canvas.height);
    const image = ctx.getImageData(0, 0, canvas.width, canvas.height);
    const code = jsQR(image.data, image.width, image.height);
    if (code && code.data) {
      finish(code.data);
      return;
    }
    raf = requestAnimationFrame(decodeFrame);
  }

  async function startCamera() {
    error = null;
    try {
      stream = await navigator.mediaDevices.getUserMedia({
        video: { facingMode: "environment" },
      });
      scanning = true;
      // The element only exists once scanning is true, so wait a tick.
      await Promise.resolve();
      if (video) {
        video.srcObject = stream;
        await video.play();
        raf = requestAnimationFrame(decodeFrame);
      }
    } catch (e) {
      // No camera, denied, or unsupported in this webview. The image path
      // below still works, so this is a note, not a dead end.
      error =
        "Camera unavailable. Allow camera access, or scan a saved image instead.";
      scanning = false;
    }
  }

  function scanFile(event: Event) {
    error = null;
    const file = (event.target as HTMLInputElement).files?.[0];
    if (!file) return;

    const img = new Image();
    img.onload = () => {
      canvas.width = img.naturalWidth;
      canvas.height = img.naturalHeight;
      const ctx = canvas.getContext("2d", { willReadFrequently: true });
      if (!ctx) return;
      ctx.drawImage(img, 0, 0);
      const data = ctx.getImageData(0, 0, canvas.width, canvas.height);
      const code = jsQR(data.data, data.width, data.height);
      URL.revokeObjectURL(img.src);
      if (code && code.data) {
        finish(code.data);
      } else {
        error = "No QR code found in that image.";
      }
    };
    img.onerror = () => {
      error = "That file could not be read as an image.";
    };
    img.src = URL.createObjectURL(file);
  }

  function stop() {
    if (raf) cancelAnimationFrame(raf);
    raf = undefined;
    if (stream) {
      stream.getTracks().forEach((t) => t.stop());
      stream = null;
    }
    scanning = false;
  }

  // Tearing the camera down when the component goes away matters: a live
  // camera light left on would be alarming in a wallet.
  $effect(() => () => stop());
</script>

<div
  class="scrim"
  role="button"
  tabindex="-1"
  onclick={() => {
    stop();
    onclose();
  }}
  onkeydown={(e) => {
    if (e.key === "Escape") {
      stop();
      onclose();
    }
  }}
>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="panel card" onclick={(e) => e.stopPropagation()}>
    <header>
      <h2>Scan a QR code</h2>
      <button
        class="x"
        onclick={() => {
          stop();
          onclose();
        }}
        aria-label="Close">&times;</button
      >
    </header>

    {#if scanning}
      <div class="frame">
        <!-- svelte-ignore a11y_media_has_caption -->
        <video bind:this={video} playsinline></video>
      </div>
      <p class="hint">Point the camera at the code.</p>
    {:else}
      <div class="choices">
        <button class="btn btn-primary" onclick={startCamera}>Use camera</button>
        <button class="btn" onclick={() => fileInput?.click()}>Scan an image</button>
      </div>
    {/if}

    <input
      class="hidden-file"
      type="file"
      accept="image/*"
      bind:this={fileInput}
      onchange={scanFile}
    />

    {#if error}
      <p class="err">{error}</p>
    {/if}
  </div>
</div>

<style>
  .scrim {
    position: fixed; inset: 0; display: grid; place-items: center;
    padding: 24px; background: rgba(0, 0, 0, 0.6); z-index: 60; border: 0;
  }
  .panel {
    width: 100%; max-width: 380px; padding: 20px 22px 22px;
    box-shadow: var(--shadow); text-align: left; cursor: default;
  }
  header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 14px; }
  h2 { margin: 0; font-size: 15.5px; font-weight: 650; }
  .x { color: var(--text-muted); font-size: 20px; line-height: 1; padding: 0 4px; }
  .x:hover { color: var(--text); }

  .frame {
    aspect-ratio: 1; width: 100%; overflow: hidden;
    border-radius: var(--radius-sm); background: #000;
  }
  .frame video { width: 100%; height: 100%; object-fit: cover; }
  .hint { margin: 10px 0 0; font-size: 12.5px; color: var(--text-muted); text-align: center; }

  .choices { display: flex; gap: 8px; }
  .choices .btn { flex: 1; }

  .hidden-file { display: none; }
  .err { margin: 12px 0 0; font-size: 12.5px; color: var(--danger); }
</style>
