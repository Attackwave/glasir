//! Exercise rule selection and cache validity through the shipped executable.
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Tree(PathBuf);
impl Tree {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "glasir-rules-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn analyse(&self, overrides: bool) -> std::process::Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_glasir"));
        command.arg("analyse").arg(&self.0);
        if overrides {
            command.arg("--language-rules").arg(self.0.join("rules"));
        }
        command.output().unwrap()
    }
}
impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn rule_contents_invalidate_a_snapshot_without_source_edits() {
    let tree = Tree::new();
    std::fs::write(tree.0.join("main.go"), "package p\nfunc Run() { notify() }").unwrap();
    std::fs::create_dir(tree.0.join("rules")).unwrap();
    let original = include_str!("../parsers/languages/go.toml");
    let rule_path = tree.0.join("rules/go.toml");
    std::fs::write(&rule_path, original).unwrap();
    let first = tree.analyse(true);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let initial = std::fs::read(tree.0.join(".glasir-graph")).unwrap();
    assert!(String::from_utf8_lossy(&tree.analyse(true).stderr).contains("(incremental)"));
    std::fs::write(
        &rule_path,
        original.replace("exclude = [", "exclude = [\"notify\", "),
    )
    .unwrap();
    let changed = tree.analyse(true);
    assert!(
        changed.status.success(),
        "{}",
        String::from_utf8_lossy(&changed.stderr)
    );
    assert!(!String::from_utf8_lossy(&changed.stderr).contains("(incremental)"));
    assert_ne!(
        initial,
        std::fs::read(tree.0.join(".glasir-graph")).unwrap()
    );
    assert!(String::from_utf8_lossy(&tree.analyse(true).stderr).contains("(incremental)"));
    // No flag means bundled rules, even when a project carries override files.
    let default = tree.analyse(false);
    assert!(default.status.success());
    assert!(!String::from_utf8_lossy(&default.stderr).contains("(incremental)"));
}

#[test]
fn invalid_overrides_fail_before_a_graph_is_written() {
    let tree = Tree::new();
    std::fs::write(tree.0.join("main.go"), "package p").unwrap();
    std::fs::create_dir(tree.0.join("rules")).unwrap();
    for text in ["schema_version = 999".to_string(), "#".repeat(65 * 1024)] {
        std::fs::write(tree.0.join("rules/go.toml"), text).unwrap();
        let output = tree.analyse(true);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("go.toml"));
        assert!(!tree.0.join(".glasir-graph").exists());
    }
    let output = Command::new(env!("CARGO_BIN_EXE_glasir"))
        .arg("analyse")
        .arg(&tree.0)
        .arg("--language-rules")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires a directory"));
}
