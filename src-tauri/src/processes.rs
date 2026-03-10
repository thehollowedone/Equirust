use std::{
    ffi::c_void,
    io,
    process::{Child, Command},
};
use tauri::{AppHandle, Manager, Runtime};

#[derive(Default)]
pub struct RuntimeState {
    #[cfg(target_os = "windows")]
    job: std::sync::Mutex<Option<WindowsJob>>,
}

pub fn spawn_managed_child<R: Runtime>(
    app: &AppHandle<R>,
    command: &mut Command,
    purpose: &str,
) -> io::Result<Child> {
    let child = command.spawn()?;
    attach_managed_child(app, &child, purpose);
    Ok(child)
}

pub fn attach_managed_child<R: Runtime>(app: &AppHandle<R>, child: &Child, purpose: &str) {
    let state = app.state::<RuntimeState>();

    if let Err(err) = state.inner().attach_child(child) {
        log::warn!(
            "Failed to register managed child process for {}: {}",
            purpose,
            crate::privacy::sanitize_text_for_log(&err)
        );
    } else {
        log::info!("Registered managed child process for {}", purpose);
    }
}

#[cfg(target_os = "windows")]
struct WindowsJob {
    handle: isize,
}

#[cfg(target_os = "windows")]
impl Drop for WindowsJob {
    fn drop(&mut self) {
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        if self.handle != 0 {
            let _ = unsafe { CloseHandle(HANDLE(self.handle as *mut c_void)) };
        }
    }
}

#[cfg(target_os = "windows")]
impl RuntimeState {
    fn attach_child(&self, child: &Child) -> Result<(), String> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::JobObjects::AssignProcessToJobObject;

        let job = self.ensure_job_handle()?;
        let child_handle = HANDLE(child.as_raw_handle());
        unsafe { AssignProcessToJobObject(HANDLE(job as *mut c_void), child_handle) }
            .map_err(|err| err.to_string())
    }

    fn ensure_job_handle(&self) -> Result<isize, String> {
        let mut guard = self
            .job
            .lock()
            .map_err(|_| "managed process job mutex poisoned".to_owned())?;
        if let Some(job) = guard.as_ref() {
            return Ok(job.handle);
        }

        let handle = create_kill_on_close_job()?;
        *guard = Some(WindowsJob { handle });
        Ok(handle)
    }
}

#[cfg(target_os = "windows")]
fn create_kill_on_close_job() -> Result<isize, String> {
    use std::mem::size_of;
    use windows::core::PCWSTR;
    use windows::Win32::System::JobObjects::{
        CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    let job = unsafe { CreateJobObjectW(None, PCWSTR::null()) }.map_err(|err| err.to_string())?;

    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

    unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    }
    .map_err(|err| {
        use windows::Win32::Foundation::CloseHandle;
        let _ = unsafe { CloseHandle(job) };
        err.to_string()
    })?;

    Ok(job.0 as isize)
}

#[cfg(not(target_os = "windows"))]
impl RuntimeState {
    fn attach_child(&self, _child: &Child) -> Result<(), String> {
        Ok(())
    }
}
