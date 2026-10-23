//! The file tools fall back to the session's root when their working directory
//! has been deleted, and say so on the first line of the result.

use std::path::{Path, PathBuf};

use aigentic_core::Tool;
use aigentic_tools::{EditFileTool, GrepTool, ListDirTool, Workdir, WriteFileTool};
use serde_json::json;

/// A project root holding `README.md` and `src/main.rs`, a workdir moved into a
/// child directory and that directory removed: the next call repairs. The
/// removed path is returned so the expected line is built from it.
fn fallen_back() -> (tempfile::TempDir, Workdir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("README.md"), "# hello\n").unwrap();
    std::fs::create_dir(root.path().join("src")).unwrap();
    std::fs::write(root.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    let gone = root.path().join("gone");
    std::fs::create_dir(&gone).unwrap();
    let workdir = Workdir::new(root.path());
    workdir.set(&gone);
    std::fs::remove_dir_all(&gone).unwrap();
    (root, workdir, gone)
}

fn notice(root: &Path, gone: &Path) -> String {
    format!(
        "the working directory {} is gone; ran in {}\n",
        gone.display(),
        root.display()
    )
}

#[tokio::test]
async fn write_file_and_edit_file_report_the_fallback_and_work_in_the_root() {
    let (root, workdir, gone) = fallen_back();
    let line = notice(root.path(), &gone);
    let write = WriteFileTool::new(workdir);
    let out = write
        .call(json!({"path": "new/n.txt", "content": "one\n"}))
        .await
        .unwrap();
    assert!(out.content.starts_with(&line), "{}", out.content);
    assert!(
        out.content.contains(&format!(
            "wrote {} bytes to {}",
            "one\n".len(),
            root.path().join("new/n.txt").display()
        )),
        "{}",
        out.content
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("new/n.txt")).unwrap(),
        "one\n",
        "the file landed under the root, not the removed directory"
    );

    let (root, workdir, gone) = fallen_back();
    let line = notice(root.path(), &gone);
    let edit = EditFileTool::new(workdir);
    let out = edit
        .call(json!({"path": "README.md", "old_string": "hello", "new_string": "hej"}))
        .await
        .unwrap();
    assert!(out.content.starts_with(&line), "{}", out.content);
    assert_eq!(
        std::fs::read_to_string(root.path().join("README.md")).unwrap(),
        "# hej\n"
    );
}

#[tokio::test]
async fn list_dir_and_grep_report_the_fallback_and_read_the_root() {
    let (root, workdir, gone) = fallen_back();
    let line = notice(root.path(), &gone);
    let out = ListDirTool::new(workdir).call(json!({})).await.unwrap();
    assert!(out.content.starts_with(&line), "{}", out.content);
    let listing = out.content.strip_prefix(&line).unwrap();
    assert!(
        listing.contains(&format!("README.md  {}", "# hello\n".len())),
        "{listing}"
    );
    assert!(listing.contains("src/"), "{listing}");

    let (root, workdir, gone) = fallen_back();
    let line = notice(root.path(), &gone);
    let out = GrepTool::new(workdir)
        .call(json!({"pattern": "hello"}))
        .await
        .unwrap();
    assert!(out.content.starts_with(&line), "{}", out.content);
    assert!(
        out.content.contains("README.md:1:# hello"),
        "{}",
        out.content
    );
}
