use crate::xar::Xar;

use crate::mount::lock::Lock;
use nix::sys::stat;
use nix::sys::statfs::statfs;
use nix::unistd::{chown, getegid, geteuid, mkdir, Uid};
use std::env;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::PathBuf;

const DEFAULT_MOUNT_ROOTS: &[&str] = &["/mnt/xarfuse", "/dev/shm"];
const PROC_MOUNT_NAMESPACE: &str = "/proc/self/ns/mnt";
const XAR_MOUNT_SEED: &str = "XAR_MOUNT_SEED";

pub struct Directory {
    logger: slog::Logger,
    pub path: PathBuf,
}

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

impl Directory {
    pub fn from_xar(xar: &Xar, logger: slog::Logger) -> Result<Directory, failure::Error> {
        // Path is <mount_root>/uid-N/UUID-ns-Y;
        let mount_root = find_mount_root(&xar.header.mount_root)?;
        let user_directory = get_user_basedir(geteuid());
        let mount_directory = get_mount_dir(&xar.header.uuid);

        let mut result = PathBuf::from(mount_root);
        result.push(user_directory);
        result.push(mount_directory);

        Ok(Directory {
            logger: logger,
            path: result,
        })
    }

    pub fn lock_and_mkdir(self: &Directory) -> Result<Lock, failure::Error> {
        let userdir = PathBuf::from(self.path.parent().unwrap());
        create_directory(&self.logger, &userdir)?;

        let lock = Lock::directory(&self.path)?;
        create_directory(&self.logger, &self.path)?;

        Ok(lock)
    }

    #[cfg(target_os = "macos")]
    pub fn is_mounted(self: &Directory) -> Result<bool, failure::Error> {
        match statfs(&self.path) {
            Ok(stat) => match stat.filesystem_type_name() {
                "osxfuse" | "osxfusefs" => Ok(true),
                _ => Ok(false),
            },
            Err(_) => Ok(false),
        }
    }

    #[cfg(not(target_os = "macos"))]
    pub fn is_mounted(self: &Directory) -> Result<bool, failure::Error> {
        match statfs(&self.path) {
            Ok(stat) => {
                println!("{:?}", stat.filesystem_type());
                Ok(false)
            }
            Err(_) => Ok(false),
        }
    }
}
