//! Opening the build's input files.
//!
//! [`SourceBytes`] either memory-maps a file or reads it into memory. If another process
//! truncates a mapped file, the next read of the missing part raises SIGBUS and kills the
//! process. No error value can report that. Reading into memory is therefore the default, and
//! mapping must be asked for with `InputTrust::Trusted`.

use crate::{
    config::InputTrust,
    error::{
        BuildError,
        Result,
    },
};
use memmap2::Mmap;
use std::{
    fs::File,
    ops::Deref,
    path::Path,
};

/// A file's bytes, either memory-mapped or owned.
///
/// Dereferences to `[u8]`, so both kinds work as a byte slice.
#[derive(Debug)]
pub struct SourceBytes(Repr);

#[derive(Debug)]
enum Repr {
    Mapped(Mmap),
    Owned(Box<[u8]>),
}

impl SourceBytes {
    /// Opens `path`, mapped or read depending on `trust`, and refuses a file larger than
    /// `max_bytes`.
    ///
    /// The size is checked from the file's metadata before the file is mapped or read. `what`
    /// names the kind of file in the error.
    pub fn open(
        path: &Path,
        trust: InputTrust,
        max_bytes: u64,
        what: &'static str,
    ) -> Result<Self> {
        let file = File::open(path).map_err(|source| BuildError::Io {
            op: "opening",
            path: path.to_path_buf(),
            source,
        })?;

        let len = file
            .metadata()
            .map_err(|source| BuildError::Io {
                op: "measuring",
                path: path.to_path_buf(),
                source,
            })?
            .len();

        if len > max_bytes {
            return Err(BuildError::InputTooLarge {
                what,
                path: path.to_path_buf(),
                bytes: len,
                max: max_bytes,
            });
        }

        match trust {
            InputTrust::Trusted => {
                // SAFETY: The caller passed `InputTrust::Trusted`, which promises that nothing
                // else writes to this file while it is mapped.
                let map = unsafe { Mmap::map(&file) }.map_err(|source| BuildError::Io {
                    op: "mmapping",
                    path: path.to_path_buf(),
                    source,
                })?;

                Ok(SourceBytes(Repr::Mapped(map)))
            }
            InputTrust::Untrusted => {
                let bytes = std::fs::read(path).map_err(|source| BuildError::Io {
                    op: "reading",
                    path: path.to_path_buf(),
                    source,
                })?;

                Ok(SourceBytes(Repr::Owned(bytes.into_boxed_slice())))
            }
        }
    }

    /// Memory-maps `path` whatever the configured trust, still refusing a file larger than
    /// `max_bytes`.
    ///
    /// This is for files that are safe to map because of where they are, such as a job
    /// directory that the client who uploaded them can no longer write to.
    pub fn open_mapped(path: &Path, max_bytes: u64, what: &'static str) -> Result<Self> {
        SourceBytes::open(path, InputTrust::Trusted, max_bytes, what)
    }

    /// Whether these bytes are mapped rather than owned.
    #[cfg(test)]
    #[must_use]
    pub fn is_mapped(&self) -> bool {
        matches!(self.0, Repr::Mapped(_))
    }
}

impl Deref for SourceBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match &self.0 {
            Repr::Mapped(map) => map,
            Repr::Owned(bytes) => bytes,
        }
    }
}

impl AsRef<[u8]> for SourceBytes {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn file_of(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        let mut f = File::create(&path).expect("creating a fixture file");
        f.write_all(bytes).expect("writing a fixture file");
        path
    }

    #[test]
    fn trusted_maps_and_untrusted_owns() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = file_of(dir.path(), "input.bin", b"twelve bytes");

        let mapped = SourceBytes::open(&path, InputTrust::Trusted, 1024, "test")
            .expect("mapping a small file");
        assert!(mapped.is_mapped());
        assert_eq!(&*mapped, b"twelve bytes");

        let owned = SourceBytes::open(&path, InputTrust::Untrusted, 1024, "test")
            .expect("reading a small file");
        assert!(!owned.is_mapped());
        assert_eq!(&*owned, b"twelve bytes");
    }

    #[test]
    fn a_file_over_the_cap_is_refused_before_it_is_read() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = file_of(dir.path(), "big.bin", &[0_u8; 4096]);

        for trust in [InputTrust::Trusted, InputTrust::Untrusted] {
            let err = SourceBytes::open(&path, trust, 1024, "test pack")
                .expect_err("4096 bytes is over a 1024 byte cap");

            match err {
                BuildError::InputTooLarge {
                    what, bytes, max, ..
                } => {
                    assert_eq!((what, bytes, max), ("test pack", 4096, 1024));
                }
                other => panic!("expected InputTooLarge, got {other:?}"),
            }
        }
    }

    #[test]
    fn an_empty_file_reads_as_no_bytes() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = file_of(dir.path(), "empty.bin", b"");

        let owned = SourceBytes::open(&path, InputTrust::Untrusted, 1024, "test")
            .expect("an empty file is not an error");
        assert!(owned.is_empty());
    }
}
