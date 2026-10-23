# This fork

Status: started 2026-10-23 · Tracks `stevewerme/aigentic` (`upstream`)

`carlstrombeck/aigentic-explore` is a fork of
[aigentic](https://github.com/stevewerme/aigentic). It keeps the harness
as it is and aims it at one product goal: **one person runs several
projects in parallel, from one conversation, with the work in each
project done by agents and every decision that matters still a person's.**

Model independence is a precondition, not the goal. The `Provider` trait
and the two adapters already provide it, and this fork adds nothing there.

## What the fork changes, and what it does not

Upstream's documents stay binding: `docs/PRD.md`, the `docs/PLAN-*.md`
files, the ADRs and `AGENTS.md`. The fork adds two things on top:

1. **An order.** Upstream builds the client (phase 6), then sandboxing
   (phase 7), then the portfolio orchestrator (phase 8). This fork builds
   the parallel, cross-project work first, in `docs/PLAN-portfolio.md`,
   and takes phases 6 and 7 from upstream as they land. Multiplayer
   acceptance (phase 5 step 11) and the `ratatui` client are parked here
   until the product needs a second person or a full-screen view.
2. **One plan.** `docs/PLAN-portfolio.md` is the fork's own phase. Where it
   and an upstream plan disagree, it wins in this repository only.
   Everything else follows upstream.

Nothing is renamed. Crate names, the binary, `aigentic.toml`, `.aigentic/`
and `~/.config/aigentic/` stay as upstream has them, so upstream merges stay
cheap. Rename once the fork's code has diverged enough that merging no
longer pays off. That is a deliberate decision to record here, not
something that happens bit by bit.

## Keeping up with upstream

- `upstream` is `https://github.com/stevewerme/aigentic`. Merge
  `upstream/main` into `main` at least weekly, and before starting a
  slice of `docs/PLAN-portfolio.md`. Merge commits only, no rebases, so
  every merge is visible in the history.
- Fork work goes in new files where it can: new modules, new event
  kinds, new plan sections. A change inside an upstream function is
  kept to the smallest seam that works, because each one is a future
  merge conflict.
- A slice that upstream would want (the portfolio orchestrator is in
  upstream's PRD, and ADR 0002 points the same way) is offered upstream
  as a pull request once it works here. Each slice upstream accepts is
  code the fork no longer has to carry.

## Open items

- [ ] **Who the product is for.** "Several projects in parallel" is the
  mechanism. The product is a person with a real portfolio: your own
  repositories, a team's services, or client work across companies (which
  upstream's workspaces model). Pick one before slice 5 of the portfolio
  plan. The by-hand routine in slice 1 is where the answer comes from.
- [ ] **Which models may run on the product path.** Upstream's PRD
  (settled 2026-10-07) puts only open-weight models on sovereign EU or
  local hosts on the product path, and keeps Anthropic as a reference
  backend. The code is model-independent either way. Whether this fork
  keeps that rule or allows any backend is a product decision. Until it
  is made, upstream's rule stands.
