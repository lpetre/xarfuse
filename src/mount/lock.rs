use nix::fcntl;
use nix::sys::stat;
use std::os::unix::io::RawFd;
use std::path::PathBuf;

pub struct Lock {
    fd: RawFd,
}

impl Lock {
    pub fn directory(mount: &PathBuf) -> Result<Lock, failure::Error> {
        let mount_dir = mount.file_name().unwrap();
        let mut lockfile = PathBuf::from(mount.parent().unwrap());
        lockfile.push(format!("lockfile.{}", mount_dir.to_str().unwrap()));
        let flag = fcntl::OFlag::O_RDWR | fcntl::OFlag::O_CREAT | fcntl::OFlag::O_CLOEXEC;
        let mode = stat::Mode::S_IRUSR | stat::Mode::S_IWUSR;

        let fd = fcntl::open(&lockfile, flag, mode)?;
        Ok(Lock { fd: fd })
    }

    #[cfg(target_os = "linux")]
    pub fn touch(self: &Lock) -> Result<(), failure::Error> {
        use nix::sys::time::TimeSpec;
        let now = TimeSpec::utime_now();
        stat::futimens(self.fd, &now, &now)?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    pub fn touch(self: &Lock) -> Result<(), failure::Error> {
        use nix::errno::Errno;
        let res = unsafe { libc::futimes(self.fd, std::ptr::null()) };
        Errno::result(res)?;
        Ok(())
    }
}
