extern crate failure;

pub mod directory;
pub mod lock;
use crate::mount::directory::Directory;
use crate::xar::Xar;

use std::os::unix::process::ExitStatusExt;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

impl Xar {
    pub fn mount(&self, mount: &Directory) -> Result<(), failure::Error> {
        let lock = mount.lock_and_mkdir()?;

        if !mount.is_mounted()? {
            let opts = vec![
                format!("offset={}", self.header.offset),
                format!("timeout={}", 870),
            ];

            debug!(
                self.logger,
                "Mounting";
                "mount" => mount.path.to_str().unwrap_or_default(),
                "archive" => &self.archive.to_str().unwrap_or_default()
            );
            let mut cmd = Command::new("squashfuse_ll")
                .arg(format!("-o{}", opts.join(",")))
                .arg(&self.archive)
                .arg(&mount.path)
                .spawn()?;

            let status = cmd.wait()?;
            if !status.success() {
                match status.code() {
                    Some(code) => bail!("Exited with status code: {}", code),
                    None => bail!("Process terminated by signal: {:?}", status.signal()),
                }
            }
        } else {
            debug!(
                self.logger,
                "Mounted";
                "mount" => mount.path.to_str().unwrap_or_default(),
            );
        }

        // Wait for up to 9 seconds for mount to be available
        let start = Instant::now();
        let timeout = Duration::from_secs(9);
        let sleep = Duration::from_micros(100);
        while !mount.is_mounted()? {
            if start.elapsed() > timeout {
                bail!("Timed out waiting for mount");
            }
            thread::sleep(sleep);
        }

        // Touch the lockfile
        lock.touch()?;

        Ok(())
    }
}
