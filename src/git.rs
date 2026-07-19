//! Git baseline access — resolving the `base_ref` to a commit and diffing
//! commits for promotion.
//!
//! Hank's shared base graph is built at a **baseline commit** (`base_ref`,
//! default `main`, §5.5/FR-13), and promotion (§7.5) diffs a committed change
//! against that base. This module is the single boundary to git.
//!
//! **Access decision (open question 2).** Hank *shells out* to the system `git`,
//! matching Bobbin's own `index/git.rs` precedent (stack coherence,
//! `CLAUDE.md`), adding no dependency and keeping the single-binary portability
//! story (§6.4). The choice is deliberately reversible: everything git-shaped
//! lives behind this module, so swapping to `gix`/`git2` later is localized.
//! Every call **degrades gracefully** — outside a repo, or with `git` absent, a
//! resolver returns `None` and a diff returns empty; nothing crashes (§6.4).

use std::path::{Path, PathBuf};
use std::process::Command;

/// Run `git` in `root` with `args`, returning stdout on a clean exit (status 0)
/// and `None` on any failure (git missing, not a repo, bad ref, …).
fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// Whether `root` is inside a git work tree.
#[must_use]
pub fn is_repo(root: &Path) -> bool {
    git(root, &["rev-parse", "--is-inside-work-tree"]).is_some_and(|s| s.trim() == "true")
}

/// Resolve a ref (branch, tag, `HEAD`, SHA-ish) to its full commit SHA, or
/// `None` if it does not resolve (or this is not a repo). The `^{commit}`
/// peel ensures tags resolve to the commit they point at.
#[must_use]
pub fn resolve_commit(root: &Path, reference: &str) -> Option<String> {
    let spec = format!("{reference}^{{commit}}");
    git(root, &["rev-parse", "--verify", "--quiet", &spec]).and_then(|s| {
        let sha = s.trim().to_string();
        (!sha.is_empty()).then_some(sha)
    })
}

/// The full SHA of `HEAD`, or `None` outside a repo / on an unborn branch.
#[must_use]
pub fn head_commit(root: &Path) -> Option<String> {
    resolve_commit(root, "HEAD")
}

/// The paths changed between two commit-ish refs (`from..to`), relative to the
/// repository root. Empty when either ref does not resolve, when there is no
/// diff, or outside a repo — the promotion path treats an empty set as
/// "nothing to promote" rather than an error (§7.5).
#[must_use]
pub fn changed_paths(root: &Path, from: &str, to: &str) -> Vec<PathBuf> {
    let range = format!("{from}..{to}");
    let Some(out) = git(root, &["diff", "--name-only", &range]) else {
        return Vec::new();
    };
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// The tracked file paths present in the tree at `reference`, relative to the
/// repository root. Empty when the ref does not resolve or outside a repo — the
/// caller treats an empty tree as "nothing to build" rather than an error.
#[must_use]
pub fn list_files_at(root: &Path, reference: &str) -> Vec<PathBuf> {
    let spec = format!("{reference}^{{tree}}");
    let Some(out) = git(root, &["ls-tree", "-r", "--name-only", "-z", &spec]) else {
        return Vec::new();
    };
    // `-z` gives NUL-separated paths so filenames with newlines are safe.
    out.split('\0')
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// The content of `path` in the tree at `reference`, or `None` when the blob is
/// absent at that ref, is not valid UTF-8 (binary), or this is not a repo.
#[must_use]
pub fn read_blob_at(root: &Path, reference: &str, path: &Path) -> Option<String> {
    let spec = format!("{reference}:{}", path.display());
    git(root, &["show", &spec])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// Initialize a throwaway git repo in `dir` with one committed file.
    /// Returns `false` (skip the test) if `git` is unavailable — integration
    /// with an external toolchain must skip gracefully, not fail (§13).
    fn init_repo(dir: &Path) -> bool {
        let run = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .is_ok_and(|o| o.status.success())
        };
        if !run(&["init", "-q"]) {
            return false; // git absent → skip
        }
        run(&["config", "user.email", "t@t.test"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "first"])
    }

    #[test]
    fn resolves_head_and_detects_repo() {
        let dir = tempfile::tempdir().unwrap();
        if !init_repo(dir.path()) {
            return; // skip: no git
        }
        assert!(is_repo(dir.path()));
        let head = head_commit(dir.path()).expect("HEAD resolves");
        assert_eq!(head.len(), 40, "full SHA");
        // A bogus ref does not resolve.
        assert!(resolve_commit(dir.path(), "no-such-ref").is_none());
    }

    #[test]
    fn diffs_changed_paths_between_commits() {
        let dir = tempfile::tempdir().unwrap();
        if !init_repo(dir.path()) {
            return; // skip: no git
        }
        let first = head_commit(dir.path()).unwrap();
        std::fs::write(dir.path().join("b.txt"), "two\n").unwrap();
        let run = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .unwrap();
        };
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "second"]);
        let second = head_commit(dir.path()).unwrap();

        let changed = changed_paths(dir.path(), &first, &second);
        assert_eq!(changed, vec![PathBuf::from("b.txt")]);
    }

    #[test]
    fn degrades_gracefully_outside_repo() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_repo(dir.path()));
        assert!(head_commit(dir.path()).is_none());
        assert!(resolve_commit(dir.path(), "main").is_none());
        assert!(changed_paths(dir.path(), "a", "b").is_empty());
        assert!(list_files_at(dir.path(), "HEAD").is_empty());
        assert!(read_blob_at(dir.path(), "HEAD", Path::new("a.txt")).is_none());
    }

    #[test]
    fn reads_tree_content_at_a_ref() {
        let dir = tempfile::tempdir().unwrap();
        if !init_repo(dir.path()) {
            return; // skip: no git
        }
        let first = head_commit(dir.path()).unwrap();

        // Second commit: change a.txt and add b.txt.
        std::fs::write(dir.path().join("a.txt"), "changed\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "two\n").unwrap();
        let run = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .unwrap();
        };
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "second"]);

        // The tree at the first commit has only a.txt, with its original body.
        let files = list_files_at(dir.path(), &first);
        assert_eq!(files, vec![PathBuf::from("a.txt")]);
        assert_eq!(
            read_blob_at(dir.path(), &first, Path::new("a.txt")).as_deref(),
            Some("one\n"),
            "reads the historical blob, not the working tree"
        );
        // b.txt did not exist at the first commit.
        assert!(read_blob_at(dir.path(), &first, Path::new("b.txt")).is_none());

        // HEAD sees both files and the updated content.
        let head_files = list_files_at(dir.path(), "HEAD");
        assert!(head_files.contains(&PathBuf::from("a.txt")));
        assert!(head_files.contains(&PathBuf::from("b.txt")));
        assert_eq!(
            read_blob_at(dir.path(), "HEAD", Path::new("a.txt")).as_deref(),
            Some("changed\n")
        );
    }
}
