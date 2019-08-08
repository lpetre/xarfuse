extern crate failure;
use crate::xar;
use nix::unistd::{geteuid, Uid};
use std::env;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::PathBuf;

const DEFAULT_MOUNT_ROOTS: &[&str] = &["/mnt/xarfuse", "/dev/shm"];
const PROC_MOUNT_NAMESPACE: &str = "/proc/self/ns/mnt";
const XAR_MOUNT_SEED: &str = "XAR_MOUNT_SEED";

fn find_mount_root(xar: &xar::Header) -> Result<PathBuf, failure::Error> {
    // If provided, use a non-default mount root from the header.
    if let Some(root) = &xar.mount_root {
        let attr = fs::metadata(root)?;
        let permissions = attr.permissions();
        if (permissions.mode() & 0o07777) != 0o01777 {
            bail!("Mount root {} permissions should be 0o01777", root);
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

impl crate::xar::Header {
    pub fn find_mount(&self) -> Result<PathBuf, failure::Error> {
        // Path is <mount_root>/uid-N/UUID-ns-Y;
        let mount_root = find_mount_root(self)?;
        let user_directory = get_user_basedir(geteuid());
        let mount_directory = get_mount_dir(&self.uuid);

        let mut result = PathBuf::from(mount_root);
        result.push(user_directory);
        result.push(mount_directory);

        Ok(result)
    }
}
