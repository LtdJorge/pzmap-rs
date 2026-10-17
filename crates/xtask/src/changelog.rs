//! Reads and updates the crates' CHANGELOG.md files, which follow Keep a Changelog.
//!
//! Each version has a heading such as `## [0.1.0] - 2026-10-17`, and the file ends with link
//! definitions such as `[0.1.0]: https://...` for those headings.

use anyhow::{
    Result,
    bail,
};

use crate::{
    REPO,
    version::Version,
};

const UNRELEASED: &str = "Unreleased";

/// Returns the text under the heading for `name`, such as `0.1.0` or `Unreleased`, up to the
/// next heading or the link definitions. Returns `None` if there is no such heading.
pub(crate) fn section<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let heading = format!("## [{name}]");
    let mut offset = 0;
    let mut start = None;
    for line in text.split_inclusive('\n') {
        let end = offset + line.len();
        let trimmed = line.trim_end();
        match start {
            None if is_heading(trimmed, &heading) => start = Some(end),
            Some(start) if trimmed.starts_with("## [") || is_link_definition(trimmed) => {
                return Some(&text[start..offset]);
            }
            _ => {}
        }
        offset = end;
    }
    start.map(|start| &text[start..])
}

/// Whether the section for `name` lists anything.
pub(crate) fn has_entries(text: &str, name: &str) -> bool {
    section(text, name).is_some_and(|body| !body.trim().is_empty())
}

/// Whether the file ends with a link definition for `name`, such as `[0.1.0]: https://...`.
pub(crate) fn has_link(text: &str, name: &str) -> bool {
    let prefix = format!("[{name}]: ");
    text.lines().any(|line| line.starts_with(&prefix))
}

/// Counts the list items under the heading for `name`.
pub(crate) fn count_entries(text: &str, name: &str) -> usize {
    section(text, name).map_or(0, |body| {
        body.lines()
            .filter(|line| line.trim_start().starts_with("- "))
            .count()
    })
}

/// Moves the `[Unreleased]` entries under a new heading for `version`, dated `date`, and
/// updates the links at the end of the file.
///
/// `previous` is the last released version, if any. The new version's link compares against
/// it, and the first release links to its tag instead.
pub(crate) fn release(
    text: &str,
    krate: &str,
    version: &Version,
    previous: Option<&Version>,
    date: &str,
) -> Result<String> {
    if !has_entries(text, UNRELEASED) {
        bail!("nothing is listed under [Unreleased]; add the changes to this release first");
    }
    if section(text, &version.to_string()).is_some() {
        bail!("the changelog already has a section for {version}");
    }

    let lines: Vec<&str> = text.lines().collect();
    let unreleased = format!("## [{UNRELEASED}]");
    let Some(heading) = lines
        .iter()
        .position(|line| is_heading(line.trim_end(), &unreleased))
    else {
        bail!("the changelog has no {unreleased} heading");
    };
    // The new heading goes after the blank lines that follow [Unreleased], so the entries end
    // up under it and [Unreleased] is left empty.
    let mut entries = heading + 1;
    while lines
        .get(entries)
        .is_some_and(|line| line.trim().is_empty())
    {
        entries += 1;
    }

    let tag = |version: &Version| format!("{krate}-v{version}");
    let unreleased_link = format!(
        "[{UNRELEASED}]: https://github.com/{REPO}/compare/{}...master",
        tag(version)
    );
    let version_link = match previous {
        Some(previous) => format!(
            "[{version}]: https://github.com/{REPO}/compare/{}...{}",
            tag(previous),
            tag(version)
        ),
        None => format!(
            "[{version}]: https://github.com/{REPO}/releases/tag/{}",
            tag(version)
        ),
    };

    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 4);
    out.extend(
        lines
            .iter()
            .take(heading + 1)
            .map(|line| (*line).to_owned()),
    );
    out.push(String::new());
    out.push(format!("## [{version}] - {date}"));
    out.push(String::new());
    let mut linked = false;
    for line in lines.iter().skip(entries) {
        if line.starts_with(&format!("[{UNRELEASED}]: ")) {
            out.push(unreleased_link.clone());
            out.push(version_link.clone());
            linked = true;
        } else {
            out.push((*line).to_owned());
        }
    }
    if !linked {
        while out.last().is_some_and(|line| line.trim().is_empty()) {
            out.pop();
        }
        out.push(String::new());
        out.push(unreleased_link);
        out.push(version_link);
    }

    let mut text = out.join("\n");
    text.push('\n');
    Ok(text)
}

/// Whether `line` is `heading`, alone or followed by a date such as ` - 2026-10-17`.
fn is_heading(line: &str, heading: &str) -> bool {
    line.strip_prefix(heading)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
}

fn is_link_definition(line: &str) -> bool {
    line.starts_with('[') && line.contains("]: ")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    const FIRST: &str = "# Changelog

Intro.

## [Unreleased]

The first public release.

### Added

- Parsers.
- Bindings.

[Unreleased]: https://github.com/LtdJorge/pzmap-rs/commits/master/crates/libpzmap
";

    fn v(text: &str) -> Version {
        text.parse().unwrap()
    }

    #[test]
    fn reads_a_section_up_to_the_links() {
        let body = section(FIRST, "Unreleased").unwrap();
        assert!(body.contains("- Bindings.\n"));
        assert!(!body.contains("[Unreleased]:"));
        assert_eq!(count_entries(FIRST, "Unreleased"), 2);
        assert!(section(FIRST, "0.1.0").is_none());
    }

    #[test]
    fn the_first_release_links_to_its_tag() {
        let out = release(FIRST, "libpzmap", &v("0.1.0"), None, "2026-10-17").unwrap();
        assert_eq!(
            out,
            "# Changelog

Intro.

## [Unreleased]

## [0.1.0] - 2026-10-17

The first public release.

### Added

- Parsers.
- Bindings.

[Unreleased]: https://github.com/LtdJorge/pzmap-rs/compare/libpzmap-v0.1.0...master
[0.1.0]: https://github.com/LtdJorge/pzmap-rs/releases/tag/libpzmap-v0.1.0
"
        );
        assert!(!has_entries(&out, "Unreleased"));
        assert!(has_link(&out, "0.1.0"));
        assert!(!has_link(FIRST, "0.1.0"));
        assert_eq!(count_entries(&out, "0.1.0"), 2);
    }

    #[test]
    fn a_later_release_compares_with_the_previous_one() {
        let first = release(FIRST, "libpzmap", &v("0.1.0"), None, "2026-10-17").unwrap();
        let next = first.replace(
            "## [Unreleased]\n\n## [0.1.0]",
            "## [Unreleased]\n\n### Fixed\n\n- A crash.\n\n## [0.1.0]",
        );
        let out = release(
            &next,
            "libpzmap",
            &v("0.1.1"),
            Some(&v("0.1.0")),
            "2026-11-02",
        )
        .unwrap();
        assert!(out.contains(
            "## [Unreleased]\n\n## [0.1.1] - 2026-11-02\n\n### Fixed\n\n- A crash.\n\n## [0.1.0]"
        ));
        assert!(out.ends_with(
            "[Unreleased]: https://github.com/LtdJorge/pzmap-rs/compare/libpzmap-v0.1.1...master
[0.1.1]: https://github.com/LtdJorge/pzmap-rs/compare/libpzmap-v0.1.0...libpzmap-v0.1.1
[0.1.0]: https://github.com/LtdJorge/pzmap-rs/releases/tag/libpzmap-v0.1.0
"
        ));
    }

    #[test]
    fn refuses_an_empty_or_repeated_release() {
        let first = release(FIRST, "libpzmap", &v("0.1.0"), None, "2026-10-17").unwrap();
        assert!(release(&first, "libpzmap", &v("0.1.1"), None, "2026-10-17").is_err());
        let again = first.replace("## [Unreleased]\n", "## [Unreleased]\n\n- More.\n");
        assert!(release(&again, "libpzmap", &v("0.1.0"), None, "2026-10-17").is_err());
    }
}
