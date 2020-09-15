use crate::fs::FileType;
use std::ptr::NonNull;

use errno::Errno;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("IO failure {0}")]
    IoError(#[from] std::io::Error),
    #[error("Invalid file type {0}")]
    InvalidFileType(u8),
    #[error("Unsupported file type {0:?}")]
    UnsupportedFileType(FileType),
}

impl Error {
    /// Takes the integer returned from a C function (-1 is an error, other values are treated as
    /// success) and converts it to a result.
    pub fn from_int(result: i32) -> Result<i32> {
        if result == -1 {
            let error = std::io::Error::last_os_error();
            Err(error.into())
        } else {
            Ok(result)
        }
    }

    /// Takes the integer returned from a C function (-1 is an error, other values are treated as
    /// success) and converts it to a result.
    pub fn from_size(result: isize) -> Result<isize> {
        if result == -1 {
            let error = std::io::Error::last_os_error();
            Err(error.into())
        } else {
            Ok(result)
        }
    }

    /// Takes the pointer returned from a C function (null is an error, other values are treated as
    /// success) and converts it to a result.
    pub fn from_ptr<T>(result: *mut T) -> Result<NonNull<T>> {
        match NonNull::new(result) {
            Some(ptr) => Ok(ptr),
            None => {
                let error = std::io::Error::last_os_error();
                Err(error.into())
            }
        }
    }

    /// Runs a closure that may set errno during its evaluation. Converts the output to a result
    /// based on whether errno is set. Where possible, use from_int or from_ptr instead, so that
    /// errors can be caught where they occur rather than after running a function.
    pub fn with_errno<T>(f: impl FnOnce() -> T) -> Result<T> {
        let old_errno = errno::errno();
        scopeguard::defer! {
            errno::set_errno(old_errno);
        }

        errno::set_errno(Errno(0));
        let output = f();
        match errno::errno() {
            Errno(0) => Ok(output),
            Errno(e) => {
                let error = std::io::Error::from_raw_os_error(e);
                Err(error.into())
            }
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
