//! Crate versions, as far as releasing needs them.

use std::{
    cmp::Ordering,
    fmt,
    str::FromStr,
};

use anyhow::{
    Error,
    Result,
    bail,
};

/// A version such as `0.1.0` or `1.0.0-rc.1`. Build metadata (`+...`) is not supported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Version {
    major: u64,
    minor: u64,
    patch: u64,
    pre: Option<String>,
}

impl FromStr for Version {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        let (numbers, pre) = match text.split_once('-') {
            Some((numbers, pre)) => (numbers, Some(pre)),
            None => (text, None),
        };
        let parts: Vec<&str> = numbers.split('.').collect();
        let [major, minor, patch] = parts.as_slice() else {
            bail!("{text:?} is not a version such as 0.1.0");
        };
        let number = |part: &str| -> Result<u64> {
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                bail!("{text:?} is not a version such as 0.1.0");
            }
            Ok(part.parse()?)
        };
        if let Some(pre) = pre {
            let valid = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-';
            if pre.is_empty() || !pre.bytes().all(valid) {
                bail!("{text:?} has an invalid pre-release label");
            }
        }
        Ok(Version {
            major: number(major)?,
            minor: number(minor)?,
            patch: number(patch)?,
            pre: pre.map(str::to_owned),
        })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(pre) = &self.pre {
            write!(f, "-{pre}")?;
        }
        Ok(())
    }
}

impl Version {
    /// Orders versions, with a pre-release before its release.
    ///
    /// Pre-release labels are compared as text, so `rc.10` sorts before `rc.2`.
    pub(crate) fn cmp_release(&self, other: &Version) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }

    /// Reads a version as PyPI writes it, where `1.0.0-rc.1` is `1.0.0rc1`.
    ///
    /// Only the forms that the release workflow produces are accepted: a plain version, or one
    /// followed by `a`, `b` or `rc` and a number.
    pub(crate) fn from_pep440(text: &str) -> Result<Self> {
        let split = text
            .find(|c: char| c.is_ascii_alphabetic())
            .unwrap_or(text.len());
        let (numbers, rest) = text.split_at(split);
        if rest.is_empty() {
            return numbers.parse();
        }
        let digits = rest.trim_start_matches(|c: char| c.is_ascii_alphabetic());
        let label = match rest.get(..rest.len() - digits.len()) {
            Some("a") => "alpha",
            Some("b") => "beta",
            Some("rc") => "rc",
            _ => bail!("{text:?} is not a version this workflow publishes to PyPI"),
        };
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            bail!("{text:?} is not a version this workflow publishes to PyPI");
        }
        format!("{numbers}-{label}.{digits}").parse()
    }

    /// Whether a dependency written as `version = "<requirement>"` accepts this version.
    ///
    /// Cargo reads such a requirement as `^<requirement>`. It accepts newer versions up to the
    /// next breaking one: the next major version from 1.0, the next minor version below 1.0, and
    /// only the same version below 0.1. A pre-release is accepted only by a requirement on that
    /// same pre-release.
    pub(crate) fn satisfies_caret(&self, requirement: &Version) -> bool {
        if self.pre.is_some() {
            return self == requirement;
        }
        if self.cmp_release(requirement) == Ordering::Less {
            return false;
        }
        if requirement.major > 0 {
            self.major == requirement.major
        } else if requirement.minor > 0 {
            self.major == 0 && self.minor == requirement.minor
        } else {
            self.major == 0 && self.minor == 0 && self.patch == requirement.patch
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use std::cmp::Ordering;

    use super::Version;

    fn v(text: &str) -> Version {
        text.parse().unwrap()
    }

    #[test]
    fn parses_and_prints_versions() {
        assert_eq!(v("0.1.0").to_string(), "0.1.0");
        assert_eq!(v("1.2.3-rc.1").to_string(), "1.2.3-rc.1");
        for bad in [
            "0.1",
            "0.1.0.0",
            "v0.1.0",
            "0.1.x",
            "0.1.0-",
            "0.1.0+build",
            "",
            "0..1",
        ] {
            assert!(bad.parse::<Version>().is_err(), "{bad} parsed");
        }
    }

    #[test]
    fn reads_pypi_versions() {
        assert_eq!(Version::from_pep440("0.1.0").unwrap(), v("0.1.0"));
        assert_eq!(Version::from_pep440("1.0.0rc1").unwrap(), v("1.0.0-rc.1"));
        assert_eq!(Version::from_pep440("1.0.0a2").unwrap(), v("1.0.0-alpha.2"));
        assert_eq!(Version::from_pep440("1.0.0b0").unwrap(), v("1.0.0-beta.0"));
        for bad in [
            "1.0.0.post1",
            "1.0.0.dev1",
            "1.0.0rc",
            "1!1.0.0",
            "1.0.0+local",
            "1.0",
        ] {
            assert!(Version::from_pep440(bad).is_err(), "{bad} parsed");
        }
    }

    #[test]
    fn a_pre_release_comes_before_its_release() {
        assert_eq!(v("0.2.0").cmp_release(&v("0.1.9")), Ordering::Greater);
        assert_eq!(v("1.0.0-rc.1").cmp_release(&v("1.0.0")), Ordering::Less);
        assert_eq!(v("1.0.0").cmp_release(&v("1.0.0-rc.1")), Ordering::Greater);
        assert_eq!(v("0.1.0").cmp_release(&v("0.1.0")), Ordering::Equal);
    }

    #[test]
    fn caret_requirements_follow_cargo() {
        assert!(v("0.1.5").satisfies_caret(&v("0.1.0")));
        assert!(!v("0.2.0").satisfies_caret(&v("0.1.0")));
        assert!(!v("0.1.0").satisfies_caret(&v("0.1.1")));
        assert!(v("1.4.0").satisfies_caret(&v("1.0.0")));
        assert!(!v("2.0.0").satisfies_caret(&v("1.0.0")));
        assert!(!v("0.0.2").satisfies_caret(&v("0.0.1")));
        assert!(!v("0.2.0-rc.1").satisfies_caret(&v("0.1.0")));
        assert!(v("0.2.0-rc.1").satisfies_caret(&v("0.2.0-rc.1")));
    }
}
