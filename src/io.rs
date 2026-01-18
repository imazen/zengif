//! I/O abstractions for no_std compatibility.
//!
//! This module provides I/O types that work in both std and no_std environments
//! using the `embedded-io` traits.

use core::fmt;

// Re-export embedded-io traits as our standard I/O interface
pub use embedded_io::{Error as IoError, ErrorKind, ErrorType, Read, Write};

/// I/O error type for zengif.
///
/// In no_std, this wraps an `ErrorKind`. With std, it can also wrap `std::io::Error`.
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    #[cfg(feature = "std")]
    source: Option<std::io::Error>,
}

impl Error {
    /// Create an error from an `ErrorKind`.
    #[must_use]
    pub const fn new(kind: ErrorKind) -> Self {
        Self {
            kind,
            #[cfg(feature = "std")]
            source: None,
        }
    }

    /// Create an "interrupted" error (used for cancellation).
    #[must_use]
    pub const fn interrupted() -> Self {
        Self::new(ErrorKind::Other) // ErrorKind doesn't have Interrupted, use Other
    }

    /// Get the error kind.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "I/O error: {:?}", self.kind)
    }
}

impl embedded_io::Error for Error {
    fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl core::error::Error for Error {
    #[cfg(feature = "std")]
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e as &(dyn core::error::Error + 'static))
    }
}

#[cfg(feature = "std")]
impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        let kind = match err.kind() {
            std::io::ErrorKind::NotFound => ErrorKind::Other,
            std::io::ErrorKind::PermissionDenied => ErrorKind::Other,
            std::io::ErrorKind::ConnectionRefused => ErrorKind::Other,
            std::io::ErrorKind::ConnectionReset => ErrorKind::Other,
            std::io::ErrorKind::ConnectionAborted => ErrorKind::Other,
            std::io::ErrorKind::NotConnected => ErrorKind::Other,
            std::io::ErrorKind::AddrInUse => ErrorKind::Other,
            std::io::ErrorKind::AddrNotAvailable => ErrorKind::Other,
            std::io::ErrorKind::BrokenPipe => ErrorKind::Other,
            std::io::ErrorKind::AlreadyExists => ErrorKind::Other,
            std::io::ErrorKind::WouldBlock => ErrorKind::Other,
            std::io::ErrorKind::InvalidInput => ErrorKind::InvalidInput,
            std::io::ErrorKind::InvalidData => ErrorKind::InvalidData,
            std::io::ErrorKind::TimedOut => ErrorKind::Other,
            std::io::ErrorKind::WriteZero => ErrorKind::WriteZero,
            std::io::ErrorKind::Interrupted => ErrorKind::Other,
            std::io::ErrorKind::UnexpectedEof => ErrorKind::Other,
            std::io::ErrorKind::OutOfMemory => ErrorKind::OutOfMemory,
            _ => ErrorKind::Other,
        };
        Self {
            kind,
            source: Some(err),
        }
    }
}

/// A `Cursor` wraps an in-memory buffer and provides `Read` and `Write`.
///
/// This is a no_std-compatible equivalent of `std::io::Cursor`.
pub struct Cursor<T> {
    inner: T,
    pos: u64,
}

impl<T> Cursor<T> {
    /// Create a new cursor wrapping the provided buffer.
    #[must_use]
    pub const fn new(inner: T) -> Self {
        Self { inner, pos: 0 }
    }

    /// Get the current position.
    #[must_use]
    pub const fn position(&self) -> u64 {
        self.pos
    }

    /// Set the position.
    pub fn set_position(&mut self, pos: u64) {
        self.pos = pos;
    }

    /// Get a reference to the inner buffer.
    #[must_use]
    pub const fn get_ref(&self) -> &T {
        &self.inner
    }

    /// Get a mutable reference to the inner buffer.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Consume the cursor and return the inner buffer.
    #[must_use]
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T> ErrorType for Cursor<T> {
    type Error = Error;
}

impl<T: AsRef<[u8]>> Read for Cursor<T> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let slice = self.inner.as_ref();
        let pos = self.pos as usize;

        if pos >= slice.len() {
            return Ok(0);
        }

        let remaining = &slice[pos..];
        let amt = core::cmp::min(buf.len(), remaining.len());
        buf[..amt].copy_from_slice(&remaining[..amt]);
        self.pos += amt as u64;
        Ok(amt)
    }
}

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
impl Write for Cursor<alloc::vec::Vec<u8>> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        let pos = self.pos as usize;
        let len = self.inner.len();

        // Extend if necessary
        if pos + buf.len() > len {
            self.inner.resize(pos + buf.len(), 0);
        }

        self.inner[pos..pos + buf.len()].copy_from_slice(buf);
        self.pos += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Chain two readers together.
///
/// This is a no_std-compatible equivalent of `std::io::Chain`.
pub struct Chain<A, B> {
    first: A,
    second: B,
    first_done: bool,
}

impl<A, B> Chain<A, B> {
    /// Create a new chain of two readers.
    #[must_use]
    pub const fn new(first: A, second: B) -> Self {
        Self {
            first,
            second,
            first_done: false,
        }
    }
}

impl<A: ErrorType, B: ErrorType<Error = A::Error>> ErrorType for Chain<A, B> {
    type Error = A::Error;
}

impl<A: Read, B: Read<Error = A::Error>> Read for Chain<A, B> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if !self.first_done {
            match self.first.read(buf)? {
                0 => self.first_done = true,
                n => return Ok(n),
            }
        }
        self.second.read(buf)
    }
}

/// Extension trait to chain readers.
pub trait ReadExt: Read + Sized {
    /// Chain this reader with another.
    fn chain<R: Read<Error = Self::Error>>(self, other: R) -> Chain<Self, R> {
        Chain::new(self, other)
    }
}

impl<T: Read> ReadExt for T {}

/// Standard library I/O interoperability.
///
/// Provides wrappers to use `std::io` types with `embedded_io` traits and vice versa.
#[cfg(feature = "std")]
pub mod std_io {
    use super::*;

    /// Wrapper to use `std::io::Read` as `embedded_io::Read`.
    pub struct FromStd<T>(pub T);

    impl<T> ErrorType for FromStd<T> {
        type Error = Error;
    }

    impl<T: std::io::Read> Read for FromStd<T> {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            self.0.read(buf).map_err(Error::from)
        }
    }

    impl<T: std::io::Write> Write for FromStd<T> {
        fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            self.0.write(buf).map_err(Error::from)
        }

        fn flush(&mut self) -> Result<(), Self::Error> {
            self.0.flush().map_err(Error::from)
        }
    }

    /// Wrapper to use `embedded_io::Read` as `std::io::Read`.
    pub struct ToStd<T>(pub T);

    impl<T: Read> std::io::Read for ToStd<T> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(buf).map_err(|e| {
                std::io::Error::new(
                    match e.kind() {
                        ErrorKind::InvalidData => std::io::ErrorKind::InvalidData,
                        ErrorKind::InvalidInput => std::io::ErrorKind::InvalidInput,
                        ErrorKind::WriteZero => std::io::ErrorKind::WriteZero,
                        ErrorKind::OutOfMemory => std::io::ErrorKind::OutOfMemory,
                        _ => std::io::ErrorKind::Other,
                    },
                    "embedded-io error",
                )
            })
        }
    }

    impl<T: Write> std::io::Write for ToStd<T> {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.write(buf).map_err(|e| {
                std::io::Error::new(
                    match e.kind() {
                        ErrorKind::InvalidData => std::io::ErrorKind::InvalidData,
                        ErrorKind::InvalidInput => std::io::ErrorKind::InvalidInput,
                        ErrorKind::WriteZero => std::io::ErrorKind::WriteZero,
                        ErrorKind::OutOfMemory => std::io::ErrorKind::OutOfMemory,
                        _ => std::io::ErrorKind::Other,
                    },
                    "embedded-io error",
                )
            })
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.0.flush().map_err(|e| {
                std::io::Error::new(
                    match e.kind() {
                        ErrorKind::InvalidData => std::io::ErrorKind::InvalidData,
                        ErrorKind::InvalidInput => std::io::ErrorKind::InvalidInput,
                        ErrorKind::WriteZero => std::io::ErrorKind::WriteZero,
                        ErrorKind::OutOfMemory => std::io::ErrorKind::OutOfMemory,
                        _ => std::io::ErrorKind::Other,
                    },
                    "embedded-io error",
                )
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_read() {
        let data = [1u8, 2, 3, 4, 5];
        let mut cursor = Cursor::new(&data[..]);
        let mut buf = [0u8; 3];

        assert_eq!(cursor.read(&mut buf).unwrap(), 3);
        assert_eq!(&buf, &[1, 2, 3]);

        assert_eq!(cursor.read(&mut buf).unwrap(), 2);
        assert_eq!(&buf[..2], &[4, 5]);

        assert_eq!(cursor.read(&mut buf).unwrap(), 0);
    }

    #[test]
    fn chain_read() {
        let first = [1u8, 2, 3];
        let second = [4u8, 5, 6];
        let mut chain = Cursor::new(&first[..]).chain(Cursor::new(&second[..]));
        let mut buf = [0u8; 10];

        let n = chain.read(&mut buf).unwrap();
        assert_eq!(n, 3);
        assert_eq!(&buf[..3], &[1, 2, 3]);

        let n = chain.read(&mut buf).unwrap();
        assert_eq!(n, 3);
        assert_eq!(&buf[..3], &[4, 5, 6]);

        assert_eq!(chain.read(&mut buf).unwrap(), 0);
    }
}
