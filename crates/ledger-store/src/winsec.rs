//! Windows owner-only security (P11c, D-62): what mode 0700/0600 and
//! `getuid` are on Unix.
//!
//! ```text
//! GOAL: the data root, its folders and files, and the local pipes are
//!       this account's alone, and a process can say whose another
//!       process is.
//! SOURCE: plan/served.md 3.4 ("the data root ... an owner-only DACL
//!         where Unix has mode 0700"), 3.9 ("named pipes with the SID
//!         check where Unix checks peer uid"), D-62; Microsoft's
//!         documentation of SetNamedSecurityInfoW, SDDL strings,
//!         GetNamedPipeClientProcessId and SHGetKnownFolderPath (cited,
//!         not copied).
//! INVARIANTS:
//!   1. An owner-only DACL is protected (it takes nothing from the folder
//!      above) and holds one allow entry, for this account's SID;
//!      folders' entry is inherited by what is made inside them.
//!   2. A path is private when its owner is this account (or the
//!      Administrators group a elevated account's files are owned by)
//!      and no allow entry names anyone but this account, SYSTEM or the
//!      Administrators; a missing DACL (everyone allowed) is not private.
//!   3. Every question that cannot be answered (a handle that won't open,
//!      a token that won't read) is "no": nothing here fails open.
//! ```

use std::ffi::{OsStr, OsString, c_void};
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    GetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT, SetNamedSecurityInfoW,
};
use windows_sys::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetLengthSid, GetSecurityDescriptorDacl,
    GetTokenInformation, IsWellKnownSid, OWNER_SECURITY_INFORMATION,
    PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES,
    TOKEN_QUERY, TOKEN_USER, TokenUser, WinBuiltinAdministratorsSid, WinLocalSystemSid,
};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Shell::{
    FOLDERID_LocalAppData, FOLDERID_Profile, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
};

/// An ACCESS_ALLOWED_ACE_TYPE entry's type byte.
const ACCESS_ALLOWED: u8 = 0;

/// A handle closed when dropped.
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: a handle this value opened and owns, closed once.
        unsafe { CloseHandle(self.0) };
    }
}

/// A SID: an owned copy of its bytes.
#[derive(Clone, Debug)]
pub struct Sid(Vec<u64>);

impl Sid {
    /// A copy of the SID `sid` points at.
    ///
    /// # Safety
    /// `sid` must point at a valid SID.
    unsafe fn copy_of(sid: PSID) -> Sid {
        // SAFETY: the caller's promise that `sid` is a valid SID.
        let len = unsafe { GetLengthSid(sid) } as usize;
        let mut words = vec![0u64; len.div_ceil(8)];
        // SAFETY: `len` bytes of a valid SID into a buffer of at least
        // `len` bytes; the buffer is 8-byte aligned.
        unsafe { std::ptr::copy_nonoverlapping(sid.cast::<u8>(), words.as_mut_ptr().cast(), len) };
        Sid(words)
    }

    fn as_psid(&self) -> PSID {
        self.0.as_ptr() as PSID
    }

    /// The SID as the text SDDL uses (`S-1-5-21-...`).
    pub fn to_text(&self) -> io::Result<String> {
        let mut out: *mut u16 = std::ptr::null_mut();
        // SAFETY: a valid SID, and an out pointer to receive an allocation.
        if unsafe { ConvertSidToStringSidW(self.as_psid(), &mut out) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let text = from_wide_ptr(out);
        // SAFETY: the allocation ConvertSidToStringSidW made, freed once.
        unsafe { LocalFree(out.cast()) };
        text.into_string()
            .map_err(|_| io::Error::other("a SID that is not text"))
    }
}

impl PartialEq for Sid {
    fn eq(&self, other: &Sid) -> bool {
        // SAFETY: both are valid SIDs copied from the system.
        unsafe { EqualSid(self.as_psid(), other.as_psid()) != 0 }
    }
}

impl Eq for Sid {}

/// The text a NUL-terminated wide string holds.
fn from_wide_ptr(p: *const u16) -> OsString {
    let mut len = 0;
    // SAFETY: the caller passes a NUL-terminated string the system made.
    while unsafe { *p.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: `len` units were just read from it.
    OsString::from_wide(unsafe { std::slice::from_raw_parts(p, len) })
}

/// A path or text as a NUL-terminated wide string.
pub fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// The user SID of the token `token` (TOKEN_QUERY).
fn token_user(token: HANDLE) -> io::Result<Sid> {
    let mut needed = 0u32;
    // SAFETY: asking only for the size; the call is expected to fail
    // with "insufficient buffer" and set `needed`.
    unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed) };
    if needed == 0 {
        return Err(io::Error::last_os_error());
    }
    // 8-byte aligned for TOKEN_USER's pointers.
    let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
    // SAFETY: a buffer of at least `needed` bytes.
    if unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buf.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the buffer now holds a TOKEN_USER whose SID points inside it.
    Ok(unsafe { Sid::copy_of((*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid) })
}

/// This process's account.
pub fn current_user() -> io::Result<Sid> {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: the pseudo-handle of this process, and an out handle.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = Owned(token);
    token_user(token.0)
}

/// The account process `pid` runs as. An error when it can't be told
/// (a process of another account usually can't be opened at all).
pub fn process_user(pid: u32) -> io::Result<Sid> {
    // SAFETY: a plain open of a process by number.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }
    let process = Owned(process);
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: a process handle just opened, and an out handle.
    if unsafe { OpenProcessToken(process.0, TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = Owned(token);
    token_user(token.0)
}

/// A security descriptor the system allocated, freed when dropped.
struct Descriptor(PSECURITY_DESCRIPTOR);

impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: an allocation of the system's, freed once.
        unsafe { LocalFree(self.0) };
    }
}

/// The descriptor of SDDL `text`.
fn from_sddl(text: &str) -> io::Result<Descriptor> {
    let wide = wide(OsStr::new(text));
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: a NUL-terminated string and an out pointer.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide.as_ptr(),
            SDDL_REVISION_1,
            &mut sd,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Descriptor(sd))
}

/// The SDDL of a DACL that holds one allow entry, full access for this
/// account, protected; for a folder, inherited by what it holds.
fn owner_only_sddl(folder: bool) -> io::Result<String> {
    let sid = current_user()?.to_text()?;
    Ok(if folder {
        format!("D:P(A;OICI;FA;;;{sid})")
    } else {
        format!("D:P(A;;FA;;;{sid})")
    })
}

/// Make `path` this account's alone (INVARIANT 1): the mode 0700 of a
/// folder, the 0600 of a file.
pub fn set_owner_only(path: &Path) -> io::Result<()> {
    let folder = std::fs::metadata(path)?.is_dir();
    let sd = from_sddl(&owner_only_sddl(folder)?)?;
    let mut present = 0;
    let mut defaulted = 0;
    let mut dacl: *mut ACL = std::ptr::null_mut();
    // SAFETY: a descriptor the system made, and out values.
    if unsafe { GetSecurityDescriptorDacl(sd.0, &mut present, &mut dacl, &mut defaulted) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let name = wide(path.as_os_str());
    // SAFETY: a NUL-terminated path, no owner or group to set, and a DACL
    // that lives in `sd` past the call.
    let status = unsafe {
        SetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            dacl,
            std::ptr::null(),
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    Ok(())
}

/// Whether the owner and the allow entries of `path` are this account's
/// (INVARIANT 2): the answer to "do others have access".
pub fn is_private(path: &Path) -> io::Result<bool> {
    let me = current_user()?;
    let name = wide(path.as_os_str());
    let mut owner: PSID = std::ptr::null_mut();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: a NUL-terminated path and out pointers; the descriptor the
    // call allocates holds the owner and the DACL and is freed below.
    let status = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let _free = Descriptor(sd);
    // SAFETY: `owner` points into `sd`, alive here.
    let owner_ok = !owner.is_null()
        && (unsafe { EqualSid(owner, me.as_psid()) } != 0
            || unsafe { IsWellKnownSid(owner, WinBuiltinAdministratorsSid) } != 0);
    if !owner_ok || dacl.is_null() {
        return Ok(false);
    }
    // SAFETY: `dacl` points into `sd`; its entries are read in place.
    let count = unsafe { (*dacl).AceCount };
    for i in 0..u32::from(count) {
        let mut ace: *mut c_void = std::ptr::null_mut();
        // SAFETY: an index below the entry count of a valid ACL.
        if unsafe { GetAce(dacl, i, &mut ace) } == 0 {
            return Ok(false);
        }
        // SAFETY: every ACE starts with its header.
        let header = unsafe { *ace.cast::<windows_sys::Win32::Security::ACE_HEADER>() };
        if header.AceType != ACCESS_ALLOWED {
            // Deny entries only take access away; the object-specific
            // allow types don't occur on files, and one is not private.
            if (1..=3).contains(&header.AceType) {
                continue;
            }
            return Ok(false);
        }
        // SAFETY: an ACCESS_ALLOWED_ACE's SID starts at SidStart.
        let sid = unsafe {
            std::ptr::addr_of_mut!(
                (*ace.cast::<windows_sys::Win32::Security::ACCESS_ALLOWED_ACE>()).SidStart
            )
        }
        .cast::<c_void>();
        // SAFETY: a valid SID inside the ACE.
        let ours = unsafe {
            EqualSid(sid, me.as_psid()) != 0
                || IsWellKnownSid(sid, WinLocalSystemSid) != 0
                || IsWellKnownSid(sid, WinBuiltinAdministratorsSid) != 0
        };
        if !ours {
            return Ok(false);
        }
    }
    Ok(true)
}

/// A known folder's path, from the shell (never an environment variable
/// the process that started us chose).
fn known_folder(id: &windows_sys::core::GUID) -> Option<PathBuf> {
    let mut out: *mut u16 = std::ptr::null_mut();
    // SAFETY: a known-folder id, the current account's token (null), and an
    // out pointer for a string the shell allocates.
    let hr =
        unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT as u32, std::ptr::null_mut(), &mut out) };
    if hr != 0 || out.is_null() {
        if !out.is_null() {
            // SAFETY: freed once, as the API asks even on failure.
            unsafe { CoTaskMemFree(out.cast()) };
        }
        return None;
    }
    let path = PathBuf::from(from_wide_ptr(out));
    // SAFETY: the shell's allocation, freed once.
    unsafe { CoTaskMemFree(out.cast()) };
    path.is_absolute().then_some(path)
}

/// This account's profile folder (`C:\Users\name`).
pub fn profile_folder() -> Option<PathBuf> {
    known_folder(&FOLDERID_Profile)
}

/// This account's local application data folder (`%LOCALAPPDATA%`,
/// wherever it was redirected to).
pub fn local_app_data() -> Option<PathBuf> {
    known_folder(&FOLDERID_LocalAppData)
}

/// Rename `from` to `to`, never over something already at `to`, and
/// written through to disk.
pub fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
    let (f, t) = (wide(from.as_os_str()), wide(to.as_os_str()));
    // SAFETY: two NUL-terminated paths; no REPLACE_EXISTING flag, so an
    // existing target is an error.
    if unsafe { MoveFileExW(f.as_ptr(), t.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Security attributes that make a kernel object (a named pipe) this
/// account's alone, for a call that takes `SECURITY_ATTRIBUTES`.
pub struct OwnerOnly {
    _descriptor: Descriptor,
    attributes: SECURITY_ATTRIBUTES,
}

// SAFETY: the descriptor is immutable memory that nothing else touches.
unsafe impl Send for OwnerOnly {}
unsafe impl Sync for OwnerOnly {}

impl OwnerOnly {
    /// A descriptor with one allow entry, full access for this account.
    pub fn new() -> io::Result<OwnerOnly> {
        let sddl = format!("D:P(A;;GA;;;{})", current_user()?.to_text()?);
        let descriptor = from_sddl(&sddl)?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: 0,
        };
        Ok(OwnerOnly {
            _descriptor: descriptor,
            attributes,
        })
    }

    /// The pointer `create_with_security_attributes_raw` takes; good while
    /// this lives.
    pub fn as_raw(&self) -> *mut c_void {
        std::ptr::addr_of!(self.attributes).cast_mut().cast()
    }
}

/// This process's account, asked of the system once.
fn me() -> Option<&'static Sid> {
    static ME: std::sync::OnceLock<Option<Sid>> = std::sync::OnceLock::new();
    ME.get_or_init(|| current_user().ok()).as_ref()
}

/// Whether process `pid` runs as this account (INVARIANT 3: any doubt is
/// "no").
pub fn pid_is_us(pid: u32) -> bool {
    pid != 0 && me().is_some_and(|me| process_user(pid).is_ok_and(|them| &them == me))
}

/// The process at the client end of the pipe `handle` (a server's).
pub fn pipe_client_pid(pipe: &impl std::os::windows::io::AsRawHandle) -> io::Result<u32> {
    let mut pid = 0u32;
    // SAFETY: a pipe handle the caller owns and an out value.
    if unsafe {
        windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId(
            pipe.as_raw_handle(),
            &mut pid,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(pid)
}

/// The process at the server end of the pipe `handle` (a client's).
pub fn pipe_server_pid(pipe: &impl std::os::windows::io::AsRawHandle) -> io::Result<u32> {
    let mut pid = 0u32;
    // SAFETY: a pipe handle the caller owns and an out value.
    if unsafe {
        windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId(
            pipe.as_raw_handle(),
            &mut pid,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(pid)
}

/// How many bytes wait to be read on the pipe `handle`, without reading
/// them. A pipe whose other end has gone is `ERROR_BROKEN_PIPE`.
pub fn pipe_bytes_waiting(pipe: &impl std::os::windows::io::AsRawHandle) -> io::Result<u32> {
    let mut avail = 0u32;
    // SAFETY: a pipe handle the caller owns; only the available count is
    // asked for.
    let ok = unsafe {
        windows_sys::Win32::System::Pipes::PeekNamedPipe(
            pipe.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            &mut avail,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(avail)
}
