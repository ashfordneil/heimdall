use crate::error::{Error, Result};
use std::{
    convert::TryFrom,
    ffi::{CStr, CString},
    io::{ErrorKind, Read},
    mem::MaybeUninit,
    os::{
        raw::{c_char, c_int, c_void},
        unix::io::{AsRawFd, RawFd},
    },
    ptr::NonNull,
};

#[derive(Debug, PartialEq, Eq)]
pub enum FileType {
    Unknown,
    Fifo,
    Character,
    Directory,
    Block,
    Regular,
    Link,
    Socket,
    Whiteout,
}

impl TryFrom<u16> for FileType {
    type Error = Error;

    fn try_from(other: u16) -> Result<Self> {
        match other.to_be_bytes()[0] >> 4 {
            libc::DT_UNKNOWN => Ok(FileType::Unknown),
            libc::DT_FIFO => Ok(FileType::Fifo),
            libc::DT_CHR => Ok(FileType::Character),
            libc::DT_DIR => Ok(FileType::Directory),
            libc::DT_BLK => Ok(FileType::Block),
            libc::DT_REG => Ok(FileType::Regular),
            libc::DT_LNK => Ok(FileType::Link),
            libc::DT_SOCK => Ok(FileType::Socket),
            // libc doesn't have the macos specific things
            14 => Ok(FileType::Whiteout),
            mystery => Err(Error::InvalidFileType(mystery)),
        }
    }
}

/// A very simple wrapper around a file (or directory).
#[derive(Debug, PartialEq, Eq)]
pub struct File {
    fd: RawFd,
}

impl File {
    /// Flag for open that lets you open symlinks as if they're real files.
    const O_SYMLINK: c_int = 0x200000;

    fn increase_ulimits() -> Result<()> {
        let mut limit = MaybeUninit::uninit();
        let mut limit = unsafe {
            Error::from_int(libc::getrlimit(libc::RLIMIT_NOFILE, limit.as_mut_ptr()))?;
            limit.assume_init()
        };
        println!(
            "Changing limit from {} to {}",
            limit.rlim_cur,
            limit.rlim_cur * 2
        );
        limit.rlim_cur *= 2;
        Error::from_int(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) })?;

        Ok(())
    }

    fn open_raw(path: &CStr) -> Result<Self> {
        let fd = Error::from_int(unsafe {
            libc::open(path.as_ptr(), libc::O_RDONLY | Self::O_SYMLINK)
        })?;
        Ok(File { fd })
    }

    /// Open a new file, using a path that's relative to the current directory.
    pub fn open(path: &CStr) -> Result<Self> {
        match Self::open_raw(path) {
            Err(Error::IoError(error)) if error.raw_os_error() == Some(libc::EMFILE) => {
                Self::increase_ulimits()?;
                Self::open_raw(path)
            }
            regular => regular,
        }
    }

    fn open_at_raw(&self, path: &CStr) -> Result<Self> {
        let fd = Error::from_int(unsafe {
            libc::openat(self.fd, path.as_ptr(), libc::O_RDONLY | Self::O_SYMLINK)
        })?;
        Ok(File { fd })
    }

    /// Open a new file that is a child of this file (assuming this file is a directory).
    pub fn open_at(&self, path: &CStr) -> Result<Self> {
        match self.open_at_raw(path) {
            Err(Error::IoError(error)) if error.raw_os_error() == Some(libc::EMFILE) => {
                Self::increase_ulimits()?;
                self.open_at(path)
            }
            regular => regular,
        }
    }

    /// Gets some metadata (file type and inode number) from this file.
    pub fn stat(&self) -> Result<(FileType, u64)> {
        let mut buf = MaybeUninit::uninit();
        Error::from_int(unsafe { libc::fstat(self.fd, buf.as_mut_ptr()) })?;
        let buf = unsafe { buf.assume_init() };
        let file_type = FileType::try_from(buf.st_mode)?;
        let inode = buf.st_ino;

        Ok((file_type, inode))
    }

    /// Gets some metadata (file type and inode number) form a child of this file.
    pub fn stat_at(&self, path: &CStr) -> Result<(FileType, u64)> {
        let mut buf = MaybeUninit::uninit();
        Error::from_int(unsafe {
            libc::fstatat(
                self.fd,
                path.as_ptr(),
                buf.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        })?;
        let buf = unsafe { buf.assume_init() };
        let file_type = FileType::try_from(buf.st_mode)?;
        let inode = buf.st_ino;

        Ok((file_type, inode))
    }

    /// Scan this directory, find the names of all of the child files within this directory. Skips
    /// .. entries.
    pub fn scan(&self) -> Result<Vec<CString>> {
        let fd_clone = Error::from_int(unsafe { libc::dup(self.fd) })?;
        let dirp = Error::from_ptr(unsafe { libc::fdopendir(fd_clone) })?.as_ptr();
        scopeguard::defer! {
            if let Err(err) = Error::from_int(unsafe { libc::closedir(dirp) }) {
                log::warn!("Error closing directory {} - {}", fd_clone, err);
            }
        };

        Error::with_errno(|| {
            let mut output = Vec::new();
            while let Some(entry) = NonNull::new(unsafe { libc::readdir(dirp) }) {
                let name = unsafe {
                    let entry = entry.as_ref();
                    let name = CStr::from_ptr(entry.d_name[..].as_ptr());
                    name.to_owned()
                };
                if name.as_bytes() == b".." {
                    continue;
                }
                output.push(name);
            }
            output
        })
    }

    /// Find the file that this symlink links to.
    pub fn get_link_name(&self, name: &CStr) -> Result<CString> {
        let mut buf = Vec::with_capacity(1024);
        let length = Error::from_size(unsafe {
            libc::readlinkat(
                self.fd,
                name.as_ptr(),
                buf.as_mut_ptr() as *mut c_char,
                buf.len(),
            )
        })?;
        buf.truncate(length as usize);
        Ok(CString::new(buf).expect("Nul byte in resolved symlink name"))
    }
}

impl Read for File {
    // Be careful. The file struct can be a type of file that doesn't support the read operation,
    // and in that case this operation will fail.
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut c_void, buf.len()) } {
            -1 => Err(std::io::Error::last_os_error()),
            n if n < 0 => unreachable!(),
            n => Ok(n as usize),
        }
    }
}

impl AsRawFd for File {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

impl Drop for File {
    fn drop(&mut self) {
        if let Err(error) = Error::from_int(unsafe { libc::close(self.fd) }) {
            log::warn!("Error closing file: {}", error);
        }
    }
}
