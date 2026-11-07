# Portfolio plan: parallel work across projects

Status: draft for review, 2026-10-23 · Fork-only (`docs/FORK.md`); wins
over upstream plans in this repository where they differ · Follows
`docs/PRD.md` "Orchestrator", ADR 0002, `docs/PLAN-phase6.md` §14 and
`docs/PLAN-layer2.md` · Built on upstream 0.6.0

## 0. Goal and done-when

Upstream 0.6 has most of the parts. A person talks in one front thread
(ADR 0002). Projects are grouped into workspaces. Each project has a
brief whose current state the utility model keeps fresh. `start_job`
runs work in a child thread and returns a summary to the front thread,
and `/jobs` lists the jobs. Four things are missing before one person can
run several projects at once:

- **A job runs only in the front thread's current project.** Work for
  another project means switching the conversation first.
- **Writing jobs share one working tree.** Two jobs that write in the
  same project collide. Layer 2 keeps "one writing step at a time on the
  shared tree" and lists a worktree per build as out of scope.
- **Nothing limits the whole portfolio.** No cap on concurrent jobs and
  no daily spend limit across projects.
- **No view across projects.** A person can't see, in one place, what
  is running, what has finished, what waits on them and what each
  project's state is.

This plan adds those four things and then the PRD's orchestrator tools,
in that order.

Done when:

1. **Parallel across projects.** One message in the front thread, after
   the person approves, starts writing jobs in at least two projects in
   two workspaces, and they run at the same time.
2. **No collisions.** Two writing jobs in one git project each work in
   their own worktree on their own branch. Neither sees the other's
   changes, and the project's checkout is never touched while they run.
3. **The person merges.** A finished writing job leaves a branch and a
   summary. Merging it into the project's branch or discarding it is a
   decision the person answers in one keystroke. Nothing is pushed.
4. **One view.** `aigentic status` and `/status` show, for every
   project: the brief's current state in one line, running and queued
   jobs, branches waiting for a decision, open asks and today's cost.
   The view is a projection of the logs and briefs, with no table of its
   own.
5. **Limits hold.** With `max_jobs` reached, a new approved job queues
   and starts when a slot frees. With the portfolio's daily budget
   reached, no job starts, and the person is asked before a running one
   goes on.
6. **Resume is replay.** Killing the daemon with jobs running, queued
   and waiting on a merge decision, then starting it again, brings back
   every job in the state its log says.
7. **The routine runs from one message.** Two weeks after slice 1, the
   person's morning routine is one message to the front thread. It reads
   the portfolio, proposes the day's jobs, and starts the approved ones.

Not in this plan: scheduled runs while nobody is at the keyboard (slice
8 is gated on slice 1's evidence and ADR 0002's ladder), the sandbox
(upstream phase 7, merged in when it lands), and a second person.

## 1. Where it sits

| Crate | Gains |
| --- | --- |
| `core` | `EventKind` variants (section 7); no new dependency |
| `log` | Payloads for the new kinds; a `PortfolioState` projection over many threads' logs |
| `runtime` | `start_job` takes an optional `project`; a `worktree` module (add, list, remove, merge) beside `checks/git.rs`; the orchestrator tools in slice 7 |
| `policy` | A job in another project is checked against that project's policy, not the front thread's |
| `api` | `Request::Status`, `Request::AnswerMerge { .. }`; `Notice::Portfolio { .. }` |
| `server` | The job scheduler (slots, queue, daily budget); worktree roots passed to `create_job`; resume of queued jobs |
| `tui` | `aigentic status`, `/status`, the merge prompt, queued jobs in `/jobs` |

No crate gains a dependency edge. `ThreadRegistry::create_job` already
takes the job's `root` separately from its project, so a worktree is a
different `root` with no new seam.

## 2. Slices, one ticket each

Each slice ends with something used daily before the next starts, as the
PRD's build order asks.

### Slice 1: run the orchestrator by hand (no code)

The PRD says to run the orchestrator role by hand before automating it.
This slice does exactly that.

- Set up two workspaces and at least three real projects, each with a
  written brief. `scripts/routine-setup.sh` does the setup, and
  `docs/routine/README.md` is the guide.
- Every morning for two weeks: read each brief
  (`scripts/routine-briefs.sh`), decide the day's work, and start it as
  jobs (`start_job` from inside each project today). Note what you read,
  what you decided and what you had to look up.
- Keep the notes in the portfolio's own project, in
  `.aigentic/knowledge/routine-log.md`.

Output: the status line format a project needs, the five or so
questions the routine asks every day, and the first answer to
`docs/FORK.md`'s "who the product is for". Slices 5 and 7 are written
from these notes, not from this plan's guesses.

### Slice 2: a job in another project

- `start_job` takes `project`, one of the projects in reach. When it is
  absent, the job runs in the current project, as it does today.
- The job runs with that project's root, instructions, knowledge and
  policy. The proposal names the project, and the person approves the
  project and the job together.
- A job's questions still come to the front thread, tagged with the
  job and its project.

### Slice 3: a worktree per writing job

- In a git project, a writing job gets
  `git worktree add <project>/.aigentic/worktrees/<job> -b job/<job>`,
  branched from the project's current `HEAD`, and that worktree is its
  `root`. A `read_only` job keeps the project root.
- In a project that is not a git repository, a second writing job is
  refused while one is running, and the refusal says why.
- When a job finishes, its worktree is kept, the branch keeps its
  commits, and the front thread gets a `merge` decision proposal (ADR
  0002 point 2) with the diff stat and the job's summary.
- `y` merges the branch into the project's checked-out branch (a merge
  commit, never a rebase) and removes the worktree. `n` keeps the
  branch. Discarding a branch is deleting, so it is a separate prompt
  that never leaves *ask* (ADR 0002 point 4).
- A merge conflict is never resolved by a model in the project's
  checkout. The proposal says the branch conflicts and offers a job
  that merges the project's branch into the job's branch, in the
  worktree.
- `.aigentic/worktrees/` is gitignored by `aigentic project init` and
  checked by `aigentic doctor`.

### Slice 4: slots, the queue and the daily budget

- `[portfolio] max_jobs` (default 3) in the global config caps the
  number of jobs running at once across every project. Builds count as
  jobs.
- An approved job beyond the cap is queued (`job_queued`) and starts
  oldest first when a slot frees (`job_dequeued`).
- `[portfolio] daily_budget_usd` caps the spend across all projects,
  counted by `aigentic stats` from the logs. Reaching it stops new jobs
  from starting and asks before a running job takes its next turn.
  Spending past a budget never leaves *ask* (ADR 0002 point 4).

### Slice 5: one view

- `aigentic status` (scriptable, plain text) and `/status` in the REPL
  print one block per workspace, with one line per project and one line
  per running, queued or waiting job under it.
- Everything comes from `PortfolioState`, a projection over the front
  thread, the job threads and the briefs, cached in memory and rebuilt
  from the logs on start.
- The line format comes from slice 1's notes.

### Slice 6: resume

- On start, the daemon replays `job_queued`, `job_dequeued`,
  `job_finished` and the `merge` proposals, re-registers worktrees it
  finds in the log, and restarts unfinished jobs the way layer 2
  restarts an unfinished run.
- A worktree on disk that no log names is reported by `doctor` and
  never deleted automatically.

### Slice 7: the orchestrator's tools, in the front thread

ADR 0002 puts one conversation in front, so the orchestrator is the
front thread with more tools, not a separate thread. The PRD's rule
stays: it never reads project threads.

These tools cross a boundary upstream keeps on purpose. A workspace is
the boundary for understanding (phase 6 §14), so `read_brief` refuses a
project in another workspace, and a thread sees other workspaces only as
one line each. The orchestrator has to read briefs in every workspace.
The proposal: a thread in a project marked `[portfolio] reads = "briefs"`
may read every workspace's briefs and `PortfolioState`, and nothing
else. Files, threads and knowledge stay inside the boundary. This needs
upstream's view before it is built, because it changes a rule upstream
decided.

- `list_projects`: workspaces, projects and each brief's one line.
- `read_status(project)`: the brief and that project's lines from
  `PortfolioState`.
- `post_task(project, task, brief)`: `start_job` across projects, under
  the same decision.
- `await_result(job)`: parks until the job's `job_finished`, then
  returns its summary.
- `ask_human` exists already.

The tool set is checked against slice 1's notes before it lands. A
question the routine asked every day that no tool answers is a missing
tool.

### Slice 8: the routine on a schedule (gated)

This slice only starts when slice 1's notes show the routine was the
same on most days. Then the daemon proposes it as a scheduled routine,
the *scheduled work* kind in ADR 0002 point 6, starting at *ask*. Even
when the kind is promoted, the scheduled run only proposes jobs. It does
not start one without the slot, budget and approval rules above.

## 3. The decisions and what never leaves *ask*

| Decision | Kind | Starts at | Can be promoted |
| --- | --- | --- | --- |
| Start a job in another project | `job` | ask | yes, per ADR 0002 point 3 |
| Merge a finished job's branch | `merge` (new) | ask | yes |
| Discard a job's branch | — | ask | never (deleting) |
| Start or continue past the daily budget | — | ask | never (spending) |
| Push a job's branch | — | ask | never (push beyond a workflow) |

## 4. Events

Added, never changed, per `AGENTS.md`:

- `job_queued`: in the front thread. The approved job's proposal event,
  its project and its position. Written before the wait.
- `job_dequeued`: in the front thread. The queued job now has a slot and
  a thread id.
- `worktree_added` / `worktree_removed`: in the job's thread. The path,
  branch and base commit, so resume and `doctor` know every worktree the
  harness made.
- `merged`: in the front thread, after the `merge` decision's answer.
  The branch, the merge commit and the project branch before and after.
- `budget_warned` already exists (layer 2). The portfolio budget reuses
  it with a new `scope` value.

`merge` is a value of the existing decision kinds
(`decision_proposed` / `decision_answered`), not a new event kind.

## 5. Risks

- **Merge conflicts with upstream.** Slices 2, 3 and 4 touch
  `harness_tools.rs` and `server/src/threads.rs`, which upstream changes
  often. Keep each change at the smallest seam, merge upstream before
  each slice, and offer slices 2 to 4 upstream (`docs/FORK.md`).
- **Unattended jobs with write access.** Several jobs at once means more
  that nobody watches. The step deny list (no push, reset, rebase or
  `.env`) applies to every job. Sandboxing arrives with upstream phase
  7, and until then `max_jobs` stays small.
- **Cost.** Parallel jobs spend in parallel. The daily budget is slice 4,
  ahead of the view and the tools.

## 6. Open items

- [ ] Whether upstream accepts a portfolio project reading briefs across
  workspaces (slice 7). Ask before slice 7 starts.
- [ ] Whether `max_jobs` should be per workspace as well as global.
  Decide from slice 1's notes.
- [ ] Whether the worktree directory belongs in the project
  (`.aigentic/worktrees/`) or beside it, outside the repository (better
  for editors and file watchers). The default above is inside, because
  `doctor` and `project init` already own `.aigentic/`.
- [ ] Which model runs the front thread once it carries the
  orchestrator's tools. The PRD gives the orchestrator the strongest
  model, but the front thread also runs ordinary conversation. Possibly a
  `route` decision (ADR 0002) per turn.
