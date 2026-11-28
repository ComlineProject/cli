//! `comline store` — status, migrate, verify, prune and reset, and what
//! `build` and `release` do with a store they can't use as it is.
//!
//! The rule under test: nothing is deleted or changed without the user
//! running the command that does it. These tests have no terminal, so the
//! answers to a question on one (the offer to migrate, the reset
//! confirmation) are covered where the questions are asked, in
//! `src/commands/store.rs`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use predicates::prelude::*;

use crate::util::*;

fn built_project(temp: &Path) -> PathBuf {
    let project = fixture_project(temp);
    comline_cmd()
        .current_dir(&project)
        .arg("build")
        .assert()
        .success();
    project
}

fn stamp(project: &Path) -> PathBuf {
    project.join(".comline/FORMAT_VERSION")
}

/// The IR format the store was built in, which is the one this comline writes.
fn current_format(project: &Path) -> String {
    fs::read_to_string(stamp(project))
        .unwrap()
        .trim()
        .to_string()
}

fn backup(project: &Path) -> PathBuf {
    project.join(format!(".comline.format-{}.bak", current_format(project)))
}

/// Every file under the project's `.comline*` entries (the store, a backup, a
/// migration's directory), by path.
fn snapshot(project: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, into: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, into);
            } else {
                let name = path.strip_prefix(root).unwrap().display().to_string();
                into.insert(name, fs::read(&path).unwrap());
            }
        }
    }
    let mut into = BTreeMap::new();
    for entry in fs::read_dir(project).unwrap() {
        let path = entry.unwrap().path();
        let is_store = path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with(".comline"));
        if is_store && path.is_dir() {
            walk(project, &path, &mut into);
        }
    }
    into
}

/// A store that has lost its stamp: the way to make one `migrate` has work
/// on.
fn unstamped_project(temp: &Path) -> PathBuf {
    let project = built_project(temp);
    fs::remove_file(stamp(&project)).unwrap();
    project
}

#[test]
fn status_lists_the_store_and_changes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let project = built_project(temp.path());
    let format = current_format(&project);
    let before = snapshot(&project);

    comline_cmd()
        .current_dir(&project)
        .args(["store", "status"])
        .assert()
        .success()
        .stderr(predicate::str::contains(format!(
            "IR format {format} (this comline writes format {format})"
        )))
        .stderr(predicate::str::contains(
            "the store is in the current format",
        ))
        .stderr(predicate::str::contains("1 version(s)"))
        .stderr(predicate::str::contains("0.1.0-dev.1"));

    assert_eq!(snapshot(&project), before);
}

#[test]
fn status_of_a_project_never_built_says_so() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["store", "status"])
        .assert()
        .success()
        .stderr(predicate::str::contains("no version history yet"));
    assert!(!project.join(".comline").exists());
}

#[test]
fn status_of_a_store_it_cant_use_names_the_command_and_exits_zero() {
    let temp = tempfile::tempdir().unwrap();
    let project = built_project(temp.path());
    fs::write(stamp(&project), "1").unwrap();
    let before = snapshot(&project);

    comline_cmd()
        .current_dir(&project)
        .args(["store", "status"])
        .assert()
        .success()
        .stderr(predicate::str::contains("IR format 1"))
        .stderr(predicate::str::contains("`comline store reset`"));
    assert_eq!(snapshot(&project), before);
}

#[test]
fn migrate_on_a_current_store_does_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let project = built_project(temp.path());
    let before = snapshot(&project);

    comline_cmd()
        .current_dir(&project)
        .args(["store", "migrate"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Nothing to migrate"));

    assert_eq!(snapshot(&project), before);
}

#[test]
fn check_exits_zero_when_there_is_nothing_to_migrate() {
    let temp = tempfile::tempdir().unwrap();
    let project = built_project(temp.path());
    let before = snapshot(&project);

    comline_cmd()
        .current_dir(&project)
        .args(["store", "migrate", "--check"])
        .assert()
        .code(0)
        .stderr(predicate::str::contains("Nothing to migrate"));
    // No store at all is nothing to migrate too.
    let empty = tempfile::tempdir().unwrap();
    let fresh = fixture_project(empty.path());
    comline_cmd()
        .current_dir(&fresh)
        .args(["store", "migrate", "--check"])
        .assert()
        .code(0);

    assert_eq!(snapshot(&project), before);
    assert!(!fresh.join(".comline").exists());
}

#[test]
fn check_exits_three_when_the_migration_would_be_refused() {
    let temp = tempfile::tempdir().unwrap();
    let project = built_project(temp.path());
    fs::write(stamp(&project), "1").unwrap();
    let before = snapshot(&project);

    comline_cmd()
        .current_dir(&project)
        .args(["store", "migrate", "--check"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("would be refused"))
        .stderr(predicate::str::contains("IR format 1"))
        .stderr(predicate::str::contains("`comline store reset`"));

    // Nothing written: not the lock, not a directory.
    assert_eq!(snapshot(&project), before);
    assert!(!project.join(".comline.migrating").exists());
}

#[test]
fn check_exits_two_when_a_migration_would_run_and_writes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let project = unstamped_project(temp.path());
    let before = snapshot(&project);

    // Without a format named, the store is refused: its format is unknown.
    comline_cmd()
        .current_dir(&project)
        .args(["store", "migrate", "--check"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("--assume-format"));

    let current = current_format_of_unstamped(&project);
    comline_cmd()
        .current_dir(&project)
        .args(["store", "migrate", "--check", "--assume-format", &current])
        .assert()
        .code(2)
        .stdout(predicate::str::contains("(assumed)"))
        .stdout(predicate::str::contains("nothing deleted"));

    assert_eq!(snapshot(&project), before);
    assert!(!project.join(".comline.migrating").exists());
    assert!(!stamp(&project).exists());
}

/// The format a store without its stamp would be assumed to be: the one this
/// comline writes, which `status` reports as the target.
fn current_format_of_unstamped(project: &Path) -> String {
    let output = comline_cmd()
        .current_dir(project)
        .args(["--plain", "store", "status"])
        .assert()
        .success()
        .get_output()
        .clone();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    stderr
        .split("this comline writes format ")
        .nth(1)
        .and_then(|rest| rest.split(')').next())
        .unwrap_or_else(|| panic!("no target format in {stderr}"))
        .to_string()
}

#[test]
fn migrate_stamps_an_unstamped_store_keeps_the_old_one_and_maps_the_hashes() {
    let temp = tempfile::tempdir().unwrap();
    let project = unstamped_project(temp.path());
    let current = current_format_of_unstamped(&project);
    let map = temp.path().join("hashes.txt");

    // Without a format named, the store is refused and nothing changes.
    let before = snapshot(&project);
    comline_cmd()
        .current_dir(&project)
        .args(["store", "migrate"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("--assume-format"));
    assert_eq!(snapshot(&project), before);

    comline_cmd()
        .current_dir(&project)
        .args(["store", "migrate", "--assume-format", &current, "--map"])
        .arg(&map)
        .assert()
        .success()
        .stderr(predicate::str::contains("Migrated the store"))
        .stderr(predicate::str::contains(format!(
            ".comline.format-{current}.bak"
        )));

    assert_eq!(current_format(&project), current);
    let kept = backup(&project);
    assert!(kept.is_dir(), "the old store is kept");
    assert!(kept.join("refs/heads/main").exists());
    assert!(
        !project.join(".comline.migrating").exists(),
        "the lock is gone"
    );

    // The same bytes, so the same hashes: one `<old> <new>` line per commit.
    let map = fs::read_to_string(&map).unwrap();
    assert_eq!(map.lines().count(), 1, "{map}");
    for line in map.lines() {
        let (old, new) = line.split_once(' ').unwrap();
        assert_eq!(old, new);
        assert_eq!(old.len(), 64, "{line}");
    }

    // The migrated store is a store: verified, listed with its backup, built
    // on; and the backup stays until the user prunes it.
    comline_cmd()
        .current_dir(&project)
        .args(["store", "verify"])
        .assert()
        .success()
        .stderr(predicate::str::contains("The store is sound"));
    comline_cmd()
        .current_dir(&project)
        .args(["store", "status"])
        .assert()
        .success()
        .stderr(predicate::str::contains("backups, kept until"))
        .stderr(predicate::str::contains(format!(
            ".comline.format-{current}.bak"
        )));
    comline_cmd()
        .current_dir(&project)
        .arg("build")
        .assert()
        .success();
    assert!(kept.is_dir());

    // Migrating again is a no-op.
    let after = snapshot(&project);
    comline_cmd()
        .current_dir(&project)
        .args(["store", "migrate"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Nothing to migrate"));
    assert_eq!(snapshot(&project), after);

    comline_cmd()
        .current_dir(&project)
        .args(["store", "prune"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Removed 1 backup(s)"));
    assert!(!kept.exists());
    assert!(project.join(".comline/refs/heads/main").exists());
}

#[test]
fn check_writes_nothing_so_it_takes_neither_restart_nor_map() {
    let temp = tempfile::tempdir().unwrap();
    let project = built_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["store", "migrate", "--check", "--restart"])
        .assert()
        .failure()
        .code(2);
    comline_cmd()
        .current_dir(&project)
        .args(["store", "migrate", "--check", "--map", "hashes.txt"])
        .assert()
        .failure()
        .code(2);
    assert!(!project.join("hashes.txt").exists());
}

#[test]
fn verify_passes_a_sound_store_and_names_the_file_of_a_damaged_one() {
    let temp = tempfile::tempdir().unwrap();
    let project = built_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["store", "verify"])
        .assert()
        .success()
        .stderr(predicate::str::contains("1 version(s)"));

    let victim = fs::read_dir(project.join(".comline/objects"))
        .unwrap()
        .flat_map(|dir| fs::read_dir(dir.unwrap().path()).unwrap())
        .map(|file| file.unwrap().path())
        .next()
        .unwrap();
    fs::remove_file(&victim).unwrap();
    let before = snapshot(&project);

    comline_cmd()
        .current_dir(&project)
        .args(["store", "verify"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(
            victim.file_name().unwrap().to_string_lossy().to_string(),
        ));
    assert_eq!(snapshot(&project), before);
}

#[test]
fn verify_refuses_a_store_in_another_format() {
    let temp = tempfile::tempdir().unwrap();
    let project = built_project(temp.path());
    fs::write(stamp(&project), "1").unwrap();

    comline_cmd()
        .current_dir(&project)
        .args(["store", "verify"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("IR format 1"));
}

#[test]
fn prune_with_nothing_to_remove_says_so() {
    let temp = tempfile::tempdir().unwrap();
    let project = built_project(temp.path());
    let before = snapshot(&project);

    comline_cmd()
        .current_dir(&project)
        .args(["store", "prune"])
        .assert()
        .success()
        .stderr(predicate::str::contains("No backups to remove"));
    assert_eq!(snapshot(&project), before);
}

#[test]
fn reset_without_a_terminal_needs_force_and_changes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let project = built_project(temp.path());
    fs::write(stamp(&project), "1").unwrap();
    let before = snapshot(&project);

    comline_cmd()
        .current_dir(&project)
        .args(["store", "reset"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("without confirmation"))
        .stderr(predicate::str::contains("--force"));

    assert_eq!(snapshot(&project), before);
}

#[test]
fn reset_force_moves_the_store_aside_deletes_nothing_and_keeps_the_cache() {
    let temp = tempfile::tempdir().unwrap();
    let project = built_project(temp.path());
    let format = current_format(&project);
    let cache = project.join(".comline/deps-cache/dep/file.txt");
    fs::create_dir_all(cache.parent().unwrap()).unwrap();
    fs::write(&cache, "fetched").unwrap();
    fs::write(stamp(&project), "1").unwrap();
    let before: BTreeMap<String, Vec<u8>> = snapshot(&project)
        .into_iter()
        .filter(|(path, _)| !path.contains("deps-cache"))
        .collect();

    comline_cmd()
        .current_dir(&project)
        .args(["store", "reset", "--force"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Moved the store aside"))
        .stderr(predicate::str::contains("nothing was deleted"));

    // Every file of the old store is in the backup, byte for byte.
    let kept = project.join(".comline.format-1.bak");
    let after = snapshot(&project);
    for (path, bytes) in &before {
        let moved = path.replacen(".comline/", ".comline.format-1.bak/", 1);
        assert_eq!(after.get(&moved), Some(bytes), "{path}");
    }
    assert!(kept.is_dir());
    // The cache is not the store: it stays where it was.
    assert_eq!(fs::read_to_string(&cache).unwrap(), "fetched");
    assert!(!kept.join("deps-cache").exists());
    assert!(!stamp(&project).exists());
    assert!(!project.join(".comline.resetting").exists());

    // The next build starts a new history, and the backup is untouched by it.
    comline_cmd()
        .current_dir(&project)
        .arg("build")
        .assert()
        .success()
        .stderr(predicate::str::contains("initial build 0.1.0-dev.1"));
    assert_eq!(current_format(&project), format);
    for (path, bytes) in &before {
        let moved = path.replacen(".comline/", ".comline.format-1.bak/", 1);
        assert_eq!(snapshot(&project).get(&moved), Some(bytes), "{path}");
    }
    comline_cmd()
        .current_dir(&project)
        .args(["store", "prune"])
        .assert()
        .success();
    assert!(!kept.exists());
}

#[test]
fn prune_refuses_while_nothing_has_replaced_the_backup() {
    let temp = tempfile::tempdir().unwrap();
    let project = built_project(temp.path());
    comline_cmd()
        .current_dir(&project)
        .args(["store", "reset", "--force"])
        .assert()
        .success();
    let before = snapshot(&project);

    comline_cmd()
        .current_dir(&project)
        .args(["store", "prune"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("build first"));

    assert_eq!(snapshot(&project), before);
}

#[test]
fn a_store_from_a_newer_comline_is_never_reset() {
    let temp = tempfile::tempdir().unwrap();
    let project = built_project(temp.path());
    fs::write(stamp(&project), "999").unwrap();
    let before = snapshot(&project);

    comline_cmd()
        .current_dir(&project)
        .args(["store", "reset", "--force"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("never reset or touched"));

    assert_eq!(snapshot(&project), before);
}

#[test]
fn reset_with_no_history_has_nothing_to_do() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["store", "reset", "--force"])
        .assert()
        .failure()
        .code(2);
    assert!(!project.join(".comline").exists());
}

/// A migration that was interrupted stops every command that would use the
/// store, naming the command that finishes it, and nothing is done for the
/// user: with no terminal there is no question either.
#[test]
fn build_and_release_stop_on_an_interrupted_migration_and_change_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let project = built_project(temp.path());
    fs::create_dir(project.join(".comline.migrating")).unwrap();
    let before = snapshot(&project);

    for args in [&["build"][..], &["release", "--yes"][..]] {
        comline_cmd()
            .current_dir(&project)
            .args(args)
            .assert()
            .failure()
            .code(2)
            .stderr(predicate::str::contains("was interrupted"))
            .stderr(predicate::str::contains("`comline store migrate`"));
    }

    assert_eq!(snapshot(&project), before);
    assert!(project.join(".comline.migrating").is_dir());
}

#[test]
fn an_interrupted_reset_names_reset_and_a_build_never_resets() {
    let temp = tempfile::tempdir().unwrap();
    let project = built_project(temp.path());
    fs::create_dir(project.join(".comline.resetting")).unwrap();
    let before = snapshot(&project);

    comline_cmd()
        .current_dir(&project)
        .arg("build")
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("`comline store reset`"));

    assert_eq!(snapshot(&project), before);
}

/// A refusal that has no migration to offer names `comline store reset` in
/// its help, and no command (a build included) resets on its own.
#[test]
fn a_store_with_no_path_names_reset_and_a_build_does_not_reset() {
    let temp = tempfile::tempdir().unwrap();
    let project = built_project(temp.path());
    fs::write(stamp(&project), "1").unwrap();
    let before = snapshot(&project);

    let output = comline_cmd()
        .current_dir(&project)
        .args(["--plain", "build"])
        .assert()
        .failure()
        .code(2)
        .get_output()
        .clone();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("`comline store reset`"), "{stderr}");
    let (_, help) = stderr
        .split_once("\nhelp: ")
        .unwrap_or_else(|| panic!("no help line in {stderr}"));
    assert!(help.contains("`comline store migrate`"), "{stderr}");
    assert!(help.contains("`comline store reset`"), "{stderr}");

    assert_eq!(snapshot(&project), before);
}
