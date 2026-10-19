//! Starting the renderer. On Windows it starts inside an AppContainer, Windows' app sandbox:
//! with no capabilities it can't open the user's files, reach the network or see other
//! programs, beyond what the job object and low integrity it adds itself already stop. If the
//! AppContainer can't be made (an older Windows, or a policy against it), the renderer starts
//! as a plain process and locks itself down as before. Elsewhere it is always a plain process.

use std::io::{Read, Write};
use std::path::Path;

/// A started renderer: its standard input and output, and the means to stop it.
pub struct Contained {
    pub stdin: Box<dyn Write + Send>,
    pub stdout: Box<dyn Read + Send>,
    process: Process,
    /// Whether it runs in an AppContainer.
    pub app_container: bool,
}

enum Process {
    Plain(std::process::Child),
    #[cfg(windows)]
    Windows(windows::Handle),
}

impl Drop for Contained {
    fn drop(&mut self) {
        self.kill();
    }
}

impl Contained {
    /// Stops it and waits for it to end. Dropping it does the same.
    pub fn kill(&mut self) {
        match &mut self.process {
            Process::Plain(child) => {
                let _ = child.kill();
                let _ = child.wait();
            }
            #[cfg(windows)]
            Process::Windows(handle) => handle.kill(),
        }
    }
}

/// Starts `program` with the one argument `arg`, its standard error discarded, in an
/// AppContainer where there is one.
pub fn spawn(program: &Path, arg: &str) -> std::io::Result<Contained> {
    #[cfg(windows)]
    if let Ok(contained) = windows::spawn(program, arg) {
        return Ok(contained);
    }
    spawn_plain(program, arg)
}

/// As [`spawn`], always as a plain process, for when one in an AppContainer doesn't work.
pub fn spawn_plain(program: &Path, arg: &str) -> std::io::Result<Contained> {
    use std::process::{Command, Stdio};
    let mut child = Command::new(program)
        .arg(arg)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let missing = || std::io::Error::other("no pipe");
    let stdin = child.stdin.take().ok_or_else(missing)?;
    let stdout = child.stdout.take().ok_or_else(missing)?;
    Ok(Contained {
        stdin: Box::new(stdin),
        stdout: Box::new(stdout),
        process: Process::Plain(child),
        app_container: false,
    })
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows {
    use std::fs::File;
    use std::io::Error;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use std::path::Path;

    use windows_sys::Win32::Foundation::{
        CloseHandle, GENERIC_EXECUTE, GENERIC_READ, HANDLE, HANDLE_FLAG_INHERIT, LocalFree,
        SetHandleInformation,
    };
    use windows_sys::Win32::Security::Authorization::{
        EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW, SE_FILE_OBJECT, SetEntriesInAclW,
        SetNamedSecurityInfoW, TRUSTEE_IS_SID, TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::Isolation::{
        CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
    };
    use windows_sys::Win32::Security::{
        ACL, DACL_SECURITY_INFORMATION, FreeSid, NO_INHERITANCE, PSECURITY_DESCRIPTOR, PSID,
        SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES,
    };
    use windows_sys::Win32::System::Pipes::CreatePipe;
    use windows_sys::Win32::System::Threading::{
        CREATE_NO_WINDOW, CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
        INFINITE, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, PROCESS_INFORMATION,
        STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute,
        WaitForSingleObject,
    };

    use super::{Contained, Process};

    /// The AppContainer's name; Windows keeps its profile once made.
    const NAME: &str = "io.github.nishantnkumar_cell.Panna.Renderer";
    /// HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS).
    const ALREADY_EXISTS: i32 = 0x800700B7_u32 as i32;

    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain([0]).collect()
    }

    /// A process handle, closed when dropped.
    pub(super) struct Handle(HANDLE);

    // SAFETY: a process handle may be used and closed from any thread.
    unsafe impl Send for Handle {}

    impl Handle {
        pub(super) fn kill(&mut self) {
            // SAFETY: a live process handle from CreateProcessW.
            unsafe {
                TerminateProcess(self.0, 1);
                WaitForSingleObject(self.0, INFINITE);
            }
        }
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: owned, and not used after this.
            unsafe { CloseHandle(self.0) };
        }
    }

    /// The AppContainer's SID, freed when dropped.
    struct Sid(PSID);

    impl Drop for Sid {
        fn drop(&mut self) {
            // SAFETY: from CreateAppContainerProfile or DeriveAppContainerSidFromAppContainerName,
            // whose SIDs are freed with FreeSid.
            unsafe { FreeSid(self.0) };
        }
    }

    fn container() -> Result<Sid, Error> {
        let name = wide(NAME.as_ref());
        let shown = wide("Panna page renderer".as_ref());
        let mut sid: PSID = std::ptr::null_mut();
        // SAFETY: NUL-terminated strings, no capabilities, and a place for the SID.
        let made = unsafe {
            CreateAppContainerProfile(
                name.as_ptr(),
                shown.as_ptr(),
                shown.as_ptr(),
                std::ptr::null(),
                0,
                &mut sid,
            )
        };
        if made == ALREADY_EXISTS {
            // SAFETY: as above.
            let found = unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) };
            if found != 0 {
                return Err(Error::other(format!("AppContainer: {found:#x}")));
            }
        } else if made != 0 {
            return Err(Error::other(format!("AppContainer: {made:#x}")));
        }
        Ok(Sid(sid))
    }

    /// Lets the AppContainer read and run `program`. Programs installed under Program Files
    /// already allow it; one run from elsewhere (a download, a build folder) needs saying so.
    fn allow_running(program: &[u16], sid: &Sid) -> Result<(), Error> {
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: a NUL-terminated path; the descriptor is freed below.
        let read = unsafe {
            GetNamedSecurityInfoW(
                program.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut dacl,
                std::ptr::null_mut(),
                &mut descriptor,
            )
        };
        if read != 0 {
            return Err(Error::from_raw_os_error(read as i32));
        }
        let access = EXPLICIT_ACCESS_W {
            grfAccessPermissions: GENERIC_READ | GENERIC_EXECUTE,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: NO_INHERITANCE,
            Trustee: TRUSTEE_W {
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
                ptstrName: sid.0.cast(),
                ..TRUSTEE_W::default()
            },
        };
        let mut new: *mut ACL = std::ptr::null_mut();
        // SAFETY: one entry, the file's current list (owned by `descriptor`), and a place for
        // the new list, freed below.
        let merged = unsafe { SetEntriesInAclW(1, &access, dacl, &mut new) };
        let set = if merged == 0 {
            // SAFETY: a NUL-terminated path and a valid list.
            unsafe {
                SetNamedSecurityInfoW(
                    program.as_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    new,
                    std::ptr::null(),
                )
            }
        } else {
            merged
        };
        // SAFETY: both were allocated by the calls above and aren't used again.
        unsafe {
            if !new.is_null() {
                LocalFree(new.cast());
            }
            LocalFree(descriptor);
        }
        if set == 0 {
            Ok(())
        } else {
            Err(Error::from_raw_os_error(set as i32))
        }
    }

    /// An inheritable pipe: (read end, write end).
    fn pipe() -> Result<(HANDLE, HANDLE), Error> {
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: 1,
        };
        let (mut read, mut write): (HANDLE, HANDLE) = (std::ptr::null_mut(), std::ptr::null_mut());
        // SAFETY: places for the two handles, and valid attributes.
        if unsafe { CreatePipe(&mut read, &mut write, &attributes, 0) } == 0 {
            return Err(Error::last_os_error());
        }
        Ok((read, write))
    }

    pub(super) fn spawn(program: &Path, arg: &str) -> Result<Contained, Error> {
        let sid = container()?;
        let path = wide(program.as_os_str());
        // Not fatal: the file may already allow it, and starting will say if not.
        let _ = allow_running(&path, &sid);

        let (child_in, to_child) = pipe()?;
        let (from_child, child_out) = pipe()?;
        // Only the child's ends go to the child.
        // SAFETY: handles just made.
        unsafe {
            SetHandleInformation(to_child, HANDLE_FLAG_INHERIT, 0);
            SetHandleInformation(from_child, HANDLE_FLAG_INHERIT, 0);
        }
        let close_all = |handles: &[HANDLE]| {
            for h in handles {
                // SAFETY: each is a handle made above and not used again.
                unsafe { CloseHandle(*h) };
            }
        };

        let mut size = 0usize;
        // SAFETY: asks only for the size the list needs for two attributes.
        unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 2, 0, &mut size) };
        let mut list_buffer = vec![0u8; size];
        let list: LPPROC_THREAD_ATTRIBUTE_LIST = list_buffer.as_mut_ptr().cast();
        let capabilities = SECURITY_CAPABILITIES {
            AppContainerSid: sid.0,
            Capabilities: std::ptr::null_mut(),
            CapabilityCount: 0,
            Reserved: 0,
        };
        let inherited = [child_in, child_out];
        // SAFETY: the buffer has the size asked for; the values outlive the list, which is
        // deleted below after CreateProcessW.
        let ready = unsafe {
            InitializeProcThreadAttributeList(list, 2, 0, &mut size) != 0
                && UpdateProcThreadAttribute(
                    list,
                    0,
                    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
                    (&raw const capabilities).cast(),
                    size_of::<SECURITY_CAPABILITIES>(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                ) != 0
                && UpdateProcThreadAttribute(
                    list,
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    inherited.as_ptr().cast(),
                    size_of_val(&inherited),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                ) != 0
        };
        if !ready {
            let e = Error::last_os_error();
            close_all(&[child_in, to_child, from_child, child_out]);
            return Err(e);
        }

        // SAFETY: zeroed is a valid value for these plain C structures.
        let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = child_in;
        startup.StartupInfo.hStdOutput = child_out;
        startup.StartupInfo.hStdError = std::ptr::null_mut();
        startup.lpAttributeList = list;
        let quoted = format!("\"{}\" {arg}", program.display());
        let mut command: Vec<u16> = wide(quoted.as_ref());
        // SAFETY: zeroed is valid for PROCESS_INFORMATION.
        let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: a NUL-terminated path and a writable command line; the startup information
        // and its attribute list are valid; only the two listed handles are inherited.
        let started = unsafe {
            CreateProcessW(
                path.as_ptr(),
                command.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW,
                std::ptr::null(),
                std::ptr::null(),
                (&raw const startup).cast(),
                &mut info,
            ) != 0
        };
        let e = Error::last_os_error();
        // SAFETY: the list was initialised above and isn't used again.
        unsafe { DeleteProcThreadAttributeList(list) };
        drop(list_buffer);
        // The child has its own copies now (or failed to start).
        close_all(&[child_in, child_out]);
        if !started {
            close_all(&[to_child, from_child]);
            return Err(e);
        }
        close_all(&[info.hThread]);
        // SAFETY: each handle is owned here and handed to exactly one owner.
        let (stdin, stdout) = unsafe { (File::from_raw_handle(to_child), File::from_raw_handle(from_child)) };
        Ok(Contained {
            stdin: Box::new(stdin),
            stdout: Box::new(stdout),
            process: Process::Windows(Handle(info.hProcess)),
            app_container: true,
        })
    }
}
