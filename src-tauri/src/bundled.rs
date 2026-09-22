//! Executables that ship inside the Android APK.
//!
//! On the desktop, Tor and the Monero wallet daemon are downloaded the first
//! time they are used. Android will not run a file an app downloaded: since
//! Android 10 an app's own data is mounted no-exec for it. What it will run is
//! a file the package installer extracted from the APK's native-library folder
//! — read-only to the app, and verified along with the rest of the package.
//!
//! So both travel as `lib*.so` entries beside this library, put there at build
//! time by `scripts/android-binaries.mjs` after checking each against a pinned
//! SHA-256, and are run from wherever the installer extracted them. Tor's own
//! Android build is already an executable named that way; that is how Tor
//! Browser for Android runs it too.

use std::path::{Path, PathBuf};

/// This library's own file name, from the crate, so a rename cannot leave the
/// search below looking for something that no longer exists.
const SELF: &str = concat!("/lib", env!("CARGO_CRATE_NAME"), ".so");

/// The folder the installer extracted this app's native libraries into.
///
/// Found by looking this library up in the process's own memory map rather
/// than asking the JVM, which would mean a JNI round trip for a path the
/// kernel already has. It only exists when the libraries were extracted — the
/// manifest asks for that with `extractNativeLibs` — because a library mapped
/// straight out of the APK has no folder of its own, and then neither do the
/// executables that were meant to sit beside it.
fn native_lib_dir() -> Option<PathBuf> {
    let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
    maps.lines()
        // The path is the last field and may be the only one containing '/'.
        .filter_map(|line| line.find('/').map(|at| &line[at..]))
        .find(|path| path.ends_with(SELF))
        .and_then(|path| Path::new(path).parent().map(Path::to_path_buf))
}

/// The bundled executable named `name`, if this build carries it.
pub fn executable(name: &str) -> Option<PathBuf> {
    let path = native_lib_dir()?.join(name);
    path.is_file().then_some(path)
}

/// Keeps a child from inheriting this process's open files.
///
/// Rust opens its own descriptors close-on-exec, but the WebView and the
/// Android runtime share this process and do not always. Left alone, Tor and
/// the Monero daemon start out holding the WebView's shared memory, its GPU
/// channels, its sockets, and its data-directory lock. That last one matters:
/// while a child held it, a restarted app could not start its WebView again.
///
/// Marking them close-on-exec, rather than closing them, leaves the pipe Rust
/// uses to report a failed exec working until the exec itself, so a program
/// that cannot start is still reported as one.
pub fn keep_descriptors_private(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;

    // Read in the parent: sysconf is not safe to call between fork and exec.
    let max = match unsafe { libc::sysconf(libc::_SC_OPEN_MAX) } {
        n if n > 3 => n as libc::c_int,
        _ => 1024,
    };

    // SAFETY: the closure runs in the forked child before exec, where only
    // async-signal-safe calls are allowed. fcntl is one, and nothing here
    // allocates or takes a lock. Descriptors 0-2 are already the child's own
    // stdio by this point, so starting at 3 leaves them alone.
    unsafe {
        command.pre_exec(move || {
            for fd in 3..max {
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }
            Ok(())
        });
    }
}

