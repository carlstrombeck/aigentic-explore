use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The working directory shared by the built-in tools. Cheap to clone; all
/// clones see the same directory. It also remembers the directory it was
/// created with, which is where a call falls back to once its own directory
/// no longer exists.
#[derive(Debug, Clone)]
pub struct Workdir {
    root: Arc<PathBuf>,
    state: Arc<Mutex<State>>,
}

#[derive(Debug)]
struct State {
    current: PathBuf,
    /// The directory a repair replaced, until [`Workdir::take_notice`] reports
    /// it.
    gone: Option<PathBuf>,
}

impl Workdir {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        Self {
            root: Arc::new(path.clone()),
            state: Arc::new(Mutex::new(State {
                current: path,
                gone: None,
            })),
        }
    }

    /// The directory this `Workdir` was created with: the directory the
    /// session started in, which never moves.
    pub fn root(&self) -> PathBuf {
        (*self.root).clone()
    }

    /// The directory the tools resolve against: the stored one when it is
    /// still a directory, else the root when that is one, else the stored
    /// path unchanged so the call fails as it would any other way.
    ///
    /// A fallback replaces the stored directory with the root and leaves a
    /// notice for [`Workdir::take_notice`]. Neither path is canonicalised.
    pub fn current(&self) -> PathBuf {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.current.is_dir() {
            return state.current.clone();
        }
        let root = &*self.root;
        if root.is_dir() && *root != state.current {
            let gone = std::mem::replace(&mut state.current, root.clone());
            state.gone = Some(gone);
            return root.clone();
        }
        state.current.clone()
    }

    /// Change the directory the tools resolve against. The root is never
    /// touched, so a `cd` cannot move where a fallback goes, and a notice
    /// already pending is left for its own call to report.
    pub fn set(&self, path: impl Into<PathBuf>) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).current = path.into();
    }

    /// Take the line announcing that the stored directory was gone and the
    /// root was used instead, or `None` when no fallback is pending.
    ///
    /// The line is consumed: only the call that takes it reports it. A caller
    /// that can still fail takes it once it has a result, so a failed call
    /// leaves the line for the next one.
    pub fn take_notice(&self) -> Option<String> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let gone = state.gone.take()?;
        Some(format!(
            "the working directory {} is gone; ran in {}\n",
            gone.display(),
            state.current.display()
        ))
    }

    /// Resolve `path` against the current directory. Absolute paths pass through.
    pub fn resolve(&self, path: impl AsRef<Path>) -> PathBuf {
        self.current().join(path)
    }
}

/// Put a pending fallback line in front of a tool's result content. Tools go
/// through here so the line's position and wording exist once.
pub(crate) fn with_notice(notice: Option<String>, content: String) -> String {
    match notice {
        Some(line) => line + &content,
        None => content,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fallback_line(gone: &Path, ran_in: &Path) -> String {
        format!(
            "the working directory {} is gone; ran in {}\n",
            gone.display(),
            ran_in.display()
        )
    }

    #[test]
    fn workdir_falls_back_to_the_root_and_reports_once() {
        let root = tempfile::tempdir().unwrap();
        let gone = root.path().join("gone");
        std::fs::create_dir(&gone).unwrap();
        let w = Workdir::new(root.path());
        w.set(&gone);
        std::fs::remove_dir_all(&gone).unwrap();

        assert_eq!(w.current(), root.path());
        assert_eq!(
            w.take_notice(),
            Some(fallback_line(&gone, root.path())),
            "the repair reports the directory it left and the one it used"
        );
        assert_eq!(w.take_notice(), None, "the line is reported once");
        assert_eq!(w.root(), root.path(), "a fallback keeps the root");

        let again = root.path().join("gone-again");
        w.set(&again);
        assert_eq!(w.current(), root.path());
        assert_eq!(w.take_notice(), Some(fallback_line(&again, root.path())));
    }

    #[test]
    fn workdir_falls_back_when_the_stored_path_is_a_file() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("f");
        std::fs::write(&file, "x").unwrap();
        let w = Workdir::new(root.path());
        w.set(&file);

        assert_eq!(w.current(), root.path());
        assert_eq!(w.take_notice(), Some(fallback_line(&file, root.path())));
    }

    #[test]
    fn workdir_keeps_the_stored_path_when_the_root_is_gone() {
        let gone = {
            let d = tempfile::tempdir().unwrap();
            d.path().join("nope")
        };
        let w = Workdir::new(&gone);

        assert_eq!(
            w.current(),
            gone,
            "a root that is not a directory is never used"
        );
        assert_eq!(w.take_notice(), None);
    }
}
