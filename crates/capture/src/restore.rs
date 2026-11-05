//! `tattle record restore`: rebuild an app folder from a recording's final
//! snapshot. Used to make the frozen starting apps in `corpus/bases/` that
//! edit prompts build on.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::shadow::Shadow;

/// Extract the final snapshot (the last `Stop`, or else the last snapshot)
/// of `recording/snapshots.bundle` into `out`. Returns the ref used.
pub fn restore(recording: &Path, out: &Path) -> Result<String> {
    let bundle = crate::fsx::canonical(&recording.join("snapshots.bundle"))
        .with_context(|| format!("no snapshots.bundle in {}", recording.display()))?;
    if out.exists() && fs::read_dir(out)?.next().is_some() {
        bail!("{} is not empty", out.display());
    }
    fs::create_dir_all(out)?;
    let out = crate::fsx::canonical(out)?;

    let tmp = out
        .parent()
        .context("output folder has no parent")?
        .join(format!(".cv-restore-{}", std::process::id()));
    let result = extract(&bundle, &tmp, &out);
    let _ = crate::fsx::remove_dir_all(&tmp);
    result
}

fn extract(bundle: &Path, tmp: &Path, out: &Path) -> Result<String> {
    let repo = Shadow {
        git_dir: tmp.to_path_buf(),
        index: tmp.join("index"),
        work_tree: tmp.to_path_buf(),
    };
    fs::create_dir_all(tmp)?;
    repo.ensure()?;
    repo.git([
        "fetch".as_ref(),
        "-q".as_ref(),
        bundle.as_os_str(),
        "refs/cv/*:refs/cv/*".as_ref(),
    ])?;
    let refs = repo.git(["for-each-ref", "--sort=refname", "--format=%(refname)", "refs/cv/"])?;
    let refs: Vec<&str> = refs.lines().filter(|r| !r.ends_with("/last")).collect();
    let chosen = refs
        .iter()
        .rev()
        .find(|r| r.ends_with("-Stop"))
        .or(refs.last())
        .context("the bundle has no snapshots")?
        .to_string();

    // Check the snapshot out into `out` with its own index, so no external
    // `tar` is needed (Windows).
    let into = Shadow { git_dir: tmp.to_path_buf(), index: tmp.join("restore.index"), work_tree: out.to_path_buf() };
    into.git(["read-tree", chosen.as_str()]).with_context(|| format!("could not read snapshot {chosen}"))?;
    into.git(["checkout-index", "-a", "-f"]).with_context(|| format!("could not extract snapshot {chosen}"))?;
    Ok(chosen)
}
