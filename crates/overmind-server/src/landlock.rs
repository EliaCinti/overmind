//! Landlock: the cage's second layer, where the kernel has one (ADR-0029).
//!
//! The unprivileged uid keeps an agent out of *Overmind's* data. It does not
//! confine a run to its own directory, because every run shares that uid — so
//! one run can reach a sibling's worktree, which macOS has never allowed.
//! Landlock closes that, and closes it in the kernel rather than through a
//! helper binary the user has to have.
//!
//! It is a second layer and not the answer, because it does not exist
//! everywhere: Docker Desktop's kernel ships without it, and that is how
//! everyone on macOS and Windows has Overmind. Asked of the kernel rather than
//! inferred from the platform's name, so a Docker Desktop that enables it one
//! day gains this with no change here.
//!
//! # Why this is written against the raw ABI
//!
//! `libc` carries the syscall numbers and not the structures, and the
//! ergonomic wrappers all default to **best effort** — quietly enforcing less
//! than you asked for when the kernel is older than your policy. That default
//! is the one thing this must not do. ADR-0023 chose the opposite failure
//! direction: when the cage cannot be built the agent does not start, loudly,
//! rather than running with more reach than intended. So the policy here is
//! computed from the ABI the kernel *reports*, and anything that fails is an
//! error rather than a downgrade.
//!
//! Two structures, and one of them is a trap worth naming: the kernel declares
//! `landlock_path_beneath_attr` **packed**, so it is twelve bytes and not
//! sixteen. A natural `#[repr(C)]` would put the file descriptor four bytes
//! late and grant rules nobody wrote.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::Path;

// The same three numbers on every architecture Linux has added them to: they
// entered the generic syscall table together and were backfilled identically.
const SYS_CREATE_RULESET: libc::c_long = 444;
const SYS_ADD_RULE: libc::c_long = 445;
const SYS_RESTRICT_SELF: libc::c_long = 446;

/// Ask for the ABI version instead of creating anything.
const CREATE_RULESET_VERSION: u32 = 1;
/// The only rule type there is, and the only one we want: a path and below it.
const RULE_PATH_BENEATH: libc::c_long = 1;

// `LANDLOCK_ACCESS_FS_*`, in the order the kernel header declares them.
const FS_EXECUTE: u64 = 1 << 0;
const FS_WRITE_FILE: u64 = 1 << 1;
const FS_READ_FILE: u64 = 1 << 2;
const FS_READ_DIR: u64 = 1 << 3;
const FS_REMOVE_DIR: u64 = 1 << 4;
const FS_REMOVE_FILE: u64 = 1 << 5;
const FS_MAKE_CHAR: u64 = 1 << 6;
const FS_MAKE_DIR: u64 = 1 << 7;
const FS_MAKE_REG: u64 = 1 << 8;
const FS_MAKE_SOCK: u64 = 1 << 9;
const FS_MAKE_FIFO: u64 = 1 << 10;
const FS_MAKE_BLOCK: u64 = 1 << 11;
const FS_MAKE_SYM: u64 = 1 << 12;
/// ABI 2 (kernel 5.19): renaming or linking across directories.
const FS_REFER: u64 = 1 << 13;
/// ABI 3 (kernel 6.2): shortening a file you may not otherwise write.
const FS_TRUNCATE: u64 = 1 << 14;

/// Only the first field, deliberately.
///
/// The kernel accepts the ABI-1 size from any version and reads exactly the
/// fields it covers, so asking for the smallest structure that expresses a
/// filesystem policy is also the most portable thing to send. The later fields
/// govern network and IPC scoping, which this does not use: the network is the
/// job ([ADR-0023](../../docs/adr/0023-os-level-sandboxing.md)).
#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
}

/// Packed, because the kernel says so. See the module note.
#[repr(C, packed)]
struct PathBeneathAttr {
    allowed_access: u64,
    parent_fd: i32,
}

/// The Landlock ABI this kernel speaks, or `None` when it speaks none.
///
/// `ENOSYS` here is the ordinary answer on a kernel built without
/// `CONFIG_SECURITY_LANDLOCK` — Docker Desktop's, for one — and `EOPNOTSUPP`
/// the answer when it was built in but left out of the boot-time LSM list.
/// Neither is a failure: they are this layer not applying.
pub fn abi() -> Option<u32> {
    let answer = unsafe {
        libc::syscall(
            SYS_CREATE_RULESET,
            std::ptr::null::<RulesetAttr>(),
            0usize,
            CREATE_RULESET_VERSION,
        )
    };
    (answer > 0).then_some(answer as u32)
}

/// Everything about the filesystem this ABI lets us govern.
///
/// Every right the kernel knows is *handled* — that is what makes the policy
/// deny-by-default, since a right nobody handles is a right granted
/// everywhere. Rights that arrived after the running kernel would be refused
/// outright, so the set is built up to the version it reported.
fn handled_access(abi: u32) -> u64 {
    let mut handled = FS_EXECUTE
        | FS_WRITE_FILE
        | FS_READ_FILE
        | FS_READ_DIR
        | FS_REMOVE_DIR
        | FS_REMOVE_FILE
        | FS_MAKE_CHAR
        | FS_MAKE_DIR
        | FS_MAKE_REG
        | FS_MAKE_SOCK
        | FS_MAKE_FIFO
        | FS_MAKE_BLOCK
        | FS_MAKE_SYM;
    if abi >= 2 {
        handled |= FS_REFER;
    }
    if abi >= 3 {
        handled |= FS_TRUNCATE;
    }
    // `LANDLOCK_ACCESS_FS_IOCTL_DEV` (ABI 5) is left unhandled on purpose. It
    // governs ioctls on device files, not reach into the filesystem, and
    // handling it would deny an agent the terminal-shaped ioctls ordinary
    // tools make on their own stdio. It is not a way out of the cage.
    handled
}

/// What a run may do where it may only look.
fn read_only(handled: u64) -> u64 {
    (FS_EXECUTE | FS_READ_FILE | FS_READ_DIR) & handled
}

/// A built ruleset, held open until the spawn that applies it is done with it.
pub struct Ruleset(OwnedFd);

impl std::fmt::Debug for Ruleset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Ruleset(fd {})", self.0.as_raw_fd())
    }
}

impl Ruleset {
    pub fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

/// Build the policy: deny everything, then grant these.
///
/// Fails rather than returning a weaker ruleset, for the reason in the module
/// note — the caller treats an error as "this layer is not available", never as
/// "this layer is present but smaller than advertised".
pub fn build(write: &[std::path::PathBuf], read: &[&Path]) -> std::io::Result<Ruleset> {
    let Some(abi) = abi() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "this kernel has no Landlock",
        ));
    };
    let handled = handled_access(abi);
    let attr = RulesetAttr {
        handled_access_fs: handled,
    };
    let fd = unsafe {
        libc::syscall(
            SYS_CREATE_RULESET,
            &attr as *const RulesetAttr,
            std::mem::size_of::<RulesetAttr>(),
            0u32,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // Owned from here on, so every early return below closes it.
    let ruleset = unsafe { OwnedFd::from_raw_fd(fd as RawFd) };

    for path in read {
        allow(&ruleset, path, read_only(handled))?;
    }
    for path in write {
        allow(&ruleset, path, handled)?;
    }
    Ok(Ruleset(ruleset))
}

/// Grant `access` on `path` and everything beneath it.
///
/// A path that is not there is a path we do not grant, and that is not an
/// error: this set is written for a family of systems, and `/lib64` is absent
/// on architectures that never needed it. The distinction that matters is
/// between a path we chose not to grant and a *rule we failed to add* — the
/// second means the policy in the kernel is not the policy we wrote, and that
/// one stops the run.
fn allow(ruleset: &OwnedFd, path: &Path, access: u64) -> std::io::Result<()> {
    let Ok(dir) = std::fs::File::open(path) else {
        return Ok(());
    };
    let attr = PathBeneathAttr {
        allowed_access: access,
        parent_fd: dir.as_raw_fd(),
    };
    let added = unsafe {
        libc::syscall(
            SYS_ADD_RULE,
            ruleset.as_raw_fd(),
            RULE_PATH_BENEATH,
            &attr as *const PathBeneathAttr,
            0u32,
        )
    };
    if added != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Put this thread — and everything it goes on to `exec` — inside the ruleset.
///
/// **Async-signal-safe on purpose.** This runs inside `pre_exec`, after `fork`
/// in a process with other threads, where anything that allocates can deadlock
/// on a lock some other thread happened to hold at the moment of the fork. Two
/// bare syscalls and `last_os_error`, which reads `errno` and wraps it without
/// allocating.
///
/// `PR_SET_NO_NEW_PRIVS` first, and not as a formality: `landlock_restrict_self`
/// refuses without it, because a sandbox a setuid binary could step out of
/// would not be one.
pub fn restrict(fd: RawFd) -> std::io::Result<()> {
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    if unsafe { libc::syscall(SYS_RESTRICT_SELF, fd, 0u32) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The structure the kernel reads is packed: twelve bytes, not sixteen.
    ///
    /// Worth a test of its own because getting it wrong is silent. With natural
    /// alignment the file descriptor sits four bytes late, the kernel reads
    /// whatever padding is there, and the rules granted are not the rules
    /// written — a cage that builds cleanly and holds something else.
    #[test]
    fn the_kernels_rule_structure_is_packed() {
        assert_eq!(std::mem::size_of::<PathBeneathAttr>(), 12);
        assert_eq!(std::mem::size_of::<RulesetAttr>(), 8);
    }

    /// The policy grows with the kernel and never past it: a right the running
    /// kernel does not know would be refused outright, taking the whole cage
    /// with it.
    #[test]
    fn the_policy_is_built_up_to_the_abi_the_kernel_reports() {
        let v1 = handled_access(1);
        assert_eq!(v1 & FS_REFER, 0, "REFER arrived in ABI 2");
        assert_eq!(v1 & FS_TRUNCATE, 0, "TRUNCATE arrived in ABI 3");
        assert_ne!(v1 & FS_WRITE_FILE, 0);

        assert_ne!(handled_access(2) & FS_REFER, 0);
        assert_eq!(handled_access(2) & FS_TRUNCATE, 0);
        assert_ne!(handled_access(3) & FS_TRUNCATE, 0);
        // A kernel newer than anything we know about is still governed by
        // everything we do know, rather than by nothing.
        assert_eq!(handled_access(99), handled_access(3));
    }

    /// Looking is not touching. Read-only means read-only, whatever the ABI.
    #[test]
    fn a_readable_path_is_not_a_writable_one() {
        for abi in 1..=5 {
            let allowed = read_only(handled_access(abi));
            assert_eq!(allowed & FS_WRITE_FILE, 0, "abi {abi}");
            assert_eq!(allowed & FS_MAKE_REG, 0, "abi {abi}");
            assert_eq!(allowed & FS_REMOVE_FILE, 0, "abi {abi}");
            assert_eq!(allowed & FS_TRUNCATE, 0, "abi {abi}");
            assert_ne!(allowed & FS_READ_FILE, 0, "abi {abi}");
        }
    }

    /// On a kernel that has it, the policy must actually build — the honest
    /// half of this pair is that on a kernel without it, we say so instead.
    #[test]
    fn a_ruleset_builds_where_the_kernel_has_landlock() {
        let Some(version) = abi() else {
            eprintln!("no Landlock in this kernel — skipping");
            return;
        };
        assert!(version >= 1);
        let dir = std::env::temp_dir();
        let built = build(&[dir], &[Path::new("/usr")]);
        assert!(built.is_ok(), "{built:?}");
    }
}
