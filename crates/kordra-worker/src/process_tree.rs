//! Isolate CLI downloads from unrelated processes; terminate only our own group/job.
use anyhow::{Context, Result};
use tokio::process::{Child, Command};

pub fn configure(command: &mut Command) {
    #[cfg(unix)]
    command.process_group(0);
    command.kill_on_drop(true);
}
pub struct ProcessTree {
    #[cfg(unix)]
    group: i32,
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}
// Windows HANDLE is owned by this guard and may be closed from another thread.
#[cfg(windows)]
unsafe impl Send for ProcessTree {}
impl ProcessTree {
    pub fn attach(child: &Child) -> Result<Self> {
        #[cfg(unix)]
        {
            let group = i32::try_from(child.id().context("missing child PID")?)?;
            anyhow::ensure!(group > 0, "invalid process group");
            Ok(Self { group })
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::{Foundation::CloseHandle, System::JobObjects::*};
            let process = child.raw_handle().context("missing Windows process handle")?;
            // SAFETY: all pointers refer to valid initialized local buffers/OS handles.
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() { return Err(std::io::Error::last_os_error().into()); }
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if SetInformationJobObject(job, JobObjectExtendedLimitInformation,
                    (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of_val(&info) as u32) == 0 ||
                    AssignProcessToJobObject(job, process.cast()) == 0 {
                    let error = std::io::Error::last_os_error();
                    CloseHandle(job);
                    return Err(error).context("attaching child to Windows job object");
                }
                Ok(Self { job })
            }
        }
    }
    pub fn terminate(&self) -> Result<()> {
        #[cfg(unix)]
        {
            // SAFETY: negative PID selects the dedicated group created above.
            if unsafe { libc::kill(-self.group, libc::SIGKILL) } != 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) { return Err(error.into()); }
            }
        }
        #[cfg(windows)]
        {
            // SAFETY: this guard owns the job handle.
            if unsafe { windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job, 1) } == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        Ok(())
    }
}
#[cfg(windows)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        // Closing the final job handle kills remaining members.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.job); }
    }
}

#[cfg(all(test, any(unix, windows)))]
mod tests {
    use super::*;
    use std::process::Stdio;
    use tokio::io::AsyncReadExt;
    #[tokio::test]
    async fn group_cancellation_closes_descendant_pipes() {
        #[cfg(unix)]
        let mut command = {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 30 & wait"]);
            command
        };
        #[cfg(windows)]
        let mut command = {
            let mut command = Command::new("cmd.exe");
            command.args(["/D", "/C", "ping -n 30 127.0.0.1"]);
            command
        };
        configure(&mut command);
        command.stdout(Stdio::piped()).stderr(Stdio::null());
        let mut child = command.spawn().unwrap();
        let tree = ProcessTree::attach(&child).unwrap();
        let mut pipe = child.stdout.take().unwrap();
        // Give the child time to launch a descendant that inherits stdout.
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        tree.terminate().unwrap();
        let status = tokio::time::timeout(std::time::Duration::from_secs(4), child.wait())
            .await.unwrap().unwrap();
        assert!(!status.success());
        let mut bytes = Vec::new();
        tokio::time::timeout(std::time::Duration::from_secs(4), pipe.read_to_end(&mut bytes))
            .await.expect("descendant kept the pipe open").unwrap();
    }
}
