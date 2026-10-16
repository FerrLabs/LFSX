use super::*;

#[test]
fn the_oid_is_the_first_column_of_ls_files() {
    let output = "\
5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03 * Assets/Art/Hero.psd
b1946ac92492d2347c6235b4d2611184d4b1b5a9c8a7f7e6d5c4b3a2918070f1 - Assets/Scenes/Arena.unity
";

    assert_eq!(
        parse_ls_files(output),
        vec![
            "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03",
            "b1946ac92492d2347c6235b4d2611184d4b1b5a9c8a7f7e6d5c4b3a2918070f1"
        ]
    );
}

#[test]
fn anything_that_is_not_a_digest_is_dropped() {
    let output = "\
short * Assets/Art/Hero.psd
zzz1b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03 * Assets/Art/Villain.psd

";

    assert!(
        parse_ls_files(output).is_empty(),
        "a malformed line must not be sent as a retained oid, since anything not retained is swept"
    );
}

#[test]
fn an_uppercase_digest_is_sent_lowercase() {
    let output =
        "5891B5B522D5DF086D0FF0B110FBD9D21BB4FC7163AF34D08286A2E846F6BE03 * Assets/Art/Hero.psd\n";

    assert_eq!(
        parse_ls_files(output),
        vec!["5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03"],
        "the server matches digests exactly, so an uppercase one would let its object be swept"
    );
}

fn commit(dir: &Path) {
    git(
        dir,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-m",
            "x",
        ],
    )
    .unwrap();
}

fn origin() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]).unwrap();
    commit(dir.path());
    dir
}

fn file_url(dir: &tempfile::TempDir) -> String {
    format!("file://{}", dir.path().to_string_lossy().replace('\\', "/"))
}

fn clone_of(origin: &tempfile::TempDir, flags: &[&str]) -> tempfile::TempDir {
    let clone = tempfile::tempdir().unwrap();
    let url = file_url(origin);
    let target = clone.path().to_string_lossy().into_owned();
    let args = [&["clone", "-q"][..], flags, &[&url, &target]].concat();
    git(Path::new("."), &args).unwrap();
    clone
}

fn has_ref(dir: &Path, name: &str) -> bool {
    run_git(dir, &["rev-parse", "--verify", "-q", name])
        .unwrap()
        .status
        .success()
}

#[test]
fn only_a_wildcard_over_heads_counts_as_every_branch() {
    assert!(covers_heads("+refs/heads/*:refs/remotes/origin/*"));
    assert!(covers_heads("refs/heads/*:refs/remotes/origin/*"));
    assert!(covers_heads("+refs/*:refs/*"));
    assert!(!covers_heads("+refs/heads/main:refs/remotes/origin/main"));
    assert!(!covers_heads(
        "+refs/heads/release/*:refs/remotes/origin/release/*"
    ));
    assert!(!covers_heads("+refs/tags/*:refs/tags/*"));
}

#[test]
fn a_single_branch_clone_is_refused() {
    let origin = origin();
    let clone = clone_of(&origin, &["--single-branch"]);

    let error = prepare(clone.path(), true).unwrap_err().to_string();

    assert!(error.contains("does not fetch every branch"), "{error}");
}

#[test]
fn a_shallow_clone_is_refused() {
    let origin = origin();
    commit(origin.path());
    let clone = clone_of(&origin, &["--depth", "1"]);

    let error = prepare(clone.path(), true).unwrap_err().to_string();

    assert!(error.contains("shallow clone"), "{error}");
}

#[test]
fn preparing_fetches_new_branches_tags_and_review_refs() {
    let origin = origin();
    let clone = clone_of(&origin, &[]);
    git(origin.path(), &["branch", "feature"]).unwrap();
    git(origin.path(), &["tag", "v1"]).unwrap();
    git(origin.path(), &["update-ref", "refs/pull/7/head", "HEAD"]).unwrap();
    git(
        origin.path(),
        &["update-ref", "refs/merge-requests/3/head", "HEAD"],
    )
    .unwrap();

    prepare(clone.path(), true).unwrap();

    assert!(has_ref(clone.path(), "refs/remotes/origin/feature"));
    assert!(has_ref(clone.path(), "refs/tags/v1"));
    assert!(has_ref(clone.path(), "refs/lfsx/pull/7/head"));
    assert!(has_ref(clone.path(), "refs/lfsx/merge-requests/3/head"));
}

#[test]
fn review_refs_can_be_left_out() {
    let origin = origin();
    let clone = clone_of(&origin, &[]);
    git(origin.path(), &["update-ref", "refs/pull/7/head", "HEAD"]).unwrap();

    prepare(clone.path(), false).unwrap();

    assert!(!has_ref(clone.path(), "refs/lfsx/pull/7/head"));
}

#[test]
fn a_closed_review_ref_stops_being_retained() {
    let origin = origin();
    let clone = clone_of(&origin, &[]);
    git(origin.path(), &["update-ref", "refs/pull/7/head", "HEAD"]).unwrap();
    prepare(clone.path(), true).unwrap();
    git(origin.path(), &["update-ref", "-d", "refs/pull/7/head"]).unwrap();

    prepare(clone.path(), true).unwrap();

    assert!(!has_ref(clone.path(), "refs/lfsx/pull/7/head"));
}

#[test]
fn a_failing_fetch_is_an_error_not_a_smaller_keep_list() {
    let origin = origin();
    let clone = clone_of(&origin, &[]);
    git(
        clone.path(),
        &["remote", "set-url", REMOTE, "file:///nonexistent/repo"],
    )
    .unwrap();

    assert!(prepare(clone.path(), true).is_err());
}

#[test]
fn the_lfs_url_forms_all_name_the_same_repository() {
    for url in [
        "https://lfs.example.com/my-org/my-project",
        "https://lfs.example.com/my-org/my-project/",
        "https://lfs.example.com/my-org/my-project.git",
        "https://lfs.example.com/my-org/my-project.git/info/lfs",
    ] {
        let identity = parse_lfs_url(url).unwrap();
        assert_eq!(identity.base, "https://lfs.example.com", "{url}");
        assert_eq!(identity.repo, "my-org/my-project", "{url}");
    }
}

#[test]
fn a_forge_prefix_stays_in_the_base() {
    let identity = parse_lfs_url("https://lfs.example.com/-/work/my-org/my-project").unwrap();

    assert_eq!(identity.base, "https://lfs.example.com/-/work");
}

#[test]
fn an_lfs_url_without_a_repository_is_not_parsed() {
    assert!(parse_lfs_url("https://lfs.example.com/my-project").is_none());
    assert!(parse_lfs_url("my-org/my-project").is_none());
}

fn clone_with_lfs_url(url: &str) -> tempfile::TempDir {
    let dir = origin();
    git(dir.path(), &["config", "lfs.url", url]).unwrap();
    dir
}

#[test]
fn url_and_repo_are_inferred_from_the_clone() {
    let clone = clone_with_lfs_url("https://lfs.example.com/my-org/my-project.git/info/lfs");

    assert_eq!(
        resolve_url(clone.path(), None).unwrap(),
        "https://lfs.example.com"
    );
    assert_eq!(
        resolve_repo(clone.path(), None).unwrap(),
        "my-org/my-project"
    );
}

#[test]
fn matching_explicit_values_are_accepted() {
    let clone = clone_with_lfs_url("https://lfs.example.com/my-org/my-project");

    assert_eq!(
        resolve_url(clone.path(), Some("https://lfs.example.com/")).unwrap(),
        "https://lfs.example.com/"
    );
    assert_eq!(
        resolve_repo(clone.path(), Some("My-Org/my-project")).unwrap(),
        "My-Org/my-project"
    );
}

#[test]
fn a_url_that_is_not_the_clones_server_is_refused() {
    let clone = clone_with_lfs_url("https://lfs.example.com/my-org/my-project");

    let error = resolve_url(clone.path(), Some("https://other.example.com"))
        .unwrap_err()
        .to_string();

    assert!(error.contains("lfs.url"), "{error}");
}

#[test]
fn a_repo_that_is_not_the_clones_repository_is_refused() {
    let clone = clone_with_lfs_url("https://lfs.example.com/my-org/my-project");

    let error = resolve_repo(clone.path(), Some("my-org/other"))
        .unwrap_err()
        .to_string();

    assert!(error.contains("lfs.url"), "{error}");
}

#[test]
fn without_an_lfs_url_both_must_be_given() {
    let clone = origin();

    assert!(resolve_url(clone.path(), None).is_err());
    assert!(resolve_repo(clone.path(), None).is_err());
    assert_eq!(resolve_repo(clone.path(), Some("o/r")).unwrap(), "o/r");
}

fn clone_with_lfsconfig(url: &str) -> tempfile::TempDir {
    let dir = origin();
    let contents = format!("[lfs]\n\turl = {url}\n");
    std::fs::write(dir.path().join(".lfsconfig"), contents).unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    dir
}

#[test]
fn a_committed_lfsconfig_is_read_from_a_subdirectory() {
    let clone = clone_with_lfsconfig("https://lfs.example.com/my-org/my-project");
    let sub = clone.path().join("sub");

    assert_eq!(resolve_url(&sub, None).unwrap(), "https://lfs.example.com");
    assert_eq!(resolve_repo(&sub, None).unwrap(), "my-org/my-project");
    assert!(resolve_repo(&sub, Some("my-org/other")).is_err());
    assert!(resolve_url(&sub, Some("https://other.example.com")).is_err());
}

#[test]
fn git_config_wins_over_a_committed_lfsconfig() {
    let clone = clone_with_lfsconfig("https://lfs.example.com/my-org/my-project");
    git(
        clone.path(),
        &["config", "lfs.url", "https://local.example.com/my-org/mine"],
    )
    .unwrap();

    assert_eq!(
        resolve_url(clone.path(), None).unwrap(),
        "https://local.example.com"
    );
    assert_eq!(resolve_repo(clone.path(), None).unwrap(), "my-org/mine");
}
