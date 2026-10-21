//! The forge seam: the issue a run is for, and the comments a run posts.
//!
//! [`GhForge`] shells out to `gh`, which is why a comment body never goes
//! through `argv`: no quoting rule and no size limit stands between a
//! report and the issue.

use std::io::Write as _;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ulid::Ulid;

/// The issue a run is for, as the forge reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IssueView {
    /// The issue's title.
    pub title: String,
    /// The issue's body, as written.
    pub body: String,
}

/// What the forge's CI says about one commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CiState {
    /// No run names the commit yet: CI may not have picked it up.
    NoRuns,
    /// At least one run is queued or in progress, and none has failed.
    Pending,
    /// Every run finished, and every one succeeded or was skipped.
    Passed,
    /// A run finished with another conclusion, named here.
    Failed(String),
}

/// Every way a forge can fail.
#[derive(Debug, thiserror::Error)]
pub enum ForgeError {
    /// `gh` ran and said no.
    #[error("gh {args} failed: {message}")]
    Command {
        /// The arguments, space-joined, for the message.
        args: String,
        /// What `gh` wrote on stderr.
        message: String,
    },
    /// `gh` printed something this code cannot read.
    #[error("gh printed no readable JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// `gh` could not be run at all, or its temp file could not be written.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl ForgeError {
    /// Whether this failure is the network dropping the call rather
    /// than `gh` saying no: a timeout, a refused or reset connection, a
    /// DNS failure, or a 5xx or 429 from the API. Only what `gh` wrote
    /// on stderr can be one; a body that arrived but would not parse,
    /// or a `gh` that would not spawn, is not.
    pub fn is_network_fault(&self) -> bool {
        match self {
            Self::Command { message, .. } => names_a_network_fault(message),
            Self::Json(_) | Self::Io(_) => false,
        }
    }
}

/// Whether `text`, the stderr `gh` left, says the network dropped the
/// call. The marks are matched anywhere in the text, because `gh` writes
/// a warning line above the error it died on, and the status rule is the
/// one [`ProviderError::is_transient`](aigentic_core::ProviderError::is_transient)
/// applies: a 429 or a 5xx is worth another try, a 4xx is a refusal.
fn names_a_network_fault(text: &str) -> bool {
    const CUT: [&str; 12] = [
        "i/o timeout",
        "connection timed out",
        "timeout exceeded",
        "context deadline exceeded",
        "tls handshake timeout",
        "connection refused",
        "connection reset",
        "unexpected eof",
        "no such host",
        "temporary failure in name resolution",
        "server misbehaving",
        "network is unreachable",
    ];
    let text = text.to_ascii_lowercase();
    if CUT.iter().any(|mark| text.contains(mark)) {
        return true;
    }
    text.match_indices("http ").any(|(at, _)| {
        matches!(
            text[at + "http ".len()..]
                .get(..3)
                .and_then(|code| code.parse::<u16>().ok()),
            Some(429 | 500..=599)
        )
    })
}

/// Read the issue and post comments on it.
pub trait Forge {
    /// The issue's title and body.
    fn issue(&self, n: u64) -> Result<IssueView, ForgeError>;
    /// The body of every comment on the issue, oldest first.
    fn comments(&self, n: u64) -> Result<Vec<String>, ForgeError>;
    /// Post one comment.
    fn comment(&self, n: u64, body: &str) -> Result<(), ForgeError>;
    /// Close the issue. An issue that is already closed is success, not a
    /// failure: a rebuilt runner closes what the crash left open.
    fn close(&self, n: u64) -> Result<(), ForgeError>;
    /// The CI runs for the full commit SHA `sha`.
    fn ci(&self, sha: &str) -> Result<CiState, ForgeError>;
}

impl<T: Forge + ?Sized> Forge for Arc<T> {
    fn issue(&self, n: u64) -> Result<IssueView, ForgeError> {
        (**self).issue(n)
    }

    fn comments(&self, n: u64) -> Result<Vec<String>, ForgeError> {
        (**self).comments(n)
    }

    fn comment(&self, n: u64, body: &str) -> Result<(), ForgeError> {
        (**self).comment(n, body)
    }

    fn close(&self, n: u64) -> Result<(), ForgeError> {
        (**self).close(n)
    }

    fn ci(&self, sha: &str) -> Result<CiState, ForgeError> {
        (**self).ci(sha)
    }
}

/// The real forge: this repository's GitHub issues, through `gh`.
#[derive(Debug, Clone, Copy, Default)]
pub struct GhForge;

impl GhForge {
    /// A forge that runs `gh` from `PATH`.
    pub fn new() -> Self {
        Self
    }
}

impl Forge for GhForge {
    fn issue(&self, n: u64) -> Result<IssueView, ForgeError> {
        #[derive(serde::Deserialize)]
        struct Wire {
            #[serde(default)]
            title: Option<String>,
            #[serde(default)]
            body: Option<String>,
        }
        let text = gh(&["issue", "view", &n.to_string(), "--json", "title,body"])?;
        let wire: Wire = serde_json::from_str(&text)?;
        Ok(IssueView {
            title: wire.title.unwrap_or_default(),
            body: wire.body.unwrap_or_default(),
        })
    }

    fn comments(&self, n: u64) -> Result<Vec<String>, ForgeError> {
        #[derive(serde::Deserialize)]
        struct Comment {
            #[serde(default)]
            body: Option<String>,
        }
        #[derive(serde::Deserialize)]
        struct Wire {
            #[serde(default)]
            comments: Vec<Comment>,
        }
        let text = gh(&["issue", "view", &n.to_string(), "--json", "comments"])?;
        let wire: Wire = serde_json::from_str(&text)?;
        Ok(wire
            .comments
            .into_iter()
            .map(|c| c.body.unwrap_or_default())
            .collect())
    }

    fn comment(&self, n: u64, body: &str) -> Result<(), ForgeError> {
        // The body goes in a file: a report is long, and argv is not the
        // place for it.
        let path = std::env::temp_dir().join(format!(
            "aigentic-comment-{}-{}.md",
            std::process::id(),
            Ulid::generate()
        ));
        {
            let mut file = std::fs::File::create(&path)?;
            file.write_all(body.as_bytes())?;
            file.flush()?;
        }
        let arg = path.to_string_lossy().into_owned();
        let result = gh(&["issue", "comment", &n.to_string(), "--body-file", &arg]);
        let _ = std::fs::remove_file(&path);
        result.map(|_| ())
    }

    fn close(&self, n: u64) -> Result<(), ForgeError> {
        let number = n.to_string();
        let Err(failure) = gh(&["issue", "close", &number]) else {
            return Ok(());
        };
        // Closing what is already closed is success, not a failure: a
        // rebuilt runner finishes what the crash left open. `gh`'s exit
        // status alone cannot tell the two apart, so ask for the state.
        if self.state(&number)? == "CLOSED" {
            return Ok(());
        }
        Err(failure)
    }

    fn ci(&self, sha: &str) -> Result<CiState, ForgeError> {
        #[derive(serde::Deserialize)]
        struct Run {
            #[serde(default)]
            status: Option<String>,
            #[serde(default)]
            conclusion: Option<String>,
            #[serde(default)]
            name: Option<String>,
        }
        let text = gh(&[
            "run",
            "list",
            "--commit",
            sha,
            "--json",
            "status,conclusion,name",
        ])?;
        let runs: Vec<Run> = serde_json::from_str(&text)?;
        Ok(fold_ci(runs.into_iter().map(|run| {
            (
                run.name.unwrap_or_default(),
                run.status.unwrap_or_default(),
                run.conclusion.unwrap_or_default(),
            )
        })))
    }
}

/// Fold `(name, status, conclusion)` per run into one state. A run that
/// isn't `completed` is pending; a completed run whose conclusion isn't
/// `success`, `skipped` or `neutral` fails the commit, named.
pub fn fold_ci(runs: impl IntoIterator<Item = (String, String, String)>) -> CiState {
    let mut any = false;
    let mut pending = false;
    for (name, status, conclusion) in runs {
        any = true;
        if status != "completed" {
            pending = true;
            continue;
        }
        if !matches!(conclusion.as_str(), "success" | "skipped" | "neutral") {
            return CiState::Failed(format!("{name}: {conclusion}"));
        }
    }
    match (any, pending) {
        (false, _) => CiState::NoRuns,
        (true, true) => CiState::Pending,
        (true, false) => CiState::Passed,
    }
}

impl GhForge {
    /// The issue's state, as `gh` reports it.
    fn state(&self, n: &str) -> Result<String, ForgeError> {
        #[derive(serde::Deserialize)]
        struct Wire {
            #[serde(default)]
            state: Option<String>,
        }
        let text = gh(&["issue", "view", n, "--json", "state"])?;
        Ok(serde_json::from_str::<Wire>(&text)?
            .state
            .unwrap_or_default())
    }
}

/// Run one `gh` command and return its stdout.
fn gh(args: &[&str]) -> Result<String, ForgeError> {
    let out = Command::new("gh").args(args).output()?;
    if !out.status.success() {
        return Err(ForgeError::Command {
            args: args.join(" "),
            message: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A forge in memory: one fixed issue and the comments posted on it.
///
/// It lives in the library, and not with the tests, because the tests that
/// need it are a separate crate: an integration test cannot see a
/// `#[cfg(test)]` item.
#[derive(Debug, Default)]
pub struct FakeForge {
    /// What `issue` answers, whatever the number asked for.
    pub issue: IssueView,
    /// Every comment posted, oldest first.
    pub comments: Mutex<Vec<String>>,
    /// Whether `close` has succeeded yet.
    closed: AtomicBool,
    /// How many more `close` calls fail before one goes through.
    close_failures: Mutex<usize>,
    /// What `ci` answers, front first; the last answer repeats. Empty
    /// answers `Passed`, and an `Err` carries the stderr of a `gh` that
    /// failed.
    answers: Mutex<Vec<Result<CiState, String>>>,
    /// How many `ci` reads the runner has made.
    ci_calls: AtomicUsize,
}

impl FakeForge {
    /// A forge whose issue is `issue` and which holds no comments yet.
    pub fn new(issue: IssueView) -> Self {
        Self {
            issue,
            comments: Mutex::new(Vec::new()),
            closed: AtomicBool::new(false),
            close_failures: Mutex::new(0),
            answers: Mutex::new(Vec::new()),
            ci_calls: AtomicUsize::new(0),
        }
    }

    /// Script what `ci` answers: each call takes the next answer, and the
    /// last one repeats.
    pub fn script_ci_answers(&self, answers: Vec<Result<CiState, String>>) {
        *self.answers.lock().expect("ci answers mutex") = answers;
    }

    /// Script the CI states `ci` answers, each one a pass of the read.
    pub fn script_ci(&self, states: Vec<CiState>) {
        self.script_ci_answers(states.into_iter().map(Ok).collect());
    }

    /// How many `ci` reads have been made.
    pub fn ci_calls(&self) -> usize {
        self.ci_calls.load(Ordering::SeqCst)
    }

    /// A forge that starts with `comments` already on the issue.
    pub fn with_comments(issue: IssueView, comments: Vec<String>) -> Self {
        Self {
            comments: Mutex::new(comments),
            ..Self::new(issue)
        }
    }

    /// A forge whose issue is already closed, as a crash before the
    /// closing comment leaves it.
    pub fn closed_at_start(mut self) -> Self {
        self.closed = AtomicBool::new(true);
        self
    }

    /// Make the next `times` `close` calls fail, as a forge that is down
    /// for a moment does.
    pub fn fail_close(&self, times: usize) {
        *self.close_failures.lock().expect("close mutex") = times;
    }

    /// Close the issue behind the runner's back, as a crash after the
    /// `gh issue close` landed leaves it.
    pub fn set_closed(&self, closed: bool) {
        self.closed.store(closed, Ordering::SeqCst);
    }

    /// Whether `close` has succeeded.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// What has been posted, oldest first.
    pub fn posted(&self) -> Vec<String> {
        self.comments.lock().expect("comments mutex").clone()
    }
}

impl Forge for FakeForge {
    fn issue(&self, _n: u64) -> Result<IssueView, ForgeError> {
        Ok(self.issue.clone())
    }

    fn comments(&self, _n: u64) -> Result<Vec<String>, ForgeError> {
        Ok(self.posted())
    }

    fn comment(&self, _n: u64, body: &str) -> Result<(), ForgeError> {
        self.comments
            .lock()
            .expect("comments mutex")
            .push(body.to_string());
        Ok(())
    }

    fn close(&self, _n: u64) -> Result<(), ForgeError> {
        if self.is_closed() {
            return Ok(());
        }
        {
            let mut left = self.close_failures.lock().expect("close mutex");
            if *left > 0 {
                *left -= 1;
                return Err(ForgeError::Command {
                    args: "issue close".to_string(),
                    message: "the forge is down".to_string(),
                });
            }
        }
        self.closed.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn ci(&self, _sha: &str) -> Result<CiState, ForgeError> {
        self.ci_calls.fetch_add(1, Ordering::SeqCst);
        let mut answers = self.answers.lock().expect("ci answers mutex");
        let answer = match answers.len() {
            0 => Ok(CiState::Passed),
            1 => answers[0].clone(),
            _ => answers.remove(0),
        };
        answer.map_err(|message| ForgeError::Command {
            args: "run list".to_string(),
            message,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stderr the incident's dropped connection left, word for word.
    const DROPPED_CONNECTION: &str = "dial tcp 140.82.121.6:443: i/o timeout";

    fn run(status: &str, conclusion: &str) -> (String, String, String) {
        ("ci".into(), status.into(), conclusion.into())
    }

    fn gh_failed(message: &str) -> ForgeError {
        ForgeError::Command {
            args: "run list".into(),
            message: message.into(),
        }
    }

    #[test]
    fn a_dropped_connection_is_a_network_fault() {
        for stderr in [
            DROPPED_CONNECTION,
            "dial tcp: lookup api.github.com on 10.0.0.1:53: no such host",
            "dial tcp 140.82.121.6:443: connect: connection refused",
            "read tcp: connection reset by peer",
            "gh: HTTP 503: Service Unavailable",
            "gh: HTTP 429: API rate limit exceeded",
            "warning: a proxy is set\ngh: dial tcp 140.82.121.6:443: i/o timeout",
        ] {
            assert!(gh_failed(stderr).is_network_fault(), "{stderr}");
        }
    }

    #[test]
    fn what_gh_refused_is_not_a_network_fault() {
        for stderr in [
            "gh: Not Found (HTTP 404)",
            "gh: Bad credentials (HTTP 401)",
            "gh: Validation Failed (HTTP 422)",
            "HTTP 200",
            "unknown flag: --commit",
        ] {
            assert!(!gh_failed(stderr).is_network_fault(), "{stderr}");
            let unreadable = serde_json::from_str::<serde_json::Value>(stderr).unwrap_err();
            assert!(!ForgeError::Json(unreadable).is_network_fault(), "{stderr}");
            assert!(
                !ForgeError::Io(std::io::Error::other(stderr)).is_network_fault(),
                "{stderr}"
            );
        }
    }

    #[test]
    fn a_commit_with_no_runs_is_not_a_pass() {
        assert_eq!(fold_ci(Vec::new()), CiState::NoRuns);
    }

    #[test]
    fn an_unfinished_run_is_pending_and_a_finished_failure_fails_first() {
        assert_eq!(
            fold_ci(vec![run("completed", "success"), run("in_progress", "")]),
            CiState::Pending
        );
        assert_eq!(
            fold_ci(vec![run("in_progress", ""), run("completed", "failure")]),
            CiState::Failed("ci: failure".into())
        );
        assert_eq!(
            fold_ci(vec![
                run("completed", "success"),
                run("completed", "skipped")
            ]),
            CiState::Passed
        );
    }
}
