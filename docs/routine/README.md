# The by-hand routine

Slice 1 of `docs/PLAN-portfolio.md`. For two weeks you play the
orchestrator yourself, every morning, across your real projects. You
build nothing in this slice. What you produce is the log, and the log
decides what slices 5 and 7 build.

## Set up (once, about 30 minutes)

1. **Clone this fork and install the binary.** The scripts live here.
   The fork's code is still upstream 0.6.0, so an upstream release
   binary works just as well, but building from here keeps the binary
   in step once the fork's code slices land.

   ```bash
   git clone https://github.com/carlstrombeck/aigentic-explore && cd aigentic-explore
   cargo install --path crates/tui --force --locked
   ```

2. **Configure a model.** Any OpenAI-compatible endpoint or Anthropic
   works. The top-level `README.md` has the `config.toml` shape. Then:

   ```bash
   aigentic doctor --probe
   ```

3. **Pick the projects.** Use two workspaces and at least three projects
   you actually work on. A workspace is a group that belongs together,
   such as "personal" and one client. The folders must exist; git
   repositories are best.

4. **Run the setup** from this repository's root:

   ```bash
   scripts/routine-setup.sh ~/Projects/portfolio \
     personal=~/Projects/site \
     personal=~/Projects/api \
     acme=~/Clients/acme-app
   ```

   It creates the workspaces, initialises or lists each project, writes
   `.aigentic/brief.md` from `docs/routine/brief.md` where none exists,
   and makes `~/Projects/portfolio`, which holds the log. It never
   overwrites a file, so it is safe to run again with more projects.

5. **Write the briefs.** In each project, fill in
   `.aigentic/brief.md`: the first line, `## Foundation` and
   `## Current state`. Keep each brief under 2,000 bytes, which is what
   goes into the model's context. The first line is what other projects
   see.

## Every morning

1. `scripts/routine-briefs.sh`: every project's first line and current
   state, across all workspaces.
2. `aigentic status`: what is running right now.
3. Decide the day's work for each project.
4. Start it. For each project: `cd` into it, run `aigentic`, and ask for
   the work. When the work is too big for the conversation, the model
   proposes a background job (`start_job`), and `y` starts it. `/jobs`
   lists running jobs.
5. Log the morning in `~/Projects/portfolio/.aigentic/knowledge/routine-log.md`,
   in the format at the top of that file. Do this while it is fresh; ten
   minutes is enough.

## Friction you will hit, and should log

These are known gaps. Each one is a slice of the plan, and the log says
how much each one costs you.

- **Jobs run only in the current project.** You have to `cd` or
  `/project use` before starting work elsewhere. This is slice 2.
- **Two writing jobs in one project share its working tree.** Don't run
  two at once in the same project. Log every time you wanted to. This is
  slice 3.
- **No one view.** `routine-briefs.sh` and `aigentic status` are two
  separate views, and neither shows jobs that are waiting on you. This is
  slice 5.
- **A thread can't read briefs in other workspaces.** That is why the
  briefs come from a script and not from the model. This is slice 7, and
  it is an open question with upstream.
- **Briefs go stale.** The utility model refreshes `## Current state`
  after a batch of file writes, but only from what happened in that
  project's threads. Log when a brief was wrong.

## After two weeks

Fill in the review at the bottom of the log. Then:

- The status line and the daily questions go into slices 5 and 7 of
  `docs/PLAN-portfolio.md`, replacing the plan's guesses.
- "Who this is for" goes into `docs/FORK.md`.
- If the routine was the same most days, slice 8 is in play.
- Start on slice 2.
