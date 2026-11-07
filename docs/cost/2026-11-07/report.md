# Cost levers, 2026-11-07

Three levers from Lance Martin's talk (AI Engineer, 6 Nov 2026), tried on aigentic with open models on TensorX: cache order, a prompt audit, and effort. The data: the 160 newest loop threads in `~/.local/share/aigentic/threads/` (15 to 25 Oct, 11,688 requests, $141), probes against TensorX, and a rerun of the 26 Oct bench (`.scratch/bench-flash-2026-10-26.md`).

## Short version

- **Cache order is already right.** Context goes global, harness, workspace, project, knowledge, memory, pins, skills, then the body (`crates/runtime/src/context.rs:72`). Tool specs are sorted by name (`crates/tools/src/registry.rs:153`). Nothing per turn comes ahead of the stable part. No code change.
- **TensorX does cache Flash prefixes**, and the log's `cost_usd` matches the $0.125 cached price exactly. Flash hits 93%, GLM 5.3 79%. Most of the misses are on TensorX's side: a call loses its prefix at random, unrelated to idle time or to anything we change. `prompt_cache_key`, `user` and `x-session-id` make no difference.
- **Effort is the biggest lever measured.** With effort off, Flash cost 3 to 4 times more on bench A and C, made 2 to 5 times as many tool calls, and scored worse. Medium was the cheapest and the best.
- **The prompt audit found no old habits** ("think step by step", "double-check", "be thorough", scratchpads). It found roles re-reading files that are already in their context, one contradiction, and one duplicated step. Fixing them saves about 1% of loop spend. A bigger saving, 5% of spend on `update_tasks`-only turns, is in harness code and is only proposed here.
- **Most expensive role: the spec check**, at $2.11 a run on GLM 5.3 (29% of loop spend in 17 runs). That is where to hill-climb.

## 1. Cache

### Hit rate by role (160 threads, before)

| Role | Model | Requests | Hit rate | Cost |
| --- | --- | --- | --- | --- |
| spec | Flash | 1,825 | 91.3% | $16.57 |
| speccheck | GLM 5.3 | 950 | 79.9% | $35.87 |
| decider | GLM 5.3 | 359 | 77.3% | $14.02 |
| merger | Flash | 536 | 89.3% | $4.70 |
| implement | Flash | 1,956 | 94.8% | $16.79 |
| judge | Flash | 5,368 | 93.2% | $27.54 |
| fix | Flash | 163 | 95.1% | $1.03 |
| **all Flash** | | | **93.1%** | $91.44 |
| **all GLM** | | | **79.2%** | $49.61 |

Hit rate = cached / (cached + uncached) input tokens. In the log, `input_tokens` is the uncached part: 8,418 uncached + 4,608 cached + 263 out at Flash prices is exactly the logged $0.0051795.

### Where the uncached tokens come from

Each call's uncached tokens are split into growth (new messages since the last call: unavoidable), a break right after our own eviction (`context_evicted`: on purpose, spaced by `evict_min_free_percent`), and every other break.

| | Flash | GLM 5.3 |
| --- | --- | --- |
| Uncached input | 38.2M | 12.2M |
| Growth | 10.5M | 2.7M |
| After our eviction | 4.5M | 0.8M |
| Other breaks | 23.3M | 8.7M |
| Calls that lose more than half the prefix | 4% | 16% |
| Cost of other breaks | $8.7 (9.5% of Flash) | $11.4 (23% of GLM) |

The other breaks are not ours:

- A GLM decider call at seq 13 read 0 cached tokens straight after one that read 29,184, with 1,113 tokens of growth. Nothing in the builder changes the first block between two calls.
- Breaks don't follow idle time. Only gaps over 5 minutes raise the rate (Flash: 4% of calls under 10 s, 19% over 300 s).
- Probe, 8k-token prefix, sequential calls, a fresh prefix per arm:

| Model | No key | `prompt_cache_key` | `user` | `x-session-id` |
| --- | --- | --- | --- | --- |
| Flash, 30 calls, 8 arms in parallel | 1 drop, 93.3% | 1, 93.9% | 1, 94.7% | 1, 93.7% |
| GLM 5.3, 30 calls, in parallel | 4, 83.2% | 3, 85.2% | 2, 90.0% | 2, 90.6% |
| GLM 5.3, 40 calls, 2 arms | 1, 96.6% | | 3, 91.2% | |

  No routing key helps. Drops rise under parallel load. That fits replicas that don't share a cache.

**After:** no code change, so the loop's hit rate is unchanged (93.1% Flash, 79.2% GLM). The bench's own threads hit 78 to 98% (table below). What can still move it: fewer, larger requests on GLM, and keeping roles off GLM, whose drops cost 3.5 times as much per token.

## 2. Prompt audit

Before: `docs/cost/2026-11-07/templates-before/`. After: `.aigentic/workflows/loop/templates/`. `git diff` shows each change.

Not found in any role template, the harness text or the global instructions: "think step by step", "double-check your work", "be thorough", or a scratchpad request.

Found and changed:

1. **Re-reading what is already in context.** `AGENTS.md` (the project instructions) and `.aigentic/knowledge/*.md` (25 KB, well under 40% of the window) are inlined into every role's prefix. The first call of each role already weighs about 13.5k tokens. Yet the templates told spec, speccheck, decider, merge and judge to read `lessons.md` and `build-loop.md`, and told every role to follow `AGENTS.md`. Speccheck read them 75 times in 17 runs, deciders 31 times, judges grepped `AGENTS.md` 96 times. Each template now says they are in context and not to read them again. Measured cost of those reads: $1.03 of $122 (decider 3.3%, speccheck 1.3%).
2. **Contradiction in speccheck.** "Every fact about the code … each `file:line`" against "a proportionate check beats an exhaustive one". Now: "the facts the design rests on", with the files read only as far as the checks need.
3. **The decider re-checking the spec check.** "Every file the spec cites" duplicated the speccheck's job. Now: the `file:line` each check item cites, plus the code an open choice turns on.
4. **Duplicated scratchpad.** `implement.md` repeated the harness's `update_tasks` instruction. Removed.
5. **Emphasis.** The judge's bold "**prove by running, not by reading.**" is now plain text. The safety rules keep their bold.

Not changed (outside the templates, Steve's call):

- `HARNESS_INSTRUCTIONS` (`crates/runtime/src/harness_tools.rs:96`) asks for `update_tasks` before anything else and again as each step starts and finishes. Requests whose only tool call is `update_tasks`: 480 of 11,688 (4.1%), **$7.14 of $141 (5.1%)**. In a workflow step nobody reads that list live.
- `~/.config/aigentic/instructions.md`: "Say what you are about to do in a line before doing it" adds output tokens before every tool call.

The runtime's runner tests (which load and render the loop) pass: 135 of 135.

## 3. Bench rerun

Same tasks and answer keys as 26 Oct, DeepSeek V4.1 Flash 1017 only, through `aigentic exec` with a separate config and threads dir, in this worktree (`main` at `ed7289d`). A to C don't use the loop templates. They run on the same prefix (global instructions, harness, `AGENTS.md`, knowledge), so the audit doesn't touch them. The differences below are drift and effort, not the audit.

The first run had no `reasoning_effort` set, where 26 Oct had medium. I reran at medium for the comparison and kept the first run as the effort data point.

| | 26 Oct, medium | 7 Nov, medium | 7 Nov, effort off |
| --- | --- | --- | --- |
| A cost per run | $0.054, $0.048 | $0.042, $0.033 | $0.108, $0.281 |
| A tool calls | 17, 11 | 14, 9 | 28, 75 |
| A score | both correct, both traps | both exact, both traps | both correct, level 2 padded with level-3 sites |
| B titles | 5 of 5 good | (not rerun at medium) | 5 of 5 good |
| C cost per run | $0.46, $0.31 | $0.45, $0.08 | $0.65, $0.98 |
| C tool calls | n/a | 80, 21 | 126, 187 |
| C verdict | changes, changes | changes, changes | changes, changes |
| C: protocol bump (blocking) | missed, found | missed, **found** | flagged non-blocking, missed |
| C: doc comment on the wrong test | found, missed | found, **found** | missed, missed |
| C: key items found | 2 of 4 | **3 of 4** | about 0.5 of 4 |
| C: extra live bug (a yes to a job refused) | found once | not found | found once |
| Cache hit, A / C | n/a | 87%, 78% / 95%, 90% | 93%, 93% / 96%, 98% |
| Reasoning tokens, C | n/a | 28k, 12k | 0, 0 |

Reading: with two runs per cell, the spread is large (the two medium C runs differ fivefold in cost). Medium is at least as good as 26 Oct and no dearer. Effort off is clearly worse: the model reasons in tool calls instead of thinking, and every tool call re-reads the whole context. The live config already sets `reasoning_effort = "medium"` for `[profiles.flash]`, and TensorX takes it again: a probe today answered with medium, high and none.

Spend today: bench $2.65 (effort off $2.04, medium $0.61), cache probes about $1 (estimated from token counts). About $3.65 in all, against $2.70 on 26 Oct. The effort-off run is the overrun.

## 4. Proposal: a cost hill climb for the spec check

Not built. The spec check is the role to climb. It costs $2.11 a run on average (GLM 5.3, $35.87 in 17 runs), has the worst hit rate (79.9%), and its quality is easy to score: the merger already records which of its items held.

- **Knobs:** model (GLM 5.3, Flash 1017, Kimi K3), `reasoning_effort` (low, medium, high), prompt (before, audited).
- **Tickets:** about 10 closed loop tickets that have a `## Spec`, a `## Spec check` and a `## Spec v2 (consolidated)`, for example #199, #200, #202, #216 and #218 from this log window, plus earlier ones. Split them by ticket number, not at random: **train** 6, **test** 4, all test tickets newer than all train tickets.
- **Answer key per ticket:** the check items Spec v2 merged (true positives), the ones it rejected (false positives), and any judge finding that a spec gap caused (misses). Built once, by hand or by the decider, and frozen before the climb.
- **Score:** recall of merged items minus a penalty per rejected item, then cost per run. A config is better only if it is no worse on score and cheaper, or better on score at the same cost.
- **Climb:** coordinate ascent from today's config (GLM 5.3, no `reasoning_effort` set, as in `[profiles.tensorx]`, audited prompt). Change one knob at a time on the train set, one run per ticket, and keep a change only when it wins on the rule above. About 5 configs × 6 tickets = 30 runs.
- **Test:** the winner against today's config on the 4 test tickets, 2 runs each: 16 runs.
- **Cost:** a spec check today is $0.5 to $3.5. On Flash it would be a fraction of that. Expect $20 to $40 in all, so it needs Steve's yes first. A cheaper first step is test tickets only, today's config against Flash at medium (8 runs, about $8).
- **Run it the bench way:** `aigentic exec --config <bench>.toml` in a worktree checked out at each ticket's base commit, with its own threads dir and nothing posted to the issue.

## 5. First step run: spec check, GLM 5.3 against Flash at medium

The four newest closed tickets that have a Spec, a Spec check and a Spec v2: #199, #200, #216, #218. One run per ticket per config, 8 runs.

- **Configs.** `glm`: GLM 5.3 with no `reasoning_effort`, which is what `[profiles.tensorx]` runs today. (Section 4 said "effort high". That was wrong: the profile sets none.) `fl`: DeepSeek V4.1 Flash 1017 at `reasoning_effort = "medium"`.
- **Setup.** A detached worktree at the last `main` commit before the ticket's real spec check. The audited `speccheck.md` rendered for the ticket, with two changes made to both arms alike: the issue and the spec come from files, so the later Spec v2, review and fix comments can't leak, and the report is the last message, since `exec` has no `finish_step`. `aigentic exec --mode auto`, a separate config and threads dir, nothing posted.
- **Key, frozen before any output was read.** The items of the real Spec check that Spec v2 merged, and the ones it rejected (only #200's item 11). Judge misses traced to a spec gap: none. #199, #200 and #218 were approved on the first review, and #216 has no review comment. Score: key items found, with #200's three line-number items counted as half each, and minus one for a claim v2 rejected.

| Ticket | Key | GLM 5.3 | Flash, medium |
| --- | --- | --- | --- |
| #199 | 3 | 1 (T1's order) | 0 (six other items, none in the key) |
| #200 | 8.5 | 1.5 (title, `Other` row, half for line fixes; minus 1 for re-raising rejected item 11) | no report |
| #216 | 5 | 2 (t9 shape, T8's constant) | no report |
| #218 | 4 | 3 (missed the invented issue quote) | 4 |
| **Found** | 20.5 | **7.5 (37%)** | **4 (20%)**; 4 of 7 on the two tickets it reported on, the same as GLM |

| Config | Cost per run | Mean | Requests | Cache hit | Wall time |
| --- | --- | --- | --- | --- | --- |
| GLM 5.3 | $2.50, $2.79, $2.24, $1.51 | **$2.26** | 44 to 92 | 81.5% to 89.6% | 13 to 15 min |
| Flash, medium | $0.29, $0.54, $0.34, $0.23 | **$0.35** | 25 to 72 | 88.5% to 93.9% | 4 to 7 min |

The real loop spec checks of these tickets cost $1.81, $3.02, $2.26 and $1.34, so the GLM arm reproduces today's cost. Spend: $10.43.

Reading:

- **Flash is 6.5 times cheaper, but it failed to report on 2 of 4 tickets.** On #200 and #216 its last message held reasoning and no text and no tool call, and the turn ended as `done`. Both runs had already confirmed findings in their reasoning. In the loop, such a step parks at `step_stop` ("done without a report") and costs a retry. That is a harness behaviour worth fixing whatever the model: a reply with reasoning only should not end a turn.
- **The key is weak.** It is one earlier GLM run's findings, and today's GLM run found only 37% of them. Both arms raised other items that look real and are not in the key. Run-to-run variance is as large as the gap between the models, so one run per cell can't tell them apart on quality.
- **Not yet a reason to switch.** The cost gap is clear and large. The quality comparison needs the empty-turn fix first, then 2 runs per cell, with the out-of-key items checked by hand.

## Files

- `.aigentic/workflows/loop/templates/*.md`: the audited templates.
- `docs/cost/2026-11-07/templates-before/`: the templates as they were.
- Analysis scripts and bench outputs are in the session scratchpad and were not committed. Ask if you want them in the repo.
