//! The memory figures of this process that `sysinfo` 0.31 does not give on Windows (spec 0054):
//! the private working set Task Manager's "Memory" column shows, the commit (`PrivateUsage`)
//! and the peak working set, all from one `GetProcessMemoryInfo` call. OneTerm reads the same
//! struct the same way.
//!
//! Every field is 0 where the OS does not give it: on Linux and macOS, when the call fails, and
//! on a Windows older than 10 22H2 / 11 22H2 with the September 2023 update, which fills only the
//! older prefix of the struct. A live process never has a private working set of 0.
//!
//! The decisions about these figures live in `process_usage.rs` and run on every OS; only the
//! Win32 call is `#[cfg(windows)]`, in `windows_process_memory`.

/// Bytes, from `PROCESS_MEMORY_COUNTERS_EX2`. 0 means the figure is not available.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct OsMemory {
    pub(crate) private_working_set: u64,
    /// `PrivateUsage`: committed private bytes, pages never touched included.
    pub(crate) commit: u64,
    pub(crate) peak_working_set: u64,
}

/// This process's counters. Blocking only for the length of one system call.
pub(crate) fn read_os_memory() -> OsMemory {
    #[cfg(windows)]
    {
        windows_process_memory::read().unwrap_or_default()
    }
    #[cfg(not(windows))]
    {
        OsMemory::default()
    }
}

#[cfg(windows)]
// SAFETY (module): the one `unsafe` block below calls a Win32 function with a struct this module
// built itself; it states its own invariants.
#[allow(unsafe_code)]
mod windows_process_memory {
    use windows::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;

    use super::OsMemory;

    /// `None` when the call fails.
    pub(super) fn read() -> Option<OsMemory> {
        let size = size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32;
        let mut counters = PROCESS_MEMORY_COUNTERS_EX2 {
            cb: size,
            ..Default::default()
        };
        // SAFETY: `GetCurrentProcess` returns a pseudo-handle that needs no close. The pointer is
        // to a live, writable struct of exactly `size` bytes; its first fields are laid out as
        // `PROCESS_MEMORY_COUNTERS`, which the API documents for this cast. A Windows without the
        // EX2 struct either rejects the larger `cb` (an error, so `None`) or fills only the older
        // prefix and leaves `PrivateWorkingSetSize` at 0, which the caller treats as unavailable.
        unsafe {
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                (&raw mut counters).cast::<PROCESS_MEMORY_COUNTERS>(),
                size,
            )
        }
        .ok()?;
        Some(OsMemory {
            private_working_set: counters.PrivateWorkingSetSize as u64,
            commit: counters.PrivateUsage as u64,
            peak_working_set: counters.PeakWorkingSetSize as u64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_has_a_private_working_set_where_the_os_gives_one() {
        let memory = read_os_memory();
        if cfg!(windows) {
            // A Windows too old for the EX2 struct reports 0; every current one does not.
            assert!(memory.private_working_set > 0);
            assert!(memory.commit >= memory.private_working_set);
            assert!(memory.peak_working_set >= memory.private_working_set);
        } else {
            assert_eq!(memory, OsMemory::default());
        }
    }
}
