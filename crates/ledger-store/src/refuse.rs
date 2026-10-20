//! Tests only: system calls refused, as a disk or a file system can
//! refuse them, in one thread of the test's own. A seccomp filter makes
//! each named call fail at once with the error chosen, before the kernel
//! looks at any path, so a refusal no real folder can be made to give
//! (a lookup that fails where making the folder would not; a link
//! refused where the name is already taken) is given every time. The
//! filter holds the calling thread only and ends with it: the rest of
//! the test process is untouched.
#![cfg(target_os = "linux")]

/// Run `f` in a thread of its own in which each call in `calls` fails
/// with `errno`, and give back what it returns; None where the system
/// won't put the filter on (the test then has nothing to show).
pub(crate) fn refusing<T: Send>(
    calls: &[libc::c_long],
    errno: i32,
    f: impl FnOnce() -> T + Send,
) -> Option<T> {
    std::thread::scope(|s| {
        s.spawn(|| {
            // Looked up once before the filter, so std knows statx is
            // there and never mistakes a refused one for a missing one.
            let _ = std::fs::symlink_metadata("/");
            filter(calls, errno).then(f)
        })
        .join()
        .unwrap()
    })
}

/// Put the filter on this thread: the call's number checked against
/// each of `calls`, a match refused with `errno`, anything else let be.
fn filter(calls: &[libc::c_long], errno: i32) -> bool {
    use libc::{
        BPF_ABS, BPF_JEQ, BPF_JMP, BPF_K, BPF_LD, BPF_RET, BPF_W, SECCOMP_RET_ALLOW,
        SECCOMP_RET_ERRNO, sock_filter,
    };
    let stmt = |code: u32, k: u32| sock_filter {
        code: code as u16,
        jt: 0,
        jf: 0,
        k,
    };
    let n = calls.len();
    let mut prog = vec![stmt(
        BPF_LD | BPF_W | BPF_ABS,
        std::mem::offset_of!(libc::seccomp_data, nr) as u32,
    )];
    for (i, call) in calls.iter().enumerate() {
        prog.push(sock_filter {
            code: (BPF_JMP | BPF_JEQ | BPF_K) as u16,
            // To the refusal, past the other checks and the allow.
            jt: u8::try_from(n - i).unwrap(),
            jf: 0,
            k: u32::try_from(*call).unwrap(),
        });
    }
    prog.push(stmt(BPF_RET | BPF_K, SECCOMP_RET_ALLOW));
    prog.push(stmt(
        BPF_RET | BPF_K,
        SECCOMP_RET_ERRNO | u32::try_from(errno).unwrap(),
    ));
    let fprog = libc::sock_fprog {
        len: u16::try_from(prog.len()).unwrap(),
        filter: prog.as_mut_ptr(),
    };
    // SAFETY: prctl with a filter program that outlives the call (the
    // kernel copies it); without TSYNC it binds this thread alone.
    unsafe {
        libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) == 0
            && libc::prctl(
                libc::PR_SET_SECCOMP,
                libc::SECCOMP_MODE_FILTER,
                &fprog as *const libc::sock_fprog,
            ) == 0
    }
}

/// The calls that look a name up without following it (std's
/// `symlink_metadata`): statx, and what std and libc fall back to.
pub(crate) const LOOKUPS: &[libc::c_long] = &[
    libc::SYS_statx,
    libc::SYS_newfstatat,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_lstat,
];

/// The calls that make a hard link (std's `hard_link`).
pub(crate) const LINKS: &[libc::c_long] = &[
    libc::SYS_linkat,
    #[cfg(target_arch = "x86_64")]
    libc::SYS_link,
];
