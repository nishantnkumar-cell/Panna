//! Locking down the process that draws pages (see `panna_core::isolate`), before it reads
//! anything from a PDF. What's possible differs by system:
//!
//! - Linux: Landlock takes away every file-system access (files already open, such as the
//!   pipes to the app, still work), seccomp refuses network sockets and starting programs,
//!   and the address space is capped.
//! - Windows: the app starts it in an AppContainer with no capabilities (see `spawn`), so it
//!   can't read the user's files or use the network; failing that, it drops to low integrity, so it can't write to the user's files or
//!   reach other programs' windows, and a job object caps its memory, stops it starting
//!   programs and keeps it off the clipboard and desktop.
//! - macOS: the address space is capped. The system sandbox isn't applied yet.
//!
//! Only the Windows calls need `unsafe`; they're in their own module.

mod spawn;

pub use spawn::{Contained, spawn, spawn_plain};

/// Most memory the process may use, in bytes.
pub const MEMORY: u64 = 4 * 1024 * 1024 * 1024;

/// Locks the calling process down. An error says what couldn't be done; the caller should
/// then not go on to read untrusted data.
pub fn lock_down() -> Result<(), String> {
    system::lock_down()
}

#[cfg(unix)]
fn cap_memory() -> Result<(), String> {
    use rustix::process::{Resource, Rlimit, setrlimit};
    let cap = Rlimit {
        current: Some(MEMORY),
        maximum: Some(MEMORY),
    };
    // No core dumps of PDF data either.
    let none = Rlimit {
        current: Some(0),
        maximum: Some(0),
    };
    setrlimit(Resource::Core, none).map_err(|e| format!("core limit: {e}"))?;
    match setrlimit(Resource::As, cap) {
        Ok(()) => Ok(()),
        // macOS doesn't enforce an address-space limit, and may refuse to set one.
        Err(_) if cfg!(target_os = "macos") => Ok(()),
        Err(e) => Err(format!("memory limit: {e}")),
    }
}

#[cfg(target_os = "linux")]
mod system {
    use std::collections::BTreeMap;

    pub(super) fn lock_down() -> Result<(), String> {
        super::cap_memory()?;
        no_files()?;
        no_network_or_programs()
    }

    /// Every file-system access is handled and none allowed. Kernels without Landlock (before
    /// 5.13, or with it turned off) are left as they are; seccomp still applies.
    fn no_files() -> Result<(), String> {
        use landlock::{ABI, Access, AccessFs, Ruleset, RulesetAttr};
        let abi = ABI::V5;
        Ruleset::default()
            .handle_access(AccessFs::from_all(abi))
            .and_then(|r| r.create())
            .and_then(|r| r.restrict_self())
            .map(|_| ())
            .map_err(|e| format!("landlock: {e}"))
    }

    /// Refuses, with "operation not permitted", the calls that open network connections,
    /// start programs, or reach into other processes.
    fn no_network_or_programs() -> Result<(), String> {
        use seccompiler::{BpfProgram, SeccompAction, SeccompFilter};
        let denied = vec![
            libc::SYS_socket,
            libc::SYS_socketpair,
            libc::SYS_connect,
            libc::SYS_bind,
            libc::SYS_listen,
            libc::SYS_accept,
            libc::SYS_accept4,
            libc::SYS_execve,
            libc::SYS_execveat,
            libc::SYS_ptrace,
            libc::SYS_process_vm_readv,
            libc::SYS_process_vm_writev,
            libc::SYS_mount,
            libc::SYS_umount2,
            libc::SYS_pivot_root,
            libc::SYS_chroot,
            libc::SYS_unshare,
            libc::SYS_setns,
            libc::SYS_keyctl,
            libc::SYS_add_key,
            libc::SYS_request_key,
            libc::SYS_bpf,
            libc::SYS_perf_event_open,
            libc::SYS_userfaultfd,
        ];
        // Elsewhere (aarch64) new processes come only from clone, which threads need too;
        // execve stays refused, so a child could only be another copy of this one.
        #[cfg(target_arch = "x86_64")]
        let denied = [denied, vec![libc::SYS_fork, libc::SYS_vfork]].concat();
        let rules: BTreeMap<i64, Vec<seccompiler::SeccompRule>> =
            denied.into_iter().map(|call| (call, Vec::new())).collect();
        let arch = std::env::consts::ARCH
            .try_into()
            .map_err(|e| format!("seccomp: {e:?}"))?;
        let filter = SeccompFilter::new(
            rules,
            SeccompAction::Allow,
            SeccompAction::Errno(libc::EPERM as u32),
            arch,
        )
        .map_err(|e| format!("seccomp: {e}"))?;
        let program: BpfProgram = filter.try_into().map_err(|e| format!("seccomp: {e:?}"))?;
        seccompiler::apply_filter(&program).map_err(|e| format!("seccomp: {e}"))
    }
}

#[cfg(target_os = "macos")]
mod system {
    pub(super) fn lock_down() -> Result<(), String> {
        super::cap_memory()
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod system {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
    use windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW;
    use windows_sys::Win32::Security::{
        GetTokenInformation, PSID, SID_AND_ATTRIBUTES, SetTokenInformation, TOKEN_ADJUST_DEFAULT,
        TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenIntegrityLevel, TokenIsAppContainer,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
        JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
        JOB_OBJECT_UILIMIT_DESKTOP, JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS,
        JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD,
        JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
        JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicUIRestrictions,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// From winnt.h: the group is an integrity label.
    const SE_GROUP_INTEGRITY: u32 = 0x20;

    pub(super) fn lock_down() -> Result<(), String> {
        job()?;
        // An AppContainer (see `spawn`) already runs at low integrity, and more besides.
        if in_app_container() {
            return Ok(());
        }
        low_integrity()
    }

    /// Whether this process was started in an AppContainer.
    fn in_app_container() -> bool {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: the current-process pseudo handle, and a place for the token handle.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return false;
        }
        let mut is: u32 = 0;
        let mut len = 0u32;
        // SAFETY: an open token, and a u32 the size this class returns.
        let ok = unsafe {
            GetTokenInformation(
                token,
                TokenIsAppContainer,
                (&raw mut is).cast(),
                size_of::<u32>() as u32,
                &mut len,
            ) != 0
        };
        // SAFETY: opened above and not used again.
        unsafe { CloseHandle(token) };
        ok && is != 0
    }

    fn job() -> Result<(), String> {
        // SAFETY: a new unnamed job object with default security; the handle is checked.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err("job object: couldn't be made".into());
        }
        // SAFETY: zeroed is a valid value for these plain C structures.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_PROCESS_MEMORY
            | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
            | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
        limits.BasicLimitInformation.ActiveProcessLimit = 1;
        limits.ProcessMemoryLimit = usize::try_from(super::MEMORY).unwrap_or(usize::MAX);
        let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
            UIRestrictionsClass: JOB_OBJECT_UILIMIT_DESKTOP
                | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
                | JOB_OBJECT_UILIMIT_EXITWINDOWS
                | JOB_OBJECT_UILIMIT_GLOBALATOMS
                | JOB_OBJECT_UILIMIT_HANDLES
                | JOB_OBJECT_UILIMIT_READCLIPBOARD
                | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
                | JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
        };
        // SAFETY: `job` is a valid job handle, and each structure is passed with its own size
        // and the class that matches it. The current-process pseudo handle needs no closing.
        let ok = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) != 0
                && SetInformationJobObject(
                    job,
                    JobObjectBasicUIRestrictions,
                    (&raw const ui).cast(),
                    size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32,
                ) != 0
                && AssignProcessToJobObject(job, GetCurrentProcess()) != 0
        };
        // The job lives as long as the process is in it; the handle is left open on purpose.
        if ok {
            Ok(())
        } else {
            Err("job object: limits not applied".into())
        }
    }

    fn low_integrity() -> Result<(), String> {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: the current-process pseudo handle, and a place for the token handle.
        if unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_DEFAULT | TOKEN_QUERY,
                &mut token,
            )
        } == 0
        {
            return Err("integrity: no token".into());
        }
        let low: Vec<u16> = "S-1-16-4096".encode_utf16().chain([0]).collect();
        let mut sid: PSID = std::ptr::null_mut();
        // SAFETY: a NUL-terminated SID string and a place for the SID, freed below.
        let converted = unsafe { ConvertStringSidToSidW(low.as_ptr(), &mut sid) } != 0;
        let set = converted && {
            let label = TOKEN_MANDATORY_LABEL {
                Label: SID_AND_ATTRIBUTES {
                    Sid: sid,
                    Attributes: SE_GROUP_INTEGRITY,
                },
            };
            // SAFETY: `token` was opened for adjusting, and the label points at a valid SID
            // for the length of the call.
            unsafe {
                SetTokenInformation(
                    token,
                    TokenIntegrityLevel,
                    (&raw const label).cast(),
                    size_of::<TOKEN_MANDATORY_LABEL>() as u32,
                ) != 0
            }
        };
        // SAFETY: each was allocated or opened above and isn't used again.
        unsafe {
            if !sid.is_null() {
                LocalFree(sid);
            }
            CloseHandle(token);
        }
        if set {
            Ok(())
        } else {
            Err("integrity: couldn't be lowered".into())
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod system {
    pub(super) fn lock_down() -> Result<(), String> {
        Err("no sandbox for this system".into())
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    /// Run in a child process, as locking down can't be undone: exits with a bit set for
    /// each thing that still worked.
    #[test]
    fn locked_down_child() {
        if std::env::var_os("PANNA_SANDBOX_CHILD").is_none() {
            return;
        }
        let locked = super::lock_down();
        let mut worked = 0;
        if locked.is_err() {
            worked |= 1;
        }
        if std::net::UdpSocket::bind("127.0.0.1:0").is_ok() {
            worked |= 2;
        }
        if std::process::Command::new("/bin/true").status().is_ok() {
            worked |= 4;
        }
        if landlock_enforced() && std::fs::read("/proc/self/status").is_ok() {
            worked |= 8;
        }
        // Allocating past the cap fails rather than taking the machine's memory.
        let mut big: Vec<u8> = Vec::new();
        if big
            .try_reserve_exact(usize::try_from(super::MEMORY).unwrap_or(usize::MAX))
            .is_ok()
        {
            worked |= 16;
        }
        std::process::exit(worked);
    }

    fn landlock_enforced() -> bool {
        std::fs::read_to_string("/sys/kernel/security/lsm").is_ok_and(|l| l.contains("landlock"))
    }

    #[test]
    fn a_locked_down_process_has_no_network_programs_or_files() {
        let Ok(me) = std::env::current_exe() else { return };
        let status = std::process::Command::new(me)
            .args(["tests::locked_down_child", "--exact", "--test-threads=1"])
            .env("PANNA_SANDBOX_CHILD", "1")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        let code = status.ok().and_then(|s| s.code());
        assert_eq!(code, Some(0), "bits for what still worked: {code:?}");
    }
}
