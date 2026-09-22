//! Running bundled binaries on Android.

use std::path::{Path, PathBuf};

const SELF: &str = concat!("/lib", env!("CARGO_CRATE_NAME"), ".so");

fn native_lib_dir() -> Option<PathBuf> {
    let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
    maps.lines()

        .filter_map(|line| line.find('/').map(|at| &line[at..]))
        .find(|path| path.ends_with(SELF))
        .and_then(|path| Path::new(path).parent().map(Path::to_path_buf))
}

pub fn executable(name: &str) -> Option<PathBuf> {
    let path = native_lib_dir()?.join(name);
    path.is_file().then_some(path)
}

pub fn keep_descriptors_private(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;

    let max = match unsafe { libc::sysconf(libc::_SC_OPEN_MAX) } {
        n if n > 3 => n as libc::c_int,
        _ => 1024,
    };

    unsafe {
        command.pre_exec(move || {
            for fd in 3..max {
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }
            Ok(())
        });
    }
}
