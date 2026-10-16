//! Which branch a chat's folder has checked out, read straight from
//! `.git/HEAD` so saving a session never spawns `git`.

use std::path::Path;

/// The branch checked out in `cwd`'s repository (or linked worktree), or
/// `None` outside a repo and on a detached HEAD.
pub fn checked_out_branch(cwd: &Path) -> Option<String> {
    let mut dir = Some(cwd);
    while let Some(at) = dir {
        let dot_git = at.join(".git");
        if dot_git.is_dir() {
            return head_branch(&dot_git);
        }
        if dot_git.is_file() {
            // A linked worktree: `.git` is a file naming its git dir.
            let text = std::fs::read_to_string(&dot_git).ok()?;
            let gitdir = text.strip_prefix("gitdir:")?.trim();
            return head_branch(&at.join(gitdir));
        }
        dir = at.parent();
    }
    None
}

fn head_branch(git_dir: &Path) -> Option<String> {
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let name = head.trim().strip_prefix("ref: refs/heads/")?;
    (!name.is_empty()).then(|| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_branch_from_a_repo_a_subfolder_and_a_linked_worktree() {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(repo.path().join(".git")).unwrap();
        std::fs::write(repo.path().join(".git/HEAD"), "ref: refs/heads/feat/work\n").unwrap();
        std::fs::create_dir_all(repo.path().join("src/deep")).unwrap();
        assert_eq!(
            checked_out_branch(repo.path()).as_deref(),
            Some("feat/work")
        );
        assert_eq!(
            checked_out_branch(&repo.path().join("src/deep")).as_deref(),
            Some("feat/work")
        );

        let gitdir = repo.path().join(".git/worktrees/wt");
        std::fs::create_dir_all(&gitdir).unwrap();
        std::fs::write(gitdir.join("HEAD"), "ref: refs/heads/issue-7-fix\n").unwrap();
        let wt = tempfile::tempdir().unwrap();
        std::fs::write(
            wt.path().join(".git"),
            format!("gitdir: {}\n", gitdir.display()),
        )
        .unwrap();
        assert_eq!(
            checked_out_branch(wt.path()).as_deref(),
            Some("issue-7-fix")
        );
    }

    #[test]
    fn detached_heads_and_plain_folders_have_no_branch() {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(repo.path().join(".git")).unwrap();
        std::fs::write(
            repo.path().join(".git/HEAD"),
            "0123456789abcdef0123456789abcdef01234567\n",
        )
        .unwrap();
        assert_eq!(checked_out_branch(repo.path()), None);
        let plain = tempfile::tempdir().unwrap();
        assert_eq!(checked_out_branch(plain.path()), None);
    }
}
