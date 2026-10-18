# Build order

The phases maitred is built in, what each must show before the next relies on it, and how far each has got.
This file is hand-edited, not synced from the architecture doc: update a phase's status in the same commit as
the work that changes it. Every ticked item names the commit or file that shows it; an item nobody has checked
stays unticked.

> Moved out of ARCHITECTURE.md (Nick, 2026-10-18) so the plan and its progress live in one versioned place.
> Status last checked against the repo 2026-10-18, at `aacfeb4`.

## What fixes the order

1. **Spikes come first.** The design's "to verify" list decides parts of the architecture, so it is phase 0, and
   each spike names the decision it unblocks.
2. **Observe-only is on the critical path.** The machine config's numbers come from a week of the sampler
   running observe-only. That week starts as soon as the sampler, a minimal supervisor and `decide` exist
   (phase 4), and runs while admission and the kill policy are built.
3. **The work machine goes first, the laptop second, both in v1** (Nick, 2026-10-15). The work machine's shared
   test container is what's holding work up now, so admission, slot pools and the container recipe come
   straight after supervision. The laptop needs the kill policy before it can retire `heavy`, because its two
   resets happened there.

Before phase 1: round 4 applied, and Nick's sign-off on this order. Both done 2026-10-15: round 4 in `4a3d2eb`, the sign-off by Nick.

## Where it stands

| Phase | Status |
| --- | --- |
| 0. Toolchain and spikes | done, except two spikes with no write-up found |
| 1. The pure core and the simulator | core done; `maitred-sim` not started |
| 2. The store | not started |
| 2b. The test kit (MitosisClaude) | in progress |
| 3–10 | not started |

## 0. Toolchain and spikes

**What:** `rust-toolchain.toml` pinned, `rust-version` 1.89+ with a CI job at exactly that version; a virtual
workspace under `crates/` of `maitred-core`, `maitred`, `maitred-sim`, `maitred-scenario` and `maitred-testkit`
(the last two empty, for the test kit's owner); `maitred-core` lint-forbidden from `unsafe`, files, processes,
clocks, environment and network (`disallowed-methods`, `disallowed-types`); clippy pedantic with `-D warnings`
in CI; `deny.toml` (permissive licences only, crates.io only, no duplicate versions or wildcards, bans on every
crate Packages rejected, RustSec advisories); `Cargo.lock` committed and a recorded dependency budget;
cargo-nextest, cargo-llvm-cov with a coverage gate on the core, cargo-mutants on the core, cargo-machete;
`bin/check`, which CI runs too, refusing bare duration literals outside the config module; CI on macOS (Apple
Silicon), Linux, and Linux containers with and without limits.

Spikes: self-measurement inside the agent sandbox; a per-interval CPU pressure figure on macOS without root; GPU
without root; the harness's background-call limits and whether a session's end kills detached children;
`docker compose exec` passing TERM and INT; the work app's `TEST_ENV_NUMBER` conventions; `/proc/locks` listing
each flock with its inode on the CI runner; whether macOS file events coalesce; flock across a Docker Desktop
bind mount (not needed for v1).

**Done when:** `bin/check` green on the empty workspace in CI; each spike's result written down with its
decision.

**Status:**

- [x] Toolchain, workspace, lints, `deny.toml`, `bin/check` (`ddcde79`)
- [x] Coverage gate on the core: lines, regions and functions in `bin/check`, branches in
      `bin/coverage-branches` (`3564c67`)
- [x] cargo-mutants on the core, only inside a memory-capped container: `bin/mutants` (`3564c67`)
- [x] CI on macOS, Linux and Linux containers, green from its first run (2026-10-18)
- [x] Spikes written up in maitred-research `spikes/`: self-measurement, macOS CPU pressure, GPU readings,
      harness limits, `docker compose exec` signals (and its broken pipe), `/proc/locks`
- [ ] Spike: the work app's `TEST_ENV_NUMBER` conventions (no write-up in maitred-research found)
- [ ] Spike: whether macOS file events coalesce (no write-up in maitred-research found)
- [ ] Spike: flock across a Docker Desktop bind mount (not needed for v1)

## 1. The pure core and the simulator

**What:** types; `decide`, `choose_victim`, the state machine; class-file parsing and argv matching; the hook's
spotter; OTLP-JSON types; `maitred-sim`, taking the test kit's scenarios.

**Done when:** the `decide`, `choose_victim`, spotter and starvation rows of Testing; no surviving mutants in
the core.

**Status:**

- [x] `decide` (`decide.rs`), `choose_victim` (`victim.rs`), the state machine (`lifecycle.rs`)
- [x] Class files and argv matching (`classes.rs`, `matching.rs`), including sudo/doas and wrappers after a
      launcher (`baf93e3`, `2f5a412`, `7abad3d`)
- [x] The hook's spotter (`spotter.rs`), including scripts a shell builds (`e8ae0a6`)
- [x] OTLP-JSON types (`otel.rs`)
- [x] Every line, region, function and branch of the core runs in a test (`3564c67`)
- [x] Mutants on every change so far: none missed
- [ ] A whole-crate mutants run with none missed (`bin/mutants --all`, or CI's weekly run)
- [ ] MT-046, which `maitred-sim` needs: a sample copies each job's latest figure whatever its age, its age
      being the sample's time less its holder's pass (item 2, Nick 2026-10-18: doc and a `Load.jobs` comment,
      no new field); the order among equally-over candidates (item 1) and the jitter's direction (item 3)
- [ ] `maitred-sim`, taking the test kit's scenarios, with the `decide` micro-benchmark (C1 in the performance
      plan). The scenario DSL and the `Publisher` it reads load from are in `maitred-scenario` (phase 2b);
      nothing in the test kit holds it up
- [ ] The starvation row of Testing, on the simulator

## 2. The store

**What:** the ticket file, its header and slots; locks with inode re-checks; requests, use files, stop markers,
slot files; project files and exclusive records; the debug-only pause points and audit entries.

**Done when:** the ticket file and Locks and liveness rows.

**Status:** not started. (The header's encoding is settled in the core, `ticket.rs`.)

## 2b. The test kit

Owned by MitosisClaude. Checked against the repo 2026-10-18, at `7b190c2`.

**What:** `maitred-scenario` and `maitred-testkit` (Test kit): the scenario DSL, the runner for both targets,
`burn`, `replay`, the pause-point controller, real error sources, the invariant checker and its audit
cross-check, the findings list, and their scenarios.

**Done when:** every finding of rounds 2 to 4 has a line in `FINDINGS.md`, and the counts add up; the checker
catches every seeded violation; `burn` and `replay` pass their own tests; every regression scenario runs and
fails on its named assertion, not on a panic or a missing binary (most fail only because the phase they cover
isn't built yet: this proves the scenario runs, not that it would catch the bug).

**Status:**

- [x] `burn`, with tests that wait for events rather than times (`ee4ac07`, `aacfeb4`)
- [x] The pause-point controller (`maitred-testkit/src/pause.rs`, `f447145`)
- [x] The scenario DSL (`maitred-scenario`): jobs, load samples, faults, expectations and `validate`, with a
      job's figure taken from its holder's last pass (`2f8731e`)
- [x] Faults at pause points, as scenario steps (`maitred-scenario/src/fault.rs`, `3ceaabf`); carrying them out
      on real processes waits on the runner
- [x] The invariant checker (`maitred-testkit/src/checker.rs`, `2c17d62`, `906ca0f`), with its tests in
      `tests/checker.rs`
- [ ] The checker's audit cross-check (ARCHITECTURE.md:744): not built; maitred writes no audit entries until
      the store (phase 2)
- [x] The findings list (`maitred-testkit/FINDINGS.md`): 228 findings, and the counts add up
      (`tests/regressions.rs`, `the_findings_lists_counts_match_its_lines`)
- [x] Phase 1's regression scenarios (`src/regressions/phase1.rs`), each valid and naming its finding
      (`tests/regressions.rs`)
- [ ] Every regression scenario runs and fails on its named assertion: waits on the runner
- [ ] MT-048: the near-tie scenario, reshaped to fail without the victim file (`54090ed`); its seed sweep
      waits on the simulator
- [ ] `replay`: only its `load/now.json` writer exists (`maitred-testkit/src/load_file.rs`, `8c40286`)
- [ ] The runner for both targets

## 3. Platform and the sampler

**What:** clocks and sleep detection; measuring a session on both platforms; sampling the machine and cgroups;
`watch`, `now.json`, the history.

**Done when:** Measurement and the sampler half of Platforms, and the scenario that a recorded trace replayed
gives the same admissions.

**Status:** not started.

## 4. Observe-only

**What:** `install machine`; a supervisor that runs jobs at once through a plain child process and records what
`decide` would have said; `status`, `result`, `wait`; the pass-through without a machine root.

**Done when:** **the data week starts** on the laptop and, if allowed, the work machine.

**Status:** not started.

## 5. Supervision

**What:** the holder (parked at enqueue, the gate socket, reports, slots, signal reset, subreaper, `leftovers`,
the drain); signals; `submit`, `cancel`, `update`, nesting; orphans, adoption, the sweep; pipes for output and
input; `wait --events`; `guard`.

**Done when:** Supervision, Input and output, Class files and the guard.

**Status:** not started.

## 6. Admission and slot pools

**What:** the admission loop, settle, aging, the fallbacks; slot pools; the machine config from the observe-only
week through the simulator.

**Done when:** Slot pools; starvation against real processes.

**Status:** not started.

## 7. The work machine

**What:** the container recipe (`install machine --container`, the sampler from the entrypoint, a `test_env`
pool sized to the container's cache databases and ports); the work app's class file with its `queue:` line and
guard lines; the first cut of the skill; maitred replaces the work plugin's suite guard, with a way back until a
week runs clean.

**Done when:** a week of the work machine's suites without a collision or a refusal loop.

**Status:** not started.

## 8. The kill policy

**What:** holder passes, `choose_victim` against the frozen sample, stop markers, the dwell and its bounds,
`holder lost`.

**Done when:** the kill scenarios in the test kit, then the laptop's own heavy jobs.

**Status:** not started. (`choose_victim` itself is phase 1's.)

## 9. The agent package

**What:** the Claude Code plugin (skill, refuse-only hook, scripts); `install claude-code` for the sandbox
allowlist and write-deny entries, which a plugin can't set; an AGENTS.md snippet; release binaries via
cargo-dist, a Homebrew tap, `cargo install`; launchd and systemd units.

**Done when:** Inside an agent sandbox, Agent conformance.

**Status:** not started.

The skill's rules for agents: call `maitred -- <program> <args>` as a background call and pipe outside it;
follow a job with `wait <id> --events` under the harness's line-per-event watcher (the docs don't promise a
notification when a background call ends); read `status --json` once for a decision and never poll it; when the
hook refuses, run exactly the command it names; read the `maitred:` line, not the exit number; never retry a
kill blindly.

## 10. The laptop

**What:** gravedigger and its research repo move from `heavy` to maitred (guard lines in their heavy scripts,
their class files), then `heavy` is retired.

**Done when:** a week with no `heavy` call and no reset.

**Status:** not started.

## Not in v1

Terminal mode; GPU reservations; named resources beyond slot pools; the containment install step; suspending jobs
instead of killing them; a queue shared between a host and its containers.
