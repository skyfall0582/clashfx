use anyhow::{Context, Result};
use std::ops::{Deref, DerefMut};
use tokio::process::{Child, Command};

pub struct OwnedChild {
    child: Child,
    #[cfg(windows)]
    _job: std::os::windows::io::OwnedHandle,
}

impl OwnedChild {
    pub fn spawn(command: &mut Command) -> Result<Self> {
        let mut child = command.kill_on_drop(true).spawn()?;
        #[cfg(windows)]
        let job = match assign_job(&child) {
            Ok(job) => job,
            Err(error) => {
                child.start_kill().ok();
                return Err(error.context("无法建立内核退出保护"));
            }
        };
        Ok(Self {
            child,
            #[cfg(windows)]
            _job: job,
        })
    }

    pub async fn output(self) -> std::io::Result<std::process::Output> {
        let Self {
            child,
            #[cfg(windows)]
            _job,
        } = self;
        child.wait_with_output().await
    }
}

impl Deref for OwnedChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.child
    }
}
impl DerefMut for OwnedChild {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}

#[cfg(windows)]
fn assign_job(child: &Child) -> Result<std::os::windows::io::OwnedHandle> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::JobObjects::*;
    unsafe {
        let raw = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        anyhow::ensure!(!raw.is_null(), std::io::Error::last_os_error());
        let job = OwnedHandle::from_raw_handle(raw);
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        anyhow::ensure!(
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            ) != 0,
            std::io::Error::last_os_error()
        );
        anyhow::ensure!(
            AssignProcessToJobObject(
                job.as_raw_handle(),
                child.raw_handle().context("内核已退出")?
            ) != 0,
            std::io::Error::last_os_error()
        );
        Ok(job)
    }
}
