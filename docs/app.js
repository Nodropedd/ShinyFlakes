// Two small jobs, both local: point the main button at the visitor's own
// system, and fill in each file's size, and the fingerprints in the verify
// fold, from release.json beside this page. Nothing is fetched from anywhere
// else.

(() => {
  const platformOf = () => {
    const ua = navigator.userAgent;
    if (/Android/i.test(ua)) return "android";
    if (/iPhone|iPad|iPod/i.test(ua)) return "apple";
    if (/Windows/i.test(ua)) return "windows";
    if (/Macintosh|Mac OS X/i.test(ua)) return "apple";
    if (/Linux|X11|CrOS/i.test(ua)) return "linux";
    return null;
  };

  const platform = platformOf();
  const card = platform && document.getElementById(platform);
  const primary = document.getElementById("primary-download");

  if (card) card.classList.add("yours");

  if (primary) {
    const direct = card && card.querySelector(".f-dl.btn-primary");
    if (platform === "android" || platform === "windows") {
      primary.href = direct.href;
      primary.textContent = platform === "android" ? "Download for Android" : "Download for Windows";
    } else if (platform === "linux") {
      // No way to tell the distribution from a browser, so take them to the
      // choice rather than guess a package format.
      primary.href = "#linux";
      primary.textContent = "Download for Linux";
    } else if (platform === "apple") {
      primary.href = "#download";
      primary.textContent = "See all downloads";
    }
  }

  const humanSize = (bytes) =>
    bytes >= 1048576 ? `${(bytes / 1048576).toFixed(1)} MB` : `${Math.round(bytes / 1024)} KB`;

  fetch("release.json", { cache: "no-cache" })
    .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
    .then((release) => {
      const version = document.getElementById("release-version");
      if (version && release.version) {
        version.textContent = `Version ${release.version} · Free and open source`;
      }
      const sums = document.getElementById("sums");
      for (const asset of release.assets || []) {
        const row = document.querySelector(`.file[data-asset="${CSS.escape(asset.key || asset.name)}"]`);
        const size = row && row.querySelector(".f-size");
        if (size && asset.size) size.textContent = humanSize(asset.size);
        // exact file for this version, no redirect
        const link = row && row.querySelector(".f-dl");
        if (link && asset.url) {
          if (primary && primary.href === link.href) primary.href = asset.url;
          link.href = asset.url;
        }

        if (sums && asset.sha256) {
          const item = document.createElement("li");
          const name = document.createElement("span");
          const sum = document.createElement("code");
          name.textContent = asset.name;
          sum.textContent = asset.sha256;
          sum.title = "Click to copy";
          sum.addEventListener("click", () => {
            navigator.clipboard?.writeText(asset.sha256).then(() => {
              sum.textContent = "Copied";
              setTimeout(() => (sum.textContent = asset.sha256), 1200);
            });
          });
          item.append(name, sum);
          sums.append(item);
        }
      }
    })
    .catch(() => {
      /* The download links work without it; SHA256SUMS.txt has the sums. */
    });
})();
