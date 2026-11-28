//! The questions `comline store reset`, `comline store prune` and `build` ask
//! on a terminal, answered through a pseudo-terminal: with a yes the command
//! does what it asked, with a no it changes nothing. Unix only (see
//! `Cargo.toml`); the answers' meaning is the same code on every platform,
//! and `store.rs` covers what happens with no terminal.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use comline_core::package::build::cas::migrate::{Consent, Engine, MigrateOptions, Point};
use expectrl::process::unix::WaitStatus;
use expectrl::session::OsSession;
use expectrl::{Eof, Expect};

use crate::store::{
    built_project, current_format, current_format_of_unstamped, snapshot, stamp, unstamped_project,
};
use crate::util::*;

/// `comline --plain <args>` in `project`, on a pseudo-terminal: stdin, stdout
/// and stderr are all a terminal, so the command asks.
fn on_a_terminal(project: &Path, args: &[&str]) -> OsSession {
    let mut command = Command::new(env!("CARGO_BIN_EXE_comline"));
    command.current_dir(project).arg("--plain").args(args);
    let mut session = OsSession::spawn(command).unwrap();
    session.set_expect_timeout(Some(Duration::from_secs(60)));
    session
}

/// Wait for the command to end: what it printed since the last `expect`, and
/// its exit code.
fn finish(mut session: OsSession) -> (String, i32) {
    let rest = session.expect(Eof).unwrap();
    let output = String::from_utf8_lossy(rest.as_bytes()).into_owned();
    match session.get_process().wait().unwrap() {
        WaitStatus::Exited(_, code) => (output, code),
        other => panic!("the command didn't exit: {other:?}\n{output}"),
    }
}

/// Wait for the `[y/N]` that ends a question, and return the text before it.
fn question(session: &mut OsSession) -> String {
    let before = session.expect("[y/N]").unwrap();
    String::from_utf8_lossy(before.before()).into_owned()
}

/// A project whose store was reset once and built again, so that it has a
/// backup beside a store that replaced it.
fn project_with_a_backup(temp: &Path) -> PathBuf {
    let project = built_project(temp);
    comline_cmd()
        .current_dir(&project)
        .args(["store", "reset", "--force"])
        .assert()
        .success();
    comline_cmd()
        .current_dir(&project)
        .arg("build")
        .assert()
        .success();
    assert!(
        project.read_dir().unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".bak")),
        "a backup"
    );
    project
}

fn backups_in(project: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = project
        .read_dir()
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_string_lossy().ends_with(".bak"))
        .collect();
    found.sort();
    found
}

#[test]
fn reset_shows_what_it_does_and_a_yes_moves_the_store_aside() {
    let temp = tempfile::tempdir().unwrap();
    let project = built_project(temp.path());
    let format = current_format(&project);
    let before = snapshot(&project);

    let mut session = on_a_terminal(&project, &["store", "reset"]);
    let asked = question(&mut session);
    assert!(asked.contains("Nothing is deleted"), "{asked}");
    assert!(asked.contains(&format!("in IR format {format}")), "{asked}");
    assert!(
        asked.contains(&format!(".comline.format-{format}.bak")),
        "{asked}"
    );
    session.send_line("y").unwrap();
    let (output, code) = finish(session);

    assert_eq!(code, 0, "{output}");
    assert!(output.contains("Moved the store aside"), "{output}");
    assert!(project
        .join(format!(".comline.format-{format}.bak/refs/heads/main"))
        .exists());
    assert!(!project.join(".comline/refs/heads/main").exists());
    // Every file of the old store is in the backup.
    let after = snapshot(&project);
    for (path, bytes) in &before {
        let moved = path.replacen(".comline/", &format!(".comline.format-{format}.bak/"), 1);
        assert_eq!(after.get(&moved), Some(bytes), "{path}");
    }
}

#[test]
fn reset_with_a_no_changes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let project = built_project(temp.path());
    let before = snapshot(&project);

    for answer in ["n", ""] {
        let mut session = on_a_terminal(&project, &["store", "reset"]);
        question(&mut session);
        session.send_line(answer).unwrap();
        let (output, code) = finish(session);

        assert_eq!(code, 0, "{output}");
        assert!(output.contains("Cancelled"), "{output}");
        assert_eq!(snapshot(&project), before);
    }
}

#[test]
fn prune_lists_the_backups_and_a_yes_removes_them() {
    let temp = tempfile::tempdir().unwrap();
    let project = project_with_a_backup(temp.path());
    let found = backups_in(&project);
    assert_eq!(found.len(), 1);
    let name = found[0].file_name().unwrap().to_string_lossy().into_owned();

    let mut session = on_a_terminal(&project, &["store", "prune"]);
    let asked = question(&mut session);
    assert!(asked.contains("would be removed"), "{asked}");
    assert!(asked.contains(&name), "{asked}");
    assert!(asked.contains("1 backup(s)"), "{asked}");
    session.send_line("y").unwrap();
    let (output, code) = finish(session);

    assert_eq!(code, 0, "{output}");
    assert!(output.contains("Removed 1 backup(s)"), "{output}");
    assert!(backups_in(&project).is_empty());
    assert!(project.join(".comline/refs/heads/main").exists());
}

#[test]
fn prune_with_a_no_keeps_the_backups() {
    let temp = tempfile::tempdir().unwrap();
    let project = project_with_a_backup(temp.path());
    let before = snapshot(&project);

    let mut session = on_a_terminal(&project, &["store", "prune"]);
    question(&mut session);
    session.send_line("n").unwrap();
    let (output, code) = finish(session);

    assert_eq!(code, 0, "{output}");
    assert!(output.contains("nothing was removed"), "{output}");
    assert_eq!(backups_in(&project).len(), 1);
    assert_eq!(snapshot(&project), before);
}

#[test]
fn prune_with_no_backups_says_so_and_asks_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let project = built_project(temp.path());

    // No question: the command ends without anything answering.
    let session = on_a_terminal(&project, &["store", "prune"]);
    let (output, code) = finish(session);

    assert_eq!(code, 0, "{output}");
    assert!(output.contains("No backups to remove"), "{output}");
    assert!(!output.contains("[y/N]"), "{output}");
}

/// A migration that stopped right after it claimed `.comline.migrating/`, as
/// a crash leaves it: the store is unstamped (its format named with
/// `--assume-format`), so core has something to migrate, and a command that
/// finds it stops and offers `comline store migrate`.
fn interrupted_migration(temp: &Path) -> PathBuf {
    let project = unstamped_project(temp);
    let format: u32 = current_format_of_unstamped(&project).parse().unwrap();
    let mut engine = Engine::current().on_point(|point| match point {
        Point::Claimed => Err(eyre::eyre!("stopped, as a crash would")),
        _ => Ok(()),
    });
    let options = MigrateOptions {
        assume_format: Some(format),
        ..MigrateOptions::default()
    };
    engine
        .migrate(&project, &options, Consent::Command)
        .expect_err("the hook stops it");
    assert!(project.join(".comline.migrating").is_dir());
    project
}

#[test]
fn build_asks_to_finish_an_interrupted_migration_and_a_yes_migrates_and_builds() {
    let temp = tempfile::tempdir().unwrap();
    let project = interrupted_migration(temp.path());

    let mut session = on_a_terminal(&project, &["build"]);
    let asked = question(&mut session);
    assert!(asked.contains("was interrupted"), "{asked}");
    assert!(asked.contains("`comline store migrate`"), "{asked}");
    session.send_line("y").unwrap();
    let (output, code) = finish(session);

    assert_eq!(code, 0, "{output}");
    assert!(output.contains("Migrated the store"), "{output}");
    assert!(output.contains("Project built"), "{output}");
    assert!(stamp(&project).exists());
    assert!(!project.join(".comline.migrating").exists());
    assert!(
        project.read_dir().unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".bak")),
        "the old store is kept"
    );
}

#[test]
fn build_with_a_no_exits_two_and_changes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let project = interrupted_migration(temp.path());
    let before = snapshot(&project);

    let mut session = on_a_terminal(&project, &["build"]);
    question(&mut session);
    session.send_line("n").unwrap();
    let (output, code) = finish(session);

    assert_eq!(code, 2, "{output}");
    assert!(output.contains("`comline store migrate`"), "{output}");
    assert_eq!(snapshot(&project), before);
    assert!(project.join(".comline.migrating").is_dir());
}

/// A reset is never offered: a store with no path to the current format is
/// refused with the message that names `comline store reset`, and nothing is
/// asked.
#[test]
fn build_never_asks_to_reset() {
    let temp = tempfile::tempdir().unwrap();
    let project = built_project(temp.path());
    fs::write(stamp(&project), "1").unwrap();
    let before = snapshot(&project);

    let session = on_a_terminal(&project, &["build"]);
    let (output, code) = finish(session);

    assert_eq!(code, 2, "{output}");
    assert!(output.contains("`comline store reset`"), "{output}");
    assert!(!output.contains("[y/N]"), "{output}");
    assert_eq!(snapshot(&project), before);
}
