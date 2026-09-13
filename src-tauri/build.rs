use std::{env, fs, path::Path};

fn main() {
    generate_swap_key();
    tauri_build::build()
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
