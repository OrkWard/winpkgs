//! Install, upgrade, GC and uninstall end to end, with a local bucket and
//! file:// downloads.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use libwpk::config::BucketEntry;
use libwpk::install::Event;
use libwpk::shim::ShimExes;
use libwpk::{Config, Layout, Wpk, bucket, download, installed, manifest::HashAlgo};

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

struct Fixture {
    root: PathBuf,
    upstream: PathBuf,
    wpk: Wpk,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("wpk-install-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let upstream = root.join("upstream");
        std::fs::create_dir_all(upstream.join("bucket")).unwrap();
        git(&upstream, &["init", "-q"]);
        std::fs::write(upstream.join("bucket/.keep"), "").unwrap();
        git(&upstream, &["add", "."]);
        git(&upstream, &["commit", "-q", "-m", "init"]);

        let layout = Layout::new(root.join("wpk"), root.join("cache"));
        bucket::add(&layout, "t", upstream.to_str().unwrap(), None).unwrap();
        std::fs::write(root.join("console.exe"), "console shim").unwrap();
        std::fs::write(root.join("gui.exe"), "gui shim").unwrap();
        let config = Config {
            buckets: vec![BucketEntry {
                name: "t".into(),
                url: upstream.display().to_string(),
            }],
            ..Config::default()
        };
        let wpk = Wpk {
            layout,
            config,
            shims: ShimExes {
                console: root.join("console.exe"),
                gui: root.join("gui.exe"),
            },
        };
        Fixture {
            root,
            upstream,
            wpk,
        }
    }

    /// Publish version `v` of package `foo` and update the bucket.
    fn publish(&self, v: &str) {
        let zip_path = self.root.join(format!("foo-{v}.zip"));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        for (name, content) in [
            (format!("foo-{v}/foo.exe"), format!("foo {v}")),
            (format!("foo-{v}/conf/default.ini"), "default".to_owned()),
        ] {
            zip.start_file(name, opts).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        let hash = download::hash_file(&zip_path, HashAlgo::Sha256).unwrap();
        let url = format!(
            "file:///{}",
            zip_path.display().to_string().replace('\\', "/")
        );
        let manifest = serde_json::json!({
            "version": v,
            "url": url,
            "hash": hash,
            "extract_dir": format!("foo-{v}"),
            "bin": ["foo.exe", ["foo.exe", "foo2", "--data $persist_dir"]],
            "shortcuts": [["foo.exe", "Foo App"]],
            "persist": ["conf", "settings.ini"],
            "notes": "hello from $dir",
            "env_set": {"FOO_HOME": "$dir"}
        });
        std::fs::write(self.upstream.join("bucket/foo.json"), manifest.to_string()).unwrap();
        git(&self.upstream, &["add", "."]);
        git(&self.upstream, &["commit", "-q", "-m", v]);
        bucket::update(&self.wpk.layout, "t", None).unwrap();
    }
}

#[test]
fn install_upgrade_gc_uninstall() {
    let f = Fixture::new("cycle");
    let layout = &f.wpk.layout;
    let log = Mutex::new(Vec::new());
    let reporter = |e: Event<'_>| log.lock().unwrap().push(format!("{e:?}"));

    f.publish("1");
    f.wpk.install(&["foo".into()], &reporter).unwrap();
    assert_eq!(
        installed::current(layout, "foo").unwrap().as_deref(),
        Some("1")
    );
    let v1 = layout.version("foo", "1");
    let shim =
        |name: &str| std::fs::read_to_string(layout.bin().join(format!("{name}.shim"))).unwrap();
    assert!(shim("foo").contains(&v1.join("foo.exe").display().to_string()));
    assert!(shim("foo2").contains(&format!(
        "args = --data {}",
        layout.persist("foo").display()
    )));
    assert_eq!(
        std::fs::read_to_string(layout.bin().join("foo.exe")).unwrap(),
        "console shim"
    );
    assert_eq!(
        std::fs::read_to_string(layout.persist("foo").join("conf/default.ini")).unwrap(),
        "default"
    );
    assert!(layout.persist("foo").join("settings.ini").is_file());
    assert!(layout.start_menu().join("Foo App.lnk").exists());
    let launcher = std::fs::read_to_string(layout.launchers("foo").join("Foo App.shim")).unwrap();
    assert!(launcher.contains(&format!("dir = {}", v1.display())));
    let receipt = installed::receipt(layout, "foo", "1").unwrap();
    assert_eq!(receipt.bins, ["foo", "foo2"]);
    assert!(receipt.explicit);
    let logged = log.lock().unwrap().join("\n");
    assert!(logged.contains("Notes") && logged.contains("EnvSet"), "{logged}");
    // Notes get the same variables as bin arguments.
    assert!(logged.contains(&format!("hello from {}", v1.display()).replace('\\', "\\\\")), "{logged}");
    assert!(
        logged.contains("Notes") && logged.contains("EnvSet"),
        "{logged}"
    );

    // Reinstalling skips.
    f.wpk.install(&["foo".into()], &reporter).unwrap();
    assert!(log.lock().unwrap().last().unwrap().contains("Skipped"));

    // User data written through the link survives the upgrade.
    std::fs::write(v1.join("conf/user.ini"), "mine").unwrap();
    f.publish("2");
    f.wpk.upgrade(&[], &reporter).unwrap();
    let v2 = layout.version("foo", "2");
    assert_eq!(
        installed::current(layout, "foo").unwrap().as_deref(),
        Some("2")
    );
    assert!(shim("foo").contains(&v2.join("foo.exe").display().to_string()));
    assert_eq!(
        std::fs::read_to_string(v2.join("conf/user.ini")).unwrap(),
        "mine"
    );
    // The first install ran GC, so the next is not due: v1 stays for now.
    assert!(v1.exists());

    libwpk::gc::run(layout).unwrap();
    assert!(!v1.exists() && v2.exists());
    assert!(layout.persist("foo").join("conf/user.ini").exists());

    installed::set_held(layout, "foo", true).unwrap();
    f.publish("3");
    f.wpk.upgrade(&[], &reporter).unwrap();
    assert_eq!(
        installed::current(layout, "foo").unwrap().as_deref(),
        Some("2")
    );
    installed::set_held(layout, "foo", false).unwrap();

    f.wpk.uninstall(&["foo".into()], false, &reporter).unwrap();
    assert!(!layout.bin().join("foo.exe").exists());
    assert!(!layout.bin().join("foo2.shim").exists());
    assert!(!layout.start_menu().join("Foo App.lnk").exists());
    assert!(!layout.package("foo").exists());
    assert!(layout.persist("foo").join("conf/user.ini").exists());
}

#[test]
fn failed_link_restores_previous_version() {
    let f = Fixture::new("rollback");
    let layout = &f.wpk.layout;
    let reporter = |_: Event<'_>| {};
    f.publish("1");
    f.wpk.install(&["foo".into()], &reporter).unwrap();
    let shim = || std::fs::read_to_string(layout.bin().join("foo.shim")).unwrap();
    let v1_shim = shim();

    // Shortcuts cannot be written once the Start menu folder is a file.
    std::fs::remove_dir_all(layout.start_menu()).unwrap();
    std::fs::write(layout.start_menu(), "").unwrap();
    f.publish("2");
    assert!(f.wpk.upgrade(&[], &reporter).is_err());
    assert_eq!(
        installed::current(layout, "foo").unwrap().as_deref(),
        Some("1")
    );
    assert_eq!(shim(), v1_shim);
    // Relinking hit the same shortcut failure, so v2 stays for GC.
    libwpk::gc::run(layout).unwrap();
    assert!(!layout.version("foo", "2").exists());
    assert!(layout.version("foo", "1").exists());
}

#[test]
fn multicall_bins_get_hard_links() {
    let f = Fixture::new("multicall");
    let layout = &f.wpk.layout;
    let zip_path = f.root.join("multi.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
    zip.start_file("tool.exe", zip::write::SimpleFileOptions::default()).unwrap();
    zip.write_all(b"multi-call").unwrap();
    zip.finish().unwrap();
    let url = format!("file:///{}", zip_path.display().to_string().replace('\\', "/"));
    let manifest = serde_json::json!({
        "version": "1",
        "url": url,
        "multicall": true,
        "bin": ["tool.exe", ["tool.exe", "alias"]]
    });
    std::fs::write(f.upstream.join("bucket/multi.json"), manifest.to_string()).unwrap();
    git(&f.upstream, &["add", "."]);
    git(&f.upstream, &["commit", "-q", "-m", "multi"]);
    bucket::update(layout, "t", None).unwrap();

    f.wpk.install(&["multi".into()], &|_: Event<'_>| {}).unwrap();
    let dir = layout.version("multi", "1");
    assert_eq!(std::fs::read(dir.join("alias.exe")).unwrap(), b"multi-call");
    let shim = std::fs::read_to_string(layout.bin().join("alias.shim")).unwrap();
    assert!(shim.contains(&dir.join("alias.exe").display().to_string()), "{shim}");
}

#[test]
fn persist_copy_follows_upgrades_and_uninstall() {
    let f = Fixture::new("persist-copy");
    let layout = &f.wpk.layout;
    let publish = |v: &str| {
        let zip_path = f.root.join(format!("app-{v}.zip"));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("app.exe", opts).unwrap();
        zip.write_all(v.as_bytes()).unwrap();
        zip.start_file("app.ini", opts).unwrap();
        zip.write_all(b"shipped").unwrap();
        zip.finish().unwrap();
        let url = format!("file:///{}", zip_path.display().to_string().replace('\\', "/"));
        let manifest = serde_json::json!({
            "version": v, "url": url, "bin": "app.exe", "persist_copy": ["app.ini", "app.db"]
        });
        std::fs::write(f.upstream.join("bucket/app.json"), manifest.to_string()).unwrap();
        git(&f.upstream, &["add", "."]);
        git(&f.upstream, &["commit", "-q", "-m", v]);
        bucket::update(layout, "t", None).unwrap();
    };
    let reporter = |_: Event<'_>| {};

    publish("1");
    f.wpk.install(&["app".into()], &reporter).unwrap();
    let v1 = layout.version("app", "1");
    assert_eq!(std::fs::read_to_string(v1.join("app.ini")).unwrap(), "shipped");
    assert!(!v1.join("app.db").exists());
    // The app replaces its files when saving.
    std::fs::remove_file(v1.join("app.ini")).unwrap();
    std::fs::write(v1.join("app.ini"), "user").unwrap();
    std::fs::write(v1.join("app.db"), "db").unwrap();

    publish("2");
    f.wpk.upgrade(&[], &reporter).unwrap();
    let v2 = layout.version("app", "2");
    assert_eq!(std::fs::read_to_string(v2.join("app.ini")).unwrap(), "user");
    assert_eq!(std::fs::read_to_string(v2.join("app.db")).unwrap(), "db");

    f.wpk.uninstall(&["app".into()], false, &reporter).unwrap();
    assert_eq!(std::fs::read_to_string(layout.persist("app").join("app.ini")).unwrap(), "user");
    f.wpk.install(&["app".into()], &reporter).unwrap();
    assert_eq!(std::fs::read_to_string(v2.join("app.ini")).unwrap(), "user");
}

#[test]
fn rejects_unsupported_and_conflicts() {
    let f = Fixture::new("reject");
    let reporter = |_: Event<'_>| {};
    let write = |name: &str, json: serde_json::Value| {
        std::fs::write(
            f.upstream.join(format!("bucket/{name}.json")),
            json.to_string(),
        )
        .unwrap();
    };
    write(
        "scripted",
        serde_json::json!({"version": "1", "url": "file:///x.zip", "post_install": "x"}),
    );
    git(&f.upstream, &["add", "."]);
    git(&f.upstream, &["commit", "-q", "-m", "s"]);
    bucket::update(&f.wpk.layout, "t", None).unwrap();
    let err = f.wpk.install(&["scripted".into()], &reporter).unwrap_err();
    assert!(err.to_string().contains("post_install"), "{err}");

    f.publish("1");
    f.wpk.install(&["foo".into()], &reporter).unwrap();
    let foo = std::fs::read_to_string(f.upstream.join("bucket/foo.json")).unwrap();
    std::fs::write(f.upstream.join("bucket/bar.json"), foo).unwrap();
    git(&f.upstream, &["add", "."]);
    git(&f.upstream, &["commit", "-q", "-m", "bar"]);
    bucket::update(&f.wpk.layout, "t", None).unwrap();
    let err = f.wpk.install(&["bar".into()], &reporter).unwrap_err();
    assert!(matches!(err, libwpk::Error::Conflict { .. }), "{err}");
}
