//! Bucket clone and update against a local upstream made with the git CLI.

use std::path::{Path, PathBuf};
use std::process::Command;

use libwpk::{Layout, bucket};

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?}");
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("wpk-bucket-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn commit_manifest(upstream: &Path, version: &str) {
    let json = format!(r#"{{"version": "{version}", "url": "https://h/a.zip"}}"#);
    std::fs::write(upstream.join("bucket").join("a.json"), json).unwrap();
    git(upstream, &["add", "."]);
    git(upstream, &["commit", "-q", "-m", version]);
}

fn version(layout: &Layout) -> String {
    let b = bucket::Bucket::open(layout, "t").unwrap();
    let json = b.manifest("a").unwrap().unwrap();
    libwpk::Manifest::parse(&json).unwrap().version
}

#[test]
fn clone_then_follow_new_and_rewritten_commits() {
    let root = temp("follow");
    let upstream = root.join("upstream");
    std::fs::create_dir_all(upstream.join("bucket")).unwrap();
    git(&upstream, &["init", "-q", "-b", "trunk"]);
    commit_manifest(&upstream, "1");

    let layout = Layout::new(root.join("wpk"), root.join("cache"));
    let url = upstream.to_str().unwrap();
    bucket::add(&layout, "t", url, None).unwrap();
    assert_eq!(version(&layout), "1");
    assert!(matches!(
        bucket::add(&layout, "t", url, None),
        Err(libwpk::Error::BucketExists(_))
    ));

    commit_manifest(&upstream, "2");
    bucket::update(&layout, "t", None).unwrap();
    assert_eq!(version(&layout), "2");

    // A force-pushed history replaces ours.
    git(&upstream, &["reset", "-q", "--hard", "HEAD~1"]);
    commit_manifest(&upstream, "3");
    bucket::update(&layout, "t", None).unwrap();
    assert_eq!(version(&layout), "3");

    let names: Vec<_> = bucket::Bucket::open(&layout, "t")
        .unwrap()
        .manifests()
        .unwrap()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(names, ["a"]);
}
