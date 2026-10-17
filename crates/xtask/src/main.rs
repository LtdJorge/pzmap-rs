//! Maintenance tasks for the workspace, written in Rust so they run the same on every platform.
//!
//! ```text
//! cargo run -p xtask -- notices                 regenerate the THIRD-PARTY-NOTICES.md files
//! cargo run -p xtask -- notices --check         fail if any of them is out of date
//! cargo run -p xtask -- release status [CRATE]  show where each crate's release stands
//! cargo run -p xtask -- release prepare CRATE VERSION
//!                                               bump a crate and date its changelog section
//! cargo run -p xtask -- release tag CRATE       tag the crate's version and push the tag
//! cargo run -p xtask -- release first CRATE     publish a crate to crates.io the first time
//! ```
//!
//! The Justfile at the repository root runs these with shorter names.

mod changelog;
mod notices;
mod release;
mod version;

use std::{
    env,
    ffi::OsString,
    fs,
    path::{
        Path,
        PathBuf,
    },
    process::{
        Command,
        ExitCode,
    },
    time::SystemTime,
};

use anyhow::{
    Context,
    Result,
    anyhow,
    bail,
};

/// The GitHub repository, as `owner/name`.
const REPO: &str = "LtdJorge/pzmap-rs";

const USAGE: &str = "usage:
  cargo run -p xtask -- notices [--check]
  cargo run -p xtask -- release status [CRATE]
  cargo run -p xtask -- release prepare CRATE VERSION
  cargo run -p xtask -- release tag CRATE
  cargo run -p xtask -- release first CRATE";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["notices"] => notices::notices(false),
        ["notices", "--check"] => notices::notices(true),
        ["release", "status"] => release::status(None),
        ["release", "status", krate] => release::status(Some(krate)),
        ["release", "prepare", krate, version] => release::prepare(krate, version),
        ["release", "tag", krate] => release::tag(krate),
        ["release", "first", krate] => release::first(krate),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// The cargo program and the workspace root that its commands run in.
struct Tools {
    cargo: OsString,
    root: PathBuf,
}

impl Tools {
    fn new() -> Result<Self> {
        // `cargo run` tells the program which cargo started it.
        let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .ok_or_else(|| anyhow!("cannot find the workspace root"))?
            .to_path_buf();
        Ok(Self { cargo, root })
    }

    fn cargo(&self) -> Command {
        let mut command = Command::new(&self.cargo);
        command.current_dir(&self.root);
        command
    }

    /// A command that runs `program` in the workspace root.
    fn command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        command.current_dir(&self.root);
        command
    }
}

/// Runs `command` and returns its output, trimmed. Fails with its error output if it fails.
fn capture(command: &mut Command) -> Result<String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let output = command
        .output()
        .with_context(|| format!("cannot run {program}"))?;
    if !output.status.success() {
        bail!(
            "{program} failed:\n{}",
            String::from_utf8_lossy(&output.stderr).trim_end()
        );
    }
    let stdout = String::from_utf8(output.stdout)
        .with_context(|| format!("{program} printed something that is not UTF-8"))?;
    Ok(stdout.trim().to_owned())
}

/// Runs `command` and returns whether it succeeded, hiding its output.
fn succeeds(command: &mut Command) -> Result<bool> {
    let program = command.get_program().to_string_lossy().into_owned();
    let output = command
        .output()
        .with_context(|| format!("cannot run {program}"))?;
    Ok(output.status.success())
}

/// Runs `command` with its output shown, and fails if it fails.
fn run(command: &mut Command) -> Result<()> {
    let program = command.get_program().to_string_lossy().into_owned();
    let status = command
        .status()
        .with_context(|| format!("cannot run {program}"))?;
    if !status.success() {
        bail!("{program} failed");
    }
    Ok(())
}

/// Reads a text file with LF line endings, whatever the checkout gave it.
fn read_text(path: &Path) -> Result<String> {
    let text =
        fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    Ok(text.replace("\r\n", "\n"))
}

/// Today's date in UTC, as `YYYY-MM-DD`.
fn today() -> String {
    let seconds = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let (year, month, day) = civil_date(seconds / 86_400);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Turns a count of days since 1970-01-01 into a year, month and day.
///
/// This is Howard Hinnant's `civil_from_days`, for dates on or after 1970.
fn civil_date(days: u64) -> (u64, u64, u64) {
    let days = days + 719_468;
    let era = days / 146_097;
    let day_of_era = days % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::civil_date;

    #[test]
    fn civil_date_matches_known_days() {
        assert_eq!(civil_date(0), (1970, 1, 1));
        assert_eq!(civil_date(11_016), (2000, 2, 29));
        assert_eq!(civil_date(11_017), (2000, 3, 1));
        assert_eq!(civil_date(20_743), (2026, 10, 17));
        assert_eq!(civil_date(20_818), (2026, 12, 31));
        assert_eq!(civil_date(20_819), (2027, 1, 1));
    }
}
