extern crate failure;
use crate::xar::Xar;
use nix::fcntl;
use nix::sys::stat;
use nix::sys::statfs::statfs;
use nix::unistd::{chown, getegid, geteuid, mkdir, Uid};
use std::env;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::io::RawFd;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_MOUNT_ROOTS: &[&str] = &["/mnt/xarfuse", "/dev/shm"];
const PROC_MOUNT_NAMESPACE: &str = "/proc/self/ns/mnt";
const XAR_MOUNT_SEED: &str = "XAR_MOUNT_SEED";

fn find_mount_root(mount_root: &Option<String>) -> Result<PathBuf, failure::Error> {
    // If provided, use a non-default mount root from the header.
    if let Some(root) = mount_root {
        let attr = fs::metadata(&root)?;
        let permissions = attr.permissions();
        if (permissions.mode() & 0o07777) != 0o01777 {
            bail!("Mount root {} permissions should be 0o01777", &root);
        }
        return Ok(PathBuf::from(root));
    }

    // Otherwise find the first proper mount root from our list of defaults.
    for candidate in DEFAULT_MOUNT_ROOTS {
        if let Ok(attr) = fs::metadata(candidate) {
            let permissions = attr.permissions();
            if (permissions.mode() & 0o07777) == 0o01777 {
                return Ok(PathBuf::from(candidate));
            }
        }
    }
    Err(format_err!("Unable to find suitable 0o01777 mount root."))
}

fn get_user_basedir(uid: Uid) -> String {
    format!("uid-{}", uid)
}

fn get_mount_dir(uuid: &str) -> String {
    let mut mount_directory = String::from(uuid);

    // We optionally also take a user-specified "seed" from the environment.  We cannot rely
    // purely on mount namespace as the kernel will aggressively re-use namespace IDs.
    if let Ok(seed) = env::var(XAR_MOUNT_SEED) {
        if !seed.is_empty() && !seed.contains('/') {
            mount_directory = format!("{}-seed-{}", mount_directory, seed);
        }
    }

    // Determine our mount namespace id via the inode on /proc/self/ns/mnt
    if let Ok(attr) = fs::metadata(PROC_MOUNT_NAMESPACE) {
        mount_directory = format!("{}-ns-{}", mount_directory, attr.ino());
    }

    mount_directory
}

fn create_directory(logger: &slog::Logger, dir: &PathBuf) -> Result<(), failure::Error> {
    let mode = stat::Mode::S_IRWXU
        | stat::Mode::S_IRGRP
        | stat::Mode::S_IXGRP
        | stat::Mode::S_IROTH
        | stat::Mode::S_IXOTH;
    if !dir.exists() {
        debug!(logger, "Creating directory"; "dir" => dir.to_str().unwrap());

        mkdir(dir, mode)?;
        chown(dir, Some(geteuid()), Some(getegid()))?
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn is_mounted(mount: &PathBuf) -> Result<bool, failure::Error> {
    match statfs(mount) {
        Ok(stat) => match stat.filesystem_type_name() {
            "osxfuse" | "osxfusefs" => Ok(true),
            _ => Ok(false),
        },
        Err(_) => Ok(false),
    }
}

#[cfg(not(target_os = "macos"))]
fn is_mounted(mount: &PathBuf) -> Result<bool, failure::Error> {
    match statfs(mount) {
        Ok(stat) => {
            println!("{:?}", stat.filesystem_type());
            Ok(false)
        }
        Err(_) => Ok(false),
    }
}

fn lock_directory(mount: &PathBuf) -> Result<RawFd, failure::Error> {
    let mount_dir = mount.file_name().unwrap();
    let mut lockfile = PathBuf::from(mount.parent().unwrap());
    lockfile.push(format!("lockfile.{}", mount_dir.to_str().unwrap()));
    let flag = fcntl::OFlag::O_RDWR | fcntl::OFlag::O_CREAT | fcntl::OFlag::O_CLOEXEC;
    let mode = stat::Mode::S_IRUSR | stat::Mode::S_IWUSR;

    let fd = fcntl::open(&lockfile, flag, mode)?;
    Ok(fd)
}

#[cfg(target_os = "linux")]
fn touch_lock(lock_fd: RawFd) -> Result<(), failure::Error> {
    use nix::sys::time::TimeSpec;
    let now = TimeSpec::utime_now();
    stat::futimens(lock_fd, &now, &now)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn touch_lock(lock_fd: RawFd) -> Result<(), failure::Error> {
    use nix::errno::Errno;
    let res = unsafe { libc::futimes(lock_fd, std::ptr::null()) };
    Errno::result(res)?;
    Ok(())
}

impl Xar {
    pub fn find_mount(&self) -> Result<PathBuf, failure::Error> {
        // Path is <mount_root>/uid-N/UUID-ns-Y;
        let mount_root = find_mount_root(&self.header.mount_root)?;
        let user_directory = get_user_basedir(geteuid());
        let mount_directory = get_mount_dir(&self.header.uuid);

        let mut result = PathBuf::from(mount_root);
        result.push(user_directory);
        result.push(mount_directory);

        Ok(result)
    }

    pub fn mount(&self, mount: PathBuf) -> Result<(), failure::Error> {
        let userdir = PathBuf::from(mount.parent().unwrap());
        create_directory(&self.logger, &userdir)?;

        let lock_fd = lock_directory(&mount)?;
        create_directory(&self.logger, &mount)?;

        if !is_mounted(&mount)? {
            let opts = vec![
                format!("offset={}", self.header.offset),
                format!("timeout={}", 870),
            ];

            debug!(self.logger, "Mounting"; "mount" => mount.to_str().unwrap(), "archive" => &self.archive.to_str().unwrap());
            let mut cmd = Command::new("squashfuse_ll")
                .arg(format!("-o{}", opts.join(",")))
                .arg(&self.archive)
                .arg(&mount)
                .spawn()?;

            let status = cmd.wait()?;
            if !status.success() {
                match status.code() {
                    Some(code) => bail!("Exited with status code: {}", code),
                    None => bail!("Process terminated by signal: {:?}", status.signal()),
                }
            }
        } else {
            debug!(self.logger, "Mounted"; "mount" => mount.to_str().unwrap());
        }

        // Wait for up to 9 seconds for mount to be available
        let start = Instant::now();
        let timeout = Duration::from_secs(9);
        let sleep = Duration::from_micros(100);
        while !is_mounted(&mount)? {
            if start.elapsed() > timeout {
                bail!("Timed out waiting for mount");
            }
            thread::sleep(sleep);
        }

        // Touch the lockfile
        touch_lock(lock_fd)?;

        Ok(())
    }
}
