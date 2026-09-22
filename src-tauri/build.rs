use std::{env, fs, path::Path};

fn main() {
    align_android_for_16k_pages();
    generate_swap_key();
    tauri_build::build()
}

/// Lays the Android shared object out for 16 KB memory pages.
///
/// Pixel 8 and later, and every arm64 phone shipping Android 16, run a 16 KB
/// page. A library whose LOAD segments are aligned to the old 4 KB page
/// cannot be mapped on one, and the dynamic linker refuses it outright:
///
///   dlopen failed: "libshinyflakes_lib.so" program alignment (4096)
///   cannot be smaller than system page size (16384)
///
/// That throws out of `Rust.<clinit>` inside `WryActivity.onCreate`, so the
/// activity dies before it ever constructs a WebView. The window keeps the
/// theme background and the app looks like a dead black screen — no front-end
/// code has run at that point, which is why polyfills and a lower esbuild
/// target never moved it.
///
/// x86_64 hides this: that loader falls back to a 4 KB compatibility mode and
/// only warns. arm64 has no fallback, so every emulator passes and the phone
/// fails.
///
/// NDK r27 and later pass this flag themselves; this project builds against
/// r26, which does not. It belongs here rather than in `.cargo/config.toml`
/// because `RUSTFLAGS` and the working directory Gradle invokes the Tauri CLI
/// from both decide whether such a file is read at all — a build script is
/// part of the crate and is read either way.
fn align_android_for_16k_pages() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!("cargo:rustc-link-arg-cdylib=-Wl,-z,max-page-size=16384");
    }
}

/// Bakes the swap API key into the binary from a gitignored file, so the key
/// never lives in committed source. `swap_key.obf` holds the obfuscated bytes
/// as a comma-separated list; if it is absent (a fresh clone, or the
/// open-source repo) the key is simply empty and swaps report that no key was
/// compiled in. Nothing here is real secrecy — see swapcfg.rs for the honest
/// limits — it only keeps the key out of version control.
fn generate_swap_key() {
    println!("cargo:rerun-if-changed=swap_key.obf");

    let raw = fs::read_to_string("swap_key.obf").unwrap_or_default();
    let list = raw
        .split(',')
        .filter_map(|t| t.trim().parse::<u8>().ok())
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(", ");

    let dest = Path::new(&env::var("OUT_DIR").unwrap()).join("swap_key.rs");
    fs::write(dest, format!("const KEY_OBF: &[u8] = &[{list}];\n")).unwrap();
}
