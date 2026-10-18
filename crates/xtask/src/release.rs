//! Releases the crates published to crates.io, PyPI and Maven Central.
//!
//! A release is a tag named after the crate and its version, such as `libpzmap-v0.1.0`.
//! Pushing the tag runs `.github/workflows/release.yml`, which publishes the crate and creates
//! the GitHub Release. These commands check each step before it happens and print the next one.

use std::{
    cmp::Ordering,
    fs,
    io::{
        self,
        IsTerminal,
        Write,
    },
    path::{
        Path,
        PathBuf,
    },
    process::Command,
};

use anyhow::{
    Context,
    Result,
    anyhow,
    bail,
};

use crate::{
    REPO,
    Tools,
    capture,
    changelog,
    notices::notices,
    read_text,
    run,
    succeeds,
    today,
    version::Version,
};

/// Where a crate is published.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Registry {
    CratesIo,
    /// PyPI, under the given project name.
    PyPi(&'static str),
    /// Maven Central, under the given group and artifact.
    Maven(&'static str, &'static str),
}

impl Registry {
    fn name(self) -> &'static str {
        match self {
            Registry::CratesIo => "crates.io",
            Registry::PyPi(_) => "PyPI",
            Registry::Maven(..) => "Maven Central",
        }
    }
}

/// The crates that `release.yml` publishes, and where. Its tag patterns list the same crates.
const RELEASED: [(&str, Registry); 5] = [
    ("libpzmap", Registry::CratesIo),
    ("pzmap2img", Registry::CratesIo),
    ("pzmap-tools", Registry::CratesIo),
    ("pzmap-python", Registry::PyPi("pzmap")),
    ("pzmap-java", Registry::Maven("io.github.ltdjorge", "pzmap")),
];

/// The GitHub environment that the crates.io trusted publishers are set up for.
const ENVIRONMENT: &str = "crates-io";

/// A workspace crate, from `cargo metadata`.
struct Package {
    name: String,
    version: Version,
    manifest: PathBuf,
    dependencies: Vec<Dependency>,
}

/// A dependency of a workspace crate on another one.
struct Dependency {
    name: String,
    /// The requirement as cargo reports it, such as `^0.1.0`.
    requirement: String,
}

impl Package {
    /// The crate's directory, relative to the workspace root, such as `crates/libpzmap`.
    fn dir(&self, root: &Path) -> Result<PathBuf> {
        let dir = self
            .manifest
            .parent()
            .ok_or_else(|| anyhow!("{} has no parent", self.manifest.display()))?;
        Ok(dir.strip_prefix(root).unwrap_or(dir).to_path_buf())
    }

    fn changelog(&self) -> Result<PathBuf> {
        let dir = self
            .manifest
            .parent()
            .ok_or_else(|| anyhow!("{} has no parent", self.manifest.display()))?;
        Ok(dir.join("CHANGELOG.md"))
    }

    fn tag(&self) -> String {
        tag_name(&self.name, &self.version)
    }
}

fn tag_name(krate: &str, version: &Version) -> String {
    format!("{krate}-v{version}")
}

impl Tools {
    fn git(&self, args: &[&str]) -> Result<String> {
        capture(self.command("git").args(args))
    }

    fn gh(&self, args: &[&str]) -> Result<String> {
        capture(self.command("gh").args(args).args(["--repo", REPO])).map_err(|error| {
            if gh_installed() {
                error
            } else {
                anyhow!(
                    "this step needs the GitHub CLI; install it from https://cli.github.com and \
                     run `gh auth login`"
                )
            }
        })
    }

    /// The workspace crates, without their registry dependencies.
    fn packages(&self) -> Result<Vec<Package>> {
        let output =
            capture(
                self.cargo()
                    .args(["metadata", "--no-deps", "--format-version", "1"]),
            )?;
        let json: serde_json::Value =
            serde_json::from_str(&output).context("cannot parse cargo metadata")?;
        let text = |value: &serde_json::Value, key: &str| -> Result<String> {
            value
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| anyhow!("cargo metadata has no {key}"))
        };
        let packages = json
            .get("packages")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| anyhow!("cargo metadata has no packages"))?;
        packages
            .iter()
            .map(|package| {
                let dependencies = package
                    .get("dependencies")
                    .and_then(serde_json::Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                    .iter()
                    // Dependencies on other workspace crates are the ones with a path.
                    .filter(|dependency| dependency.get("path").is_some())
                    .map(|dependency| {
                        Ok(Dependency {
                            name: text(dependency, "name")?,
                            requirement: text(dependency, "req")?,
                        })
                    })
                    .collect::<Result<_>>()?;
                Ok(Package {
                    name: text(package, "name")?,
                    version: text(package, "version")?.parse()?,
                    manifest: text(package, "manifest_path")?.into(),
                    dependencies,
                })
            })
            .collect()
    }

    fn package(&self, krate: &str) -> Result<Package> {
        registry(krate)?;
        self.packages()?
            .into_iter()
            .find(|package| package.name == krate)
            .ok_or_else(|| anyhow!("the workspace has no crate named {krate}"))
    }

    /// The newest version of `krate` where it is published, or `None` if it was never published.
    fn published_version(&self, krate: &str) -> Result<Option<Version>> {
        match registry(krate)? {
            Registry::CratesIo => self.crates_io_version(krate),
            Registry::PyPi(project) => self.pypi_version(project),
            Registry::Maven(group, artifact) => self.maven_version(group, artifact),
        }
    }

    /// Asks `url` with curl and returns the HTTP status and the body.
    fn http_get(&self, url: &str) -> Result<(String, String)> {
        // `--write-out` adds the HTTP status on a line of its own after the body.
        let output = capture(self.command("curl").args([
            "--silent",
            "--show-error",
            "--user-agent",
            "pzmap-rs xtask (github.com/LtdJorge/pzmap-rs)",
            "--write-out",
            "\n%{http_code}",
            url,
        ]))
        .with_context(|| format!("cannot reach {url}"))?;
        let (body, status) = output.rsplit_once('\n').unwrap_or(("", &output));
        Ok((status.to_owned(), body.to_owned()))
    }

    /// The newest release of `group:artifact` on Maven Central, or `None` if there is none.
    ///
    /// A version appears here up to 30 minutes after it is published in the Central Portal.
    fn maven_version(&self, group: &str, artifact: &str) -> Result<Option<Version>> {
        let url = format!(
            "https://repo1.maven.org/maven2/{}/{artifact}/maven-metadata.xml",
            group.replace('.', "/")
        );
        let (status, body) = self.http_get(&url)?;
        match status.as_str() {
            "404" => return Ok(None),
            "200" => {}
            _ => bail!("Maven Central answered {status} for {group}:{artifact}"),
        }
        let release = body
            .split_once("<release>")
            .and_then(|(_, rest)| rest.split_once("</release>"))
            .map(|(release, _)| release.trim())
            .ok_or_else(|| anyhow!("the Maven Central metadata for {artifact} has no release"))?;
        Ok(Some(release.parse()?))
    }

    /// The newest version of `project` on PyPI, or `None` if it was never published.
    fn pypi_version(&self, project: &str) -> Result<Option<Version>> {
        let (status, body) = self.http_get(&format!("https://pypi.org/pypi/{project}/json"))?;
        let body = body.as_str();
        match status.as_str() {
            "404" => return Ok(None),
            "200" => {}
            _ => bail!("PyPI answered {status} for {project}"),
        }
        let json: serde_json::Value =
            serde_json::from_str(body).context("cannot parse PyPI's answer")?;
        let version = json
            .pointer("/info/version")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow!("PyPI's answer for {project} has no version"))?;
        Ok(Some(Version::from_pep440(version)?))
    }

    /// The newest version of `krate` on crates.io, or `None` if it was never published.
    fn crates_io_version(&self, krate: &str) -> Result<Option<Version>> {
        let output = capture(self.cargo().args(["search", krate, "--limit", "20"]))
            .context("cannot ask crates.io for the published versions")?;
        let prefix = format!("{krate} = \"");
        for line in output.lines() {
            if let Some(rest) = line.strip_prefix(&prefix) {
                let version = rest.split('"').next().unwrap_or_default();
                return Ok(Some(version.parse()?));
            }
        }
        Ok(None)
    }

    fn tag_exists_locally(&self, tag: &str) -> Result<bool> {
        Ok(!self.git(&["tag", "--list", tag])?.is_empty())
    }

    fn tag_exists_on_github(&self, tag: &str) -> Result<bool> {
        let refname = format!("refs/tags/{tag}");
        Ok(!self
            .git(&["ls-remote", "--tags", "origin", &refname])?
            .is_empty())
    }

    /// Fails unless this checkout is on master with no local changes, and is not behind GitHub.
    fn require_clean_master(&self) -> Result<()> {
        let branch = self.git(&["rev-parse", "--abbrev-ref", "HEAD"])?;
        if branch != "master" {
            bail!("releases are made from master, but this checkout is on {branch}");
        }
        let changes = self.git(&["status", "--porcelain"])?;
        if !changes.is_empty() {
            bail!("there are uncommitted changes; commit or stash them first:\n{changes}");
        }
        self.git(&["fetch", "--quiet", "--tags", "origin", "master"])?;
        let behind = self.git(&["rev-list", "--count", "HEAD..origin/master"])?;
        if behind != "0" {
            bail!("master is {behind} commits behind GitHub; run `git pull` first");
        }
        Ok(())
    }

    /// Fails unless this commit is on GitHub's master and CI passed for it. Returns the commit.
    fn require_pushed_and_green(&self) -> Result<String> {
        if !succeeds(self.command("git").args([
            "merge-base",
            "--is-ancestor",
            "HEAD",
            "origin/master",
        ]))? {
            bail!(
                "this commit is not on GitHub yet; run `git push origin master`, wait for CI, \
                 then run this again"
            );
        }
        let commit = self.git(&["rev-parse", "HEAD"])?;
        let runs = self.gh(&[
            "run",
            "list",
            "--workflow",
            "ci.yml",
            "--event",
            "push",
            "--commit",
            &commit,
            "--limit",
            "1",
            "--json",
            "status,conclusion,url,databaseId",
        ])?;
        let runs: serde_json::Value =
            serde_json::from_str(&runs).context("cannot parse the CI runs")?;
        let Some(run) = runs.as_array().and_then(|runs| runs.first()) else {
            bail!("CI has not run for {commit} yet; wait for it to start, then run this again");
        };
        let field = |key: &str| run.get(key).and_then(serde_json::Value::as_str);
        let url = field("url").unwrap_or_default();
        match (field("status"), field("conclusion")) {
            (Some("completed"), Some("success")) => Ok(commit),
            (Some("completed"), Some(conclusion)) => {
                let id = run.get("databaseId").map(ToString::to_string);
                let id = id.as_deref().unwrap_or("<run id>");
                bail!(
                    "CI ended with {conclusion} for this commit: {url}\nFix it, or if it was \
                     cancelled, run it again with `gh run rerun {id}`"
                )
            }
            _ => bail!("CI is still running for this commit; wait for it to pass: {url}"),
        }
    }
}

/// Fails unless the crate's changelog has a section and a link for its current version.
fn require_changelog_release(package: &Package) -> Result<()> {
    let path = package.changelog()?;
    let text = read_text(&path)?;
    let version = package.version.to_string();
    if !changelog::has_entries(&text, &version) {
        bail!(
            "{} has no section for {version}; run `just release-prepare {} {version}` first",
            path.display(),
            package.name
        );
    }
    if !changelog::has_link(&text, &version) {
        bail!(
            "{} has no link line for {version} at its end, such as \
             `[{version}]: https://github.com/{REPO}/releases/tag/{}`",
            path.display(),
            package.tag()
        );
    }
    Ok(())
}

/// Whether the GitHub CLI can be run.
fn gh_installed() -> bool {
    succeeds(Command::new("gh").arg("--version")).unwrap_or(false)
}

/// Where `krate` is published. Fails for a crate that the release workflow does not publish.
fn registry(krate: &str) -> Result<Registry> {
    match RELEASED.iter().find(|(name, _)| *name == krate) {
        Some((_, registry)) => Ok(*registry),
        None => {
            let names: Vec<&str> = RELEASED.iter().map(|(name, _)| *name).collect();
            bail!(
                "{krate} is not released by the release workflow; the crates are {}",
                names.join(", ")
            )
        }
    }
}

/// Asks a yes or no question, with no as the answer when there is no terminal to ask.
fn confirm(question: &str) -> Result<bool> {
    if !io::stdin().is_terminal() {
        println!("{question} [y/N] no, because there is no terminal to answer in");
        return Ok(false);
    }
    print!("{question} [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

/// Shows where each crate's release stands, and what to do next.
pub(crate) fn status(krate: Option<&str>) -> Result<()> {
    let tools = Tools::new()?;
    if let Some(krate) = krate {
        registry(krate)?;
    }
    if tools
        .git(&["fetch", "--quiet", "--tags", "origin"])
        .is_err()
    {
        println!("Cannot reach GitHub, so tags pushed from elsewhere may be missing.\n");
    }
    let packages = tools.packages()?;
    let mut first = true;
    for package in packages
        .iter()
        .filter(|package| registry(&package.name).is_ok())
        .filter(|package| krate.is_none_or(|krate| package.name == krate))
    {
        if !first {
            println!();
        }
        first = false;
        print_status(&tools, package)?;
    }
    Ok(())
}

fn print_status(tools: &Tools, package: &Package) -> Result<()> {
    let name = &package.name;
    let pattern = format!("{name}-v*");
    let tags = tools.git(&["tag", "--list", &pattern, "--sort=-v:refname"])?;
    let last_tag = tags.lines().next();
    let tagged = tags.lines().any(|tag| tag == package.tag());
    let registry = registry(name)?;
    let published = tools.published_version(name)?;
    let release = match last_tag {
        Some(tag) => match tools.gh(&["release", "view", tag, "--json", "url"]) {
            Ok(_) => "yes",
            Err(_) if gh_installed() => "no",
            Err(_) => "unknown, because gh is not installed",
        },
        None => "no",
    };
    let dir = package.dir(&tools.root)?;
    let dir_text = dir.to_string_lossy().replace('\\', "/");
    let since = match last_tag {
        Some(tag) => {
            let range = format!("{tag}..HEAD");
            Some(tools.git(&["rev-list", "--count", &range, "--", &dir_text])?)
        }
        None => None,
    };
    let changelog_text = read_text(&package.changelog()?)?;
    let entries = changelog::count_entries(&changelog_text, "Unreleased");

    println!("{name}");
    println!("  version in Cargo.toml   {}", package.version);
    println!("  latest tag              {}", last_tag.unwrap_or("none"));
    let published_text = published.as_ref().map(ToString::to_string);
    println!(
        "  {:<24}{}",
        format!("on {}", registry.name()),
        published_text.as_deref().unwrap_or("not published")
    );
    println!("  GitHub Release          {release}");
    if let Some(since) = &since {
        println!("  commits since the tag   {since} touch {dir_text}");
    }
    println!("  under [Unreleased]      {entries} entries");

    let next = if !tagged {
        match &published {
            None if !changelog::has_entries(&changelog_text, &package.version.to_string()) => {
                format!(
                    "first release: just release-prepare {name} {}, then follow its steps",
                    package.version
                )
            }
            None if registry == Registry::CratesIo => {
                format!("first release, published by hand: just release-first {name}")
            }
            None => format!(
                "first release, which the release workflow publishes: just release-tag {name}"
            ),
            Some(published) => match published.cmp_release(&package.version) {
                Ordering::Equal => format!(
                    "{} is on {} but not tagged: just release-tag {name}",
                    package.version,
                    registry.name()
                ),
                Ordering::Less => format!(
                    "{} is prepared: push master, wait for CI, then just release-tag {name}",
                    package.version
                ),
                Ordering::Greater => format!(
                    "{} has {published}, newer than Cargo.toml; check that master is up to date",
                    registry.name()
                ),
            },
        }
    } else if published
        .as_ref()
        .is_none_or(|published| published.cmp_release(&package.version) == Ordering::Less)
        || release == "no"
    {
        unfinished_release(tools, package)?
    } else if entries > 0 || since.as_deref().is_some_and(|since| since != "0") {
        format!("when ready to release the changes: just release-prepare {name} <new version>")
    } else {
        String::from("nothing to release")
    };
    let mut lines = next.lines();
    println!(
        "  next                    {}",
        lines.next().unwrap_or_default()
    );
    for line in lines {
        println!("                          {line}");
    }
    Ok(())
}

/// Explains why the release for a crate's tag is not complete.
fn unfinished_release(tools: &Tools, package: &Package) -> Result<String> {
    let tag = package.tag();
    let version = package.version.to_string();
    let changelog = package
        .changelog()?
        .strip_prefix(&tools.root)
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let tagged_changelog = tools
        .git(&["show", &format!("{tag}:{changelog}")])
        .unwrap_or_default();
    if !changelog::has_entries(&tagged_changelog, &version) {
        return Ok(format!(
            "the release workflow cannot run, because {changelog} at {tag} has no section for \
             {version}.\nAdd the section on master, push it and wait for CI. Then move the \
             tag:\n  git push --delete origin {tag}\n  git tag -d {tag}\n  just release-tag {}",
            package.name
        ));
    }
    let runs = tools.gh(&[
        "run",
        "list",
        "--workflow",
        "release.yml",
        "--branch",
        &tag,
        "--limit",
        "1",
        "--json",
        "status,conclusion,url",
    ]);
    let run = runs
        .ok()
        .and_then(|runs| serde_json::from_str::<serde_json::Value>(&runs).ok())
        .and_then(|runs| runs.as_array().and_then(|runs| runs.first().cloned()));
    let field = |key: &str| {
        run.as_ref()
            .and_then(|run| run.get(key))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    let url = field("url")
        .unwrap_or_else(|| format!("https://github.com/{REPO}/actions/workflows/release.yml"));
    let maven = matches!(registry(&package.name)?, Registry::Maven(..));
    Ok(
        match (field("status").as_deref(), field("conclusion").as_deref()) {
            // The workflow only uploads to the Central Portal. Running it again would upload a
            // second deployment.
            (Some("completed"), Some("success")) if maven => String::from(
                "the release workflow uploaded it to the Central Portal. Publish it at\nhttps://central.sonatype.com/publishing/deployments, and Maven Central shows it\nup to 30 minutes later. Do not run the workflow again, which would upload it twice.",
            ),
            (Some("completed"), Some("success")) => {
                format!("the release workflow passed, but the release is incomplete: {url}")
            }
            (Some("completed"), Some(conclusion)) => format!(
                "the release workflow ended with {conclusion}: {url}\nFix the cause, then run \
                 it again from that page"
            ),
            (Some(_), _) => format!("the release workflow is running: {url}"),
            _ => format!("no release workflow run was found for {tag}: {url}"),
        },
    )
}

/// Bumps a crate to `version`, dates its changelog section, and updates what depends on the
/// version: other crates' requirements, Cargo.lock and the notices.
pub(crate) fn prepare(krate: &str, version: &str) -> Result<()> {
    let tools = Tools::new()?;
    let version: Version = version.parse()?;
    let package = tools.package(krate)?;
    tools.require_clean_master()?;

    let tag = tag_name(krate, &version);
    if tools.tag_exists_locally(&tag)? {
        bail!("the tag {tag} already exists");
    }

    // The current version was released if the changelog has a section for it. If it has none,
    // this is the crate's first release, and it can keep its current version.
    let changelog_path = package.changelog()?;
    let changelog_text = read_text(&changelog_path)?;
    let current_released =
        changelog::section(&changelog_text, &package.version.to_string()).is_some();
    match version.cmp_release(&package.version) {
        Ordering::Greater => {}
        Ordering::Equal if !current_released => {}
        _ => bail!(
            "{krate} is at {}; the new version must be newer",
            package.version
        ),
    }
    let previous = current_released.then_some(&package.version);
    let new_changelog = changelog::release(&changelog_text, krate, &version, previous, &today())?;
    let new_manifest = set_package_version(&read_text(&package.manifest)?, &version)
        .with_context(|| format!("cannot update {}", package.manifest.display()))?;

    // A crate that depends on this one with a requirement the new version does not meet would
    // stop building, so its requirement is raised too.
    let mut raised = Vec::new();
    let mut unchanged = Vec::new();
    for dependent in tools.packages()? {
        let Some(dependency) = dependent
            .dependencies
            .iter()
            .find(|dependency| dependency.name == krate)
        else {
            continue;
        };
        let Some(requirement) = dependency.requirement.strip_prefix('^') else {
            bail!(
                "{} requires {krate} {}; only plain requirements such as \"0.1.0\" can be \
                 updated, so change it by hand",
                dependent.name,
                dependency.requirement
            );
        };
        let requirement: Version = requirement
            .parse()
            .with_context(|| format!("cannot read {}'s requirement on {krate}", dependent.name))?;
        if version.satisfies_caret(&requirement) {
            unchanged.push((dependent.name.clone(), requirement));
        } else {
            let text = read_text(&dependent.manifest)?;
            let text = set_dependency_version(&text, krate, &requirement, &version)
                .with_context(|| format!("cannot update {}", dependent.manifest.display()))?;
            raised.push((dependent, text));
        }
    }

    write(&changelog_path, &new_changelog)?;
    write(&package.manifest, &new_manifest)?;
    for (dependent, text) in &raised {
        write(&dependent.manifest, text)?;
    }
    println!("Updating Cargo.lock");
    run(tools.cargo().args(["update", "--workspace", "--quiet"]))?;
    println!("Updating the third-party notices, which list the crates' versions");
    notices(false)?;

    println!();
    println!(
        "{krate} is now at {version}, and its changelog section is dated {}.",
        today()
    );
    if !raised.is_empty() {
        println!();
        println!("These crates now require {krate} {version}, so they need a release after it:");
        for (dependent, _) in &raised {
            println!("  {}", dependent.name);
        }
        println!("Add a line about it to the [Unreleased] section of their changelogs.");
    }
    if !unchanged.is_empty() {
        println!();
        println!("These crates accept {version} with their current requirement, so they can stay:");
        for (name, requirement) in &unchanged {
            println!("  {name}, which requires {krate} {requirement}");
        }
    }
    Ok(())
}

/// Creates the tag for a crate's current version and, once confirmed, pushes it, which starts
/// the release workflow.
pub(crate) fn tag(krate: &str) -> Result<()> {
    let tools = Tools::new()?;
    let package = tools.package(krate)?;
    let version = &package.version;
    let tag = package.tag();
    tools.require_clean_master()?;

    if tools.tag_exists_locally(&tag)? {
        bail!(
            "the tag {tag} already exists here; if it was never pushed, delete it with \
             `git tag -d {tag}` and run this again"
        );
    }
    if tools.tag_exists_on_github(&tag)? {
        bail!("the tag {tag} is already on GitHub");
    }
    require_changelog_release(&package)?;
    let registry = registry(krate)?;
    let published = tools.published_version(krate)?;
    // crates.io only lets the workflow publish a crate that exists. PyPI's pending publisher
    // lets it publish the first version too.
    if published.is_none() && registry == Registry::CratesIo {
        bail!(
            "{krate} is not on crates.io yet, and the workflow cannot publish a crate the first \
             time; run `just release-first {krate}`"
        );
    }
    let commit = tools.require_pushed_and_green()?;
    let subject = tools.git(&["log", "-1", "--format=%h %s", &commit])?;

    run(tools
        .command("git")
        .args(["tag", "--annotate", &tag, "--message"])
        .arg(format!("{krate} {version}"))
        .arg(&commit))?;
    println!("Tagged {subject} as {tag}.");
    println!();
    let already = published.as_ref() == Some(version);
    if already {
        println!(
            "{krate} {version} is already on {}, so the workflow only creates the GitHub Release.",
            registry.name()
        );
    } else if matches!(registry, Registry::Maven(..)) {
        println!(
            "Pushing the tag uploads {krate} {version} to the Central Portal. It reaches Maven \
             Central once you publish it there, and a published version cannot be replaced."
        );
    } else {
        println!(
            "Pushing the tag publishes {krate} {version} to {}. A published version cannot be \
             replaced, only yanked.",
            registry.name()
        );
    }
    if !confirm(&format!("Push {tag} to GitHub?"))? {
        println!();
        println!("The tag exists only here. Push it later with `git push origin {tag}`,");
        println!("or delete it with `git tag -d {tag}`.");
        return Ok(());
    }
    run(tools.command("git").args(["push", "origin", &tag]))?;
    println!();
    println!("The release workflow is running:");
    println!("  https://github.com/{REPO}/actions/workflows/release.yml");
    println!("When it finishes, the release is at:");
    println!("  https://github.com/{REPO}/releases/tag/{tag}");
    Ok(())
}

/// Publishes a crate to crates.io for the first time, by hand, and explains how to set up its
/// trusted publisher. crates.io only allows a trusted publisher for a crate that exists.
pub(crate) fn first(krate: &str) -> Result<()> {
    let tools = Tools::new()?;
    let package = tools.package(krate)?;
    let version = &package.version;
    if registry(krate)? != Registry::CratesIo {
        bail!(
            "{krate} goes to {}, where the release workflow can publish the first version too; \
             run `just release-tag {krate}`",
            registry(krate)?.name()
        );
    }
    tools.require_clean_master()?;

    if let Some(published) = tools.crates_io_version(krate)? {
        bail!(
            "{krate} is already on crates.io, at {published}; release it with `just release-tag \
             {krate}`"
        );
    }
    require_changelog_release(&package)?;
    for dependency in &package.dependencies {
        if tools.crates_io_version(&dependency.name)?.is_none() {
            bail!(
                "{krate} depends on {}, which is not on crates.io yet; release it first",
                dependency.name
            );
        }
    }
    tools.require_pushed_and_green()?;

    println!("This publishes {krate} {version} to crates.io with the token from `cargo login`.");
    println!("If you have no token yet, create one at https://crates.io/settings/tokens with the");
    println!("publish-new and publish-update scopes, then run `cargo login`.");
    println!();
    if !confirm(&format!("Publish {krate} {version}?"))? {
        return Ok(());
    }
    run(tools.cargo().args(["publish", "--package", krate]))?;

    println!();
    println!("{krate} {version} is on crates.io. Now let the release workflow publish it:");
    println!();
    println!("1. Open https://crates.io/crates/{krate}/settings and add a trusted publisher:");
    let (owner, repository) = REPO.split_once('/').unwrap_or((REPO, ""));
    println!("     Publisher           GitHub");
    println!("     Repository owner    {owner}");
    println!("     Repository name     {repository}");
    println!("     Workflow filename   release.yml");
    println!("     Environment         {ENVIRONMENT}");
    println!("2. Run `just release-tag {krate}`. The workflow sees that {version} is already on");
    println!("   crates.io, so it only creates the GitHub Release.");
    println!();
    println!("Once every crate has a trusted publisher, delete the token at");
    println!("https://crates.io/settings/tokens.");
    Ok(())
}

fn write(path: &Path, text: &str) -> Result<()> {
    fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))
}

/// Sets the `version` line of the `[package]` table in a Cargo.toml.
fn set_package_version(manifest: &str, version: &Version) -> Result<String> {
    let mut table = "";
    let mut done = false;
    let mut out = String::with_capacity(manifest.len());
    for line in manifest.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            table = trimmed;
        }
        if !done && table == "[package]" && trimmed.starts_with("version") {
            if !trimmed.starts_with("version = \"") {
                bail!("its version is not a plain string, so change it by hand");
            }
            out.push_str(&format!("version = \"{version}\"\n"));
            done = true;
        } else {
            out.push_str(line);
        }
    }
    if !done {
        bail!("it has no version in its [package] table");
    }
    Ok(out)
}

/// Changes the version requirement of the `krate` dependency lines in a Cargo.toml from `from`
/// to `to`, keeping the rest of each line.
fn set_dependency_version(
    manifest: &str,
    krate: &str,
    from: &Version,
    to: &Version,
) -> Result<String> {
    let old = format!("version = \"{from}\"");
    let new = format!("version = \"{to}\"");
    let prefix = format!("{krate} = ");
    let mut changed = 0;
    let mut out = String::with_capacity(manifest.len());
    for line in manifest.split_inclusive('\n') {
        if line.trim_start().starts_with(&prefix) && line.contains(&old) {
            out.push_str(&line.replacen(&old, &new, 1));
            changed += 1;
        } else {
            out.push_str(line);
        }
    }
    if changed == 0 {
        bail!("cannot find a `{prefix}` line with `{old}`, so change it by hand");
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        text.parse().unwrap()
    }

    #[test]
    fn sets_only_the_package_version() {
        let manifest = "[package]\nname = \"a\"\nversion = \"0.1.0\"\n\n[dependencies]\nb = { \
                        version = \"0.1.0\" }\n";
        assert_eq!(
            set_package_version(manifest, &v("0.2.0")).unwrap(),
            "[package]\nname = \"a\"\nversion = \"0.2.0\"\n\n[dependencies]\nb = { version = \
             \"0.1.0\" }\n"
        );
        assert!(set_package_version("[package]\nversion.workspace = true\n", &v("0.2.0")).is_err());
        assert!(set_package_version("[package]\nname = \"a\"\n", &v("0.2.0")).is_err());
    }

    #[test]
    fn raises_a_dependency_requirement() {
        let manifest = "[dependencies]\nlibpzmap = { path = \"../libpzmap\", version = \"0.1.0\" \
                        }\nother = { version = \"0.1.0\" }\n";
        assert_eq!(
            set_dependency_version(manifest, "libpzmap", &v("0.1.0"), &v("0.2.0")).unwrap(),
            "[dependencies]\nlibpzmap = { path = \"../libpzmap\", version = \"0.2.0\" }\nother \
             = { version = \"0.1.0\" }\n"
        );
        assert!(set_dependency_version(manifest, "missing", &v("0.1.0"), &v("0.2.0")).is_err());
    }
}
