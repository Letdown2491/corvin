//! Process-level secret hygiene.
//!
//! Stops the OS from writing a core dump that could contain a live seed or xpriv
//! during a signing operation. Best-effort and process-wide — it complements the
//! per-buffer `Zeroizing` (which the compiler/OS can still copy or page out) and
//! covers secrets that live in libraries we don't control (BDK, secp256k1). It is
//! NOT a substitute for OS disk-swap encryption.
//!
//! Set `CORVIN_ALLOW_COREDUMP=1` to skip this (e.g. to debug a crash).

/// Disable core dumps for this process. Call once at startup, before any secret is
/// handled. Failures are logged, never fatal — a missing hardening is not worth
/// refusing to start over.
pub fn suppress_core_dumps() {
    if std::env::var_os("CORVIN_ALLOW_COREDUMP").is_some() {
        tracing::warn!("CORVIN_ALLOW_COREDUMP set — core-dump suppression disabled");
        return;
    }

    // RLIMIT_CORE = 0: no core file on crash. POSIX, so Linux + macOS.
    #[cfg(unix)]
    {
        let zero = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: passing a valid initialized rlimit for the documented resource.
        if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &zero) } != 0 {
            tracing::warn!(
                "could not set RLIMIT_CORE=0: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    // PR_SET_DUMPABLE = 0: belt-and-suspenders on Linux — also blocks a same-user
    // process from ptrace-ing us or reading /proc/<pid>/mem. Linux-only.
    #[cfg(target_os = "linux")]
    {
        // SAFETY: prctl with PR_SET_DUMPABLE takes a single value arg; the trailing
        // args are ignored for this option.
        if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) } != 0 {
            tracing::warn!(
                "could not set PR_SET_DUMPABLE=0: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    #[cfg(unix)]
    tracing::debug!("core-dump suppression applied");
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn suppresses_core_dumps() {
        std::env::remove_var("CORVIN_ALLOW_COREDUMP");
        suppress_core_dumps();

        let mut lim = libc::rlimit {
            rlim_cur: 1,
            rlim_max: 1,
        };
        // SAFETY: valid out-param for the documented resource.
        assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_CORE, &mut lim) }, 0);
        assert_eq!(lim.rlim_cur, 0, "core file size limit is zero");

        #[cfg(target_os = "linux")]
        // SAFETY: PR_GET_DUMPABLE returns the flag as its value.
        {
            assert_eq!(
                unsafe { libc::prctl(libc::PR_GET_DUMPABLE) },
                0,
                "process is marked non-dumpable"
            );
        }
    }
}
