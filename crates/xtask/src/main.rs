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

/// Today's date in the local time zone, as `YYYY-MM-DD`.
fn today() -> String {
    jiff::Zoned::now().date().to_string()
}
