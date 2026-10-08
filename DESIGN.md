<!--
  Synced from the maitred design doc on claude.ai (rev 519, 2026-10-07):
  https://claude.ai/code/artifact/b0753fb5-a907-459b-8266-2b18f0401e5b
  Draft and revise there; this file is regenerated from it, so edits made only here are lost.
  The architecture drawing is docs/architecture.svg, exported from the doc's drawing.
-->

# maitred design

2026-10-07 · Nicholas Allen Bickford Marshall

maitred (pronounced "mate-urd") is a queue that only permits or defers jobs and records how each run went; the job always executes inside the process that enqueued it. It is for one machine shared by many independent workers, typically AI coding agents each in its own sandbox, that each want to run something heavy: a build, a mutation test, a benchmark, a data load.

## Terms

| Term | Meaning |
| --- | --- |
| worker | A process that asks maitred to run something: an agent session, a script, a person at a terminal |
| job | The command a worker wants to run |
| project | A repository, identified by its git common directory so every worktree of it is one project; outside git, the nearest directory holding `.maitred/` |
| machine | The unit whose resources maitred rations: a host, or a container with its own resource limits. One queue per machine, shared by all its users |
| ticket | A job's place in the queue: its class, reservation, project, label and state, what its job is using, and an id with a random part. It holds no command line |
| class | How a job is treated: `light`, `heavy`, `quiet`, or `none` (not queued) |
| class file | Per-project config, committed in the repository, saying which commands go through maitred, and in which class |
| reservation | The cores and memory a job is assumed to use while it runs (other named resources can come later) |
| admission | The decision that a waiting ticket may start now |
| live limits | Measured free memory below a floor, or CPU or GPU use above a ceiling, past which nothing starts, whatever the reservations say. Lower kill thresholds below them stop jobs |
| quiet | The class for jobs that need an otherwise idle machine, such as benchmarks |
| supervisor | The maitred process that waits for a ticket, then runs, measures and watches its job. It is the worker's own process |
| sampler | `maitred watch`, the one process that reads the whole machine's load and publishes it, as its own unprivileged user. It never runs or stops a job |
| machine config | The machine's own settings, owned by its administrators: capacity, live limits, kill thresholds, minimum reservations and the other numbers. It always wins over a project |
| orphan | A ticket whose supervisor has gone: a crash, a closed session, a reboot. Never marked as such; any reader sees that its lock is free |
| adoption | A worker taking over its own project's orphan with `maitred wait`, after a person confirms, and becoming its supervisor |
| record | What maitred keeps about a finished run, in the project: exit status, timings, how it ended, the load samples around it, and a capped copy of its output if asked. `maitred result` prints it |

## Problem

maitred grew out of one developer's laptop (8 cores, 16 GB) running several AI coding agents at once, each in its own sandbox and each free to start a mutation test or a full build when its task called for one. Twice in three days they started heavy jobs together and took the machine down: once by exhausting memory, once by starving the CPU until the operating system's watchdog reset it [1]. The fix was a lock: one heavy job at a time, everyone else refused. It worked, in the same way that a hammer can absolutely kill an ant if you need to kill an ant. Agents spent their time retrying in loops, ran in no particular order, and waited behind jobs that would have fit beside them [2].

The general problem: independent workers share one machine and nothing coordinates them. Each sees only its own job; none can see the machine's load, which a sandbox may hide from it. And the obvious coordinator, a daemon that runs queued jobs, is a sandbox escape: it runs whatever it is handed, with its own rights. So that's what this program does: maitred coordinates runs in the process that asked for the run.

maitred must:

1. Run waiting jobs in a defined order, with no retry loops in workers.
2. Show who holds the machine and who waits.
3. Admit work that fits under current load instead of making it wait.
4. Let jobs that need a quiet machine run on a quiet machine.
5. Never let many workers start at once on a stale reading of load.
6. Do all of that without running anything outside the requesting worker's process and sandbox.
7. Run cleanly in a standard terminal on macOS and Linux, and inside containers.
8. Record load and runs in one OpenTelemetry-compatible shape on every machine.

## Principle and threat model

**The sampler observes, maitred decides, and every job runs and dies inside the process that asked for it, in that process's sandbox.**

The threat is not a hostile outsider. It is an agent that means well and takes a shortcut: claims a light class to skip the queue, edits a script the queue trusts, runs a heavy command without maitred, or wraps a job to keep the machine awake. The design has to hold when that happens. A deliberately hostile agent, one that forges maitred's own files, is out of scope for now: agents running as the same user can't be walled off from each other's files without a daemon. What the design does instead is keep what an agent can touch small: the sampler runs as its own user and owns the load files, and a sandbox is given write access only to the tickets and requests directories.

**No process outside the sandbox runs a submitted job.** A daemon that executes queued commands runs them with its own rights, without the sandbox and without a permission prompt. An allowlist does not close that: an allowed command still runs code agents write (a build of any branch, a script in the repository). So every off-the-shelf queue that executes jobs from a daemon is ruled out:

| Tool | Why not |
| --- | --- |
| pueue | Its daemon runs the jobs: the escape above. No load-aware admission |
| task-spooler, nq | task-spooler runs jobs from a daemon. nq has the right model (no daemon, a lock per job) but runs strictly one job at a time, with no load or classes |
| GNU parallel `sem --load` | Waiter-side and load-aware, but no visible queue, no classes, no ordering guarantee |
| macOS `batch` / atrun | Load-aware, but off by default and enabled through a root launchd job |

**One process reads the machine: the sampler.** It exists so that workers never need system-introspection tools (`ps`, `top`, `pmset`, `/proc`) to see the machine; a sandbox that hides them is one reason to want that, not the only one. A supervisor reads only its own job's processes. The sampler publishes what it reads to the load files, executes nothing it is given and stops nothing, and runs as its own unprivileged user, never root, so it gives no worker more power.

**Safety rests on measurements, not on claims.** A job's class and reservation order the queue; what may start, and what must stop, is decided by the machine's load as the sampler measures it under its own user, which no agent can write. Each supervisor also measures its own job and writes the figures into its own ticket; those figures choose a victim and refine what counts as free, but never lift a live limit. A wrong class or a wrong figure can cost a slower queue, a spoiled timing or the wrong victim; if the machine carries more than the live limits allow, the kill policy stops jobs until it fits.

## Architecture

maitred supports two modes:

- `maitred watch`, the sampler, reads the machine and publishes its load. It's global to the machine or container that ran `maitred watch`.
- `maitred run` (or `maitred -- <cmd>`) is executed by each worker in its own process and sandbox; it queues, decides and supervises one job.

![maitred architecture: one sampler per machine, one supervisor per job, one writer per file](docs/architecture.svg)

Each supervisor writes only its own ticket, including what its job is using, and reads everyone else's; the sampler writes only the load files.

A job's life:

1. **Enqueue.** The worker starts `maitred -- <cmd>` and waits on it. The supervisor matches the command against the project's class file, creates its own ticket file in the machine state, and locks it for as long as it lives.
2. **Wait.** Every few seconds, in its own process: take admit.lock, read every ticket and the load file, run the admission function, release. The model is not involved.
3. **Run.** Once admitted, the supervisor marks its own ticket admitted and starts the command in a new session, still inside the worker's sandbox. While it runs, the supervisor measures it, writes the figures into its ticket and watches for the kill thresholds.
4. **Finish.** The supervisor writes the record into the project's .maitred/ (exit status, times, sleep, any kill), deletes its own ticket and exits with the job's status. The worker gets that status as if it had run the command itself.

## State on disk

State lives in two places, and neither knows where a job's output goes. **Machine state** holds only what coordination needs: one small file per live ticket, requests addressed to tickets, and the sampler's files. **Project state** holds the project's class file, committed in the repository, and its records, kept in the repository's git directory so every worktree shares them and git never tracks them. Every user of the machine can read the machine state, so a ticket carries no command line or working directory; those go only into the project's record, readable by its owner.

The machine root is shared by every user of the machine, because they share its resources. `maitred install machine` creates it, run once by an administrator, together with a dedicated, unprivileged maitred user that the sampler runs as. That user owns the root and the load files; workers can write only the tickets and requests directories, and a sandbox is given write access to those two and nothing else. Every file and directory has a stated owner and mode, and nothing in the machine state shows another user's command lines or processes beyond their queue entries. The root's location is fixed per platform (`/Users/Shared/maitred` on macOS, `/var/lib/maitred` on Linux); only test builds can move it. It must sit on a local filesystem: file locks over NFS, or across a VM's file-sharing bridge, are not reliable, so maitred refuses a network filesystem at startup. It is persistent on purpose: **queued work survives a reboot**, so a worker resumed after one does not have to enqueue everything again.

```
<machine root>/              one per machine
  admit.lock                  locked while deciding admission; never written
  sampler.lock                held by the one running sampler
  tickets/<id>                one per live ticket; written only by its supervisor
  requests/<id>/              cancel and update requests for a ticket, each written by its sender
  load/now.json               the latest sample; written only by the sampler
  load/history/               OTel data points, rotated; written only by the sampler
```

```
<project>/.maitred/classes                 the class file (read as committed on the local default branch)
<git common dir>/maitred/id                the project's random id
<git common dir>/maitred/records/<id>/     the record and the capped log (owner-only)
```

| File | Holds | Written by |
| --- | --- | --- |
| ticket | id, queue position, project, class, reservation, label, state (waiting, admitted with its process group, or stopping), what its job is using, time waited | its supervisor |
| request | the ticket it targets, cancel or update, the change, the sender (worker, project, user) and a reason | its sender |
| record | the ticket's fields plus command line, working directory, the class file's commit, start and end, exit status, sleep, how it ended and any request applied, the load samples around it, the capped log | its supervisor, when the job ends |
| `load/now.json` | when sampled, load, CPU use, free memory and memory pressure, GPU use where readable, sleep assertions | the sampler |

**maitred keeps a record of the run, never the job's output.** In a record, `exit` is the exit status and `log` is a copy of stdout and stderr. The job's real output goes wherever its own command sends it, relative to its own working directory, exactly as without maitred. Output streams through to the worker as usual; the copy in `log` is capped (configurable), or off.

Worked examples, to test the shape:

| Job | Its output | What the record holds | What it reserves |
| --- | --- | --- | --- |
| CSV export: `psql -c "\copy (…) to 'out.csv' csv"` | `out.csv` in the worker's directory | exit status, timings, the first and last part of any log | cores, memory, and one connection to the database |
| Postgres import: `pg_restore -j 4 dump.tar` | rows in the database | exit status, timings, pg_restore's messages | 4 connections, disk I/O, cores |
| Scraping with curl: `bin/fetch-pages urls.txt` | pages written under `pages/` | exit status, timings, which URLs failed | network politeness: at most one request at a time per host |

Two of these load things that are not cores or memory: connections to one database, requests to one host. **v1 reserves cores and memory only.** A reservation is stored as a map of named amounts, though, so named resources with a declared capacity (`pg:main=4`, `net:example.com=1`, as Bazel does with `--local_resources`) can be added later without changing the ticket format.

**Several projects and users share one queue.** The resource being rationed is the machine, so every project and every user on it uses the same machine state; class files and records stay in each project. Each ticket is classified by its own project's class file and labelled with its project, so their jobs interleave in one order and `maitred status` shows them all.

**One writer per file, and the lock says who is alive.** Each file has exactly one writer: a ticket its supervisor, a request its sender, the sampler's files the sampler. Everyone else only reads. A supervisor holds its ticket's lock for its whole life and never replaces the locked file, so a free lock always means a dead owner: no `ps`, no heartbeat, safe against reused pids. Readers never see a half-written file. Admission is decided under `admit.lock`, which is locked but never written, so two waiters can't both conclude they are next. A second sampler finds `sampler.lock` taken and exits. How a ticket is written in place without a reader ever seeing half of it is a mechanism for the architecture doc.

**Order without a shared clock.** Tickets are ordered by a logical sequence each new ticket takes under `admit.lock`, not by wall-clock time, so a host and a VM whose clocks disagree still agree on the order. Ids carry a random part, so two pid namespaces sharing a machine root can't produce the same one. Each supervisor measures its own waiting time on its own clock.

**Orphans and adoption.** An orphan is computed, never written: a ticket whose lock is free. It keeps its last state, so a reader can see what it was doing (waiting, or running with which process group), for which project, and for how long. Orphans keep their place in the queue but every admission rule skips them, except a running orphan whose process group is still alive, which keeps its reservation. Adoption is `maitred wait <id>` from the same project, after a person confirms, and only if the command still matches the project's class file. It is refused while the orphan's process group is still alive, since the job would then run twice. A job that was running when its supervisor died is never restarted silently; adopting it re-queues it.

**Requests, not edits.** Cancelling or updating a ticket writes a request beside it, carrying the sender and a reason; the ticket's own supervisor applies it, and the record shows it. A worker may cancel or update its own project's tickets; anyone else's needs a person.

**Cleanup.** Expired orphans and stale requests are removed during admission by the next supervisor that can take their lock. Each project's records are deleted after its retention period, and the sampler's history rotates on its own retention. No separate janitor exists.

## Admission

Every waiting supervisor runs the same pure function over the same state, under `admit.lock`, and starts its job only when that function admits its own ticket; nobody else picks. The exact algorithm is in the architecture doc; these are the rules it keeps.

| Class | Allowed to start when | Under high load |
| --- | --- | --- |
| light | its reservation fits what is free | waits on the budget and the live limits |
| heavy | its reservation fits, fewer heavy jobs run than the class's `max_running`, and the settle time has passed | waits; lighter jobs behind it may pass |
| quiet | nothing admitted by maitred is running, and the machine has stayed quiet for a few samples; or it has waited past `quiet_max_wait`, and then runs on the first empty queue with its record marked tainted | waits for the machine to settle |

- **The live limits always win.** Free memory under the floor, or CPU or GPU use over its ceiling, starts nothing, whatever the reservations say.
- **No reliable load, one at a time.** If the load file is stale, or a critical reading (CPU, memory) is missing, admission falls back to one job at a time. It never waits forever on a missing number.
- **What is free** is measured free capacity, minus what admitted jobs have reserved but aren't using yet. A job whose supervisor hasn't reported its use holds its whole reservation.
- **Queue order is kept, with backfill.** Tickets are considered in queue order. A ticket that has waited past the aging time, and that only the budget is keeping out (not its class's rules), holds its reservation out of what is free, so later, smaller jobs can only fill around it, and it starts as soon as that space is really free.
- **Nobody waits on their own size.** When nothing maitred admitted is running, the first ticket its class allows gets in even if its reservation exceeds the budget. It never overrides a class rule or the live limits.
- **A stalled waiter doesn't block.** A waiting ticket whose supervisor has stopped running passes (stopped by Ctrl-Z, or stuck) neither takes budget nor blocks anyone until it runs again.

**Reservations stop the thundering herd, and nothing a project writes can lower them.** A job takes seconds to ramp up and show in the samples, so ten waiters reading the same quiet snapshot would all start. An admitted job's reservation counts against the budget the moment it is admitted, before any load shows. The machine config sets a minimum reservation per class; a class file or checker can raise a reservation above it, never lower it.

**Settle.** Heavy starts are spaced so each one's ramp-up shows in the samples before the next starts. Getting settle and the minimums right together is the hardest part of admission, and is specified in the architecture doc.

**Where reservations come from.** A class-file rule gives a fixed reservation, or names a checker command that prints one (`<class> [cpu=<cores>] [mem=<size>]`) from what the project knows about the job. A ticket without one reserves its class default, and every reservation is at least its class minimum.

**The machine config** holds the numbers: capacity, the live limits and kill thresholds for memory, CPU and GPU, the quiet threshold, settle, aging and `quiet_max_wait`, freshness in sample intervals, and per class a minimum reservation, a default reservation and `max_running`. It is owned by the machine's administrators, always wins over what a project claims, and starts with values measured on the machine (the sampler runs observe-only first). A project's class file is validated where it is used: at enqueue, by `classes check`, in CI; never by the sampler.

## Supervision

Each supervisor is the parent of its own job: it starts it, passes signals to it, records how it ended, and is the only process that may kill it.

- **Own session.** The job runs in a new session led by a small maitred holder process, which stays until the job's processes are gone. So a signal reaches everything the job started, nothing outside can join it, and Ctrl-Z or a closing terminal reaches the job the way it would without maitred. Neither the holder nor the job inherits the ticket's lock: if they did, a daemon the job left behind (a file watcher, a preloader server) would keep the ticket looking alive.
- **Signals.** TERM, INT, HUP and QUIT sent to the supervisor go to the job's group, and stopping or resuming the supervisor (Ctrl-Z, fg) stops or resumes the job; after a grace period the supervisor sends KILL. A caller's timeout or Ctrl-C therefore stops the job and frees its reservation.
- **Nesting.** A job that itself calls maitred is still classified; it runs straight through, inside its parent's reservation, only when the caller is inside the session of a currently admitted ticket and its class is no stricter than its parent's. A stricter nested command is refused with a message saying why. A session can't be joined from outside, so this can't be forged, as an environment variable could. Nested runs count as the parent's use, so a parent that fans out past its reservation shows as overuse and is the first victim of the kill policy. A process that left the job's session (a daemon that called setsid) is not nested: it queues like any other job, and if the job waits on it, `--timeout` is what ends the wait.
- **The whole tree.** A reservation covers everything the job starts. The supervisor measures its own job's session, which runs as the same user and so is readable without privileges, writes the figures into its ticket on every pass, and reports overuse in `status` and the record. Processes that leave the session on purpose escape that measurement; they still show in the sampler's machine totals as load nobody admitted, and containing them is platform-specific (cgroups on Linux).
- **Orphaned processes.** If the supervisor itself is killed with KILL, its job can outlive it. The ticket records the process group, and admission keeps that ticket's reservation for as long as the group still exists, so the budget stays honest. A supervisor can't signal another session's processes from inside a sandbox; `maitred status` lists the group for a person to stop.

**Kill policy.** Below the live limits sit kill thresholds for free memory, CPU use and GPU use; other signals (database locks, timeouts) can be added later. When a fresh sample crosses one, each running supervisor, on its next pass, takes `admit.lock` and runs the same pure function to choose one victim. For memory and CPU it is the job furthest over its reservation on that resource, then the most recently admitted heavy job, then light, and a quiet job only past a critical threshold; v1 has no per-job GPU figure, so a GPU breach goes straight to the recency order. Only the victim's own supervisor acts on the answer: it marks its ticket `stopping`, sends TERM to its group, KILL after the grace period, and keeps the ticket as `stopping` until the sampler has published a sample taken after the kill, so the next choice sees the machine without it. Then it writes a record naming the threshold (`killed: memory`, `killed: cpu`, `killed: gpu`). While a living ticket is `stopping`, nobody chooses another victim, so one breach stops one job at a time; a `stopping` ticket whose supervisor died doesn't hold the policy up. Nothing is killed on a stale sample. A victim whose supervisor isn't running its passes (stopped, or stuck) is skipped and listed in `status` for a person. CPU trips only after staying over its threshold for several samples: a build saturating every core is normal, and only sustained saturation starves the machine. Nothing retries automatically.

**The job keeps a working terminal, and maitred still keeps its log.** When the caller is a terminal, the job runs on a pseudo-terminal that maitred relays to and from the caller and copies into the log, the way `script` and tmux do: colour, progress bars, password prompts, Ctrl-C, Ctrl-Z and window resizing all behave as they would without maitred, and the caller's terminal settings are restored on every way out. When the caller is not a terminal (an agent harness, CI, a pipe), the job gets pipes, which is what it would have had anyway, and maitred copies them into the log. Processes a rule keeps alive after the job ends are not cut off: a small drain keeps logging their output until they close it.

**Sleep.** maitred never keeps the machine awake. It records how long the machine slept during the job by comparing a clock that stops in sleep with one that doesn't (`mach_absolute_time` and `mach_continuous_time` on macOS; Linux's pair is under Platforms). A quiet job that slept gets a record marked tainted, because its timing can't be trusted.

## Class file

Each adopting project has one class file, kept in its repository and changed by pull request like any other reviewed config. It says which commands must go through maitred, and in which class.

A hook that forces heavy commands through maitred (Claude Code's PreToolUse, a shell wrapper) reads the same file, so "must be queued" and "in which class" never drift apart. The file is `.maitred/classes` at the root of the project's repository. Each row is `name | pattern | class | why`, with an optional fifth field of options:

```
# name      | pattern                          | class   | why                                       | options
mutation    | ^(bin/)?mutant +run( |$)          | heavy   | every core for hours                      | cpu=8 mem=6G
benchmark   | ^bin/bench( |$)                  | quiet   | its output is a timing; load spoils it
release     | ^make +release( |$)              | heavy   | a full optimised build on every core
dataset     | ^bin/process-dataset( |$)        | heavy   | cost depends on the dataset               | checker=bin/dataset-cost
e2e         | ^bin/e2e( |$)                    | heavy   | starts a server the suite leaves up       | leftovers=keep
tests       | ^(bin/)?(rspec|cargo +test)( |$) | none    | the suite is short; run it directly
```

- **Matching.** Before matching, a command loses leading assignments and wrappers (`env`, `exec`, `nice`, `time`, `timeout N`, `xargs`, `bundle exec`), and a path to one of the project's own files (`./bin/x`, or its absolute path) is matched as `bin/x`. A shell string (`sh -c '…'`) is split into its simple commands; each is matched, and the strictest class wins. The first matching row wins for each command. `maitred -- a && b` queues only `a`: the caller's shell runs `b` itself.
- **Classes.** `light`, `heavy`, `quiet`, or `none` (maitred refuses and says to run it directly).
- **Options.** `cpu` and `mem` give a fixed reservation. `checker` names a command that gets the job's arguments and prints a class and a reservation in the same form (`light cpu=2 mem=1200M`), from what the project knows about the job; it runs inside the sandbox like everything else, and if it fails the row's own class and reservation apply. `leftovers` says what happens to processes the job leaves behind when its main process exits: `stop` (the default), or `keep`, for a job such as an end-to-end test that starts a server it means to leave up. Kept processes stop being the ticket's, a small drain logs them until they close their output, and the sampler still counts them as load nobody admitted. The command line can move an option only in its stricter direction, as with `--class`.
- **Unlisted commands are** heavy: cautious about the machine, without the idle-machine wait that quiet would impose on every unknown command.
- **A row without a `why`, or with an option maitred doesn't know, makes the file invalid.** maitred then treats every command as heavy, warns on every run, and marks the records, rather than guess or refuse.
- **`caffeinate`,** or any wrapper that holds the machine awake, is refused in any simple command of the job, whatever the row says.

**The class file maitred reads is the one committed on the local branch.** maitred reads `.maitred/classes` as committed on the repository's default branch (configurable) in the local repository, never from the working tree. That is not a claim that anyone reviewed it: a local commit can change it, and maitred doesn't try to tell a reviewed commit from an unreviewed one. What it buys is that a change to the rules is a commit, visible in history and named in the record of every run made under it, and that a stray edit in a working tree changes nothing. Review happens where the project reviews its other config, in pull requests. To try a change without committing it, `--classes <path>` or `$MAITRED_CLASSES` points at a file in the working tree: every run under it prints a warning, its record is marked `uncommitted-classes`, and `maitred status` shows it. A project outside git gets the same treatment. The CLI offers `classes check` (validate a file) and `classes propose` (draft a change), never an apply step. A checker is code agents can edit, so its output never goes below the machine config's minimums, and the live limits, not the checker, keep the machine safe.

## Platforms

maitred runs in a standard terminal on macOS and Linux, and inside containers, from v1.

|  | Load and memory | Sleep detection | Keeping the sampler running |
| --- | --- | --- | --- |
| macOS | host statistics and memory pressure through system calls | `CLOCK_MONOTONIC` minus `CLOCK_UPTIME_RAW` | optional launchd daemon, as the maitred user |
| Linux | `/proc/loadavg`, `/proc/meminfo`, `/proc/pressure/*` | `CLOCK_BOOTTIME` minus `CLOCK_MONOTONIC` | optional systemd service, as the maitred user |
| Container | the container's cgroup limits and usage (`memory.max`, `memory.current`, `cpu.max`), not the host's load average, which a container sees but cannot act on | as Linux | the container's entrypoint, inside the container it rations |

- **The sampler is an ordinary foreground command, run as the maitred user.** `maitred watch` runs in any terminal (`sudo -u maitred maitred watch`); `maitred install machine` can register it with launchd or systemd to keep it up, still as that user. It never runs as root.
- **A supervisor measures only its own job.** Its job's processes run as the same user, so the platform lets it read their CPU and memory without privileges; the sampler, as another user, often can't, which is why the measurement lives in the supervisor.
- **GPU readings are best effort.** NVIDIA through NVML, AMD through sysfs, Apple Silicon through the accelerator statistics IOKit publishes, each expected to be readable without root (to be verified per platform). On Apple Silicon, GPU memory is the shared pool the memory limits already watch. A machine with no readable GPU has no GPU limit, and `status` says so.
- **A container is a machine.** It gets its own sampler and queue, sized to its cgroup limits. Container memory limits are caps, not guarantees, and their sum can exceed the host's memory, so host safety needs container limits that fit inside the host. One queue shared by a host and its containers through a shared directory is unsupported until file locks are shown to hold across that boundary (they are not expected to over Docker Desktop's VM file sharing).
- **Local filesystems only.** File locks over NFS or a VM's file-sharing bridge are not reliable; maitred checks the machine root's filesystem at startup and refuses a network one.

## Interface

A worker runs `maitred -- <cmd>` and, when it ends, gets the job's own exit status; it never polls.

| Command | Does |
| --- | --- |
| `maitred [--label L] [--class C] [--timeout T] -- <cmd>` (or `maitred run`) | Enqueue, wait, run, exit with the job's status. Output passes through; a capped copy is kept if asked. With `--timeout`, give up waiting after T and say so |
| `maitred submit […] -- <cmd>` | Enqueue and start a detached supervisor, print the ticket id, exit at once |
| `maitred wait <id>` / `--label L` | Block until that job (or every job with that label) has a record, then exit with its status. On an orphan from the same project, offer adoption, which a person confirms |
| `maitred status [--project] [--json]` | Who runs and who waits (position, class, waited, reservation, use), the load file's age, orphans, overuse, stopped supervisors, unexpected sleep assertions; machine-wide or one project's |
| `maitred result <id>` | Print the record. Authoritative for how a run ended |
| `maitred cancel <id> --reason R` | Write a cancel request; the ticket's own supervisor stops its job, and the record shows who and why. Own project's tickets only; anyone else's needs a person |
| `maitred update <id>` | Write an update request: a new label, or a stricter class. Nothing that could move a ticket ahead |
| `maitred classes check` / `propose` | Validate a class file; draft a change |
| `maitred export --machine` / `--project` | The sampler's history, or a project's records, through an exporter (v1: OTLP-JSON) |
| `maitred watch` | The sampler, run as the maitred user |
| `maitred install machine` | Create the machine root and the maitred user and, if asked, register the sampler with launchd or systemd. Run once by an administrator |
| `maitred install claude-code [--check]` | Install the agent skill and the PreToolUse hook, after showing every change and asking |

- **`--class` can only make a job stricter** (light to heavy to quiet), never lighter than its row. Otherwise it would be a way around the class file.
- **Exit status.** When the job ran, maitred exits with the job's own status. When maitred itself decided (a refusal, a timeout, a kill at the live limits, a cancel), it says so on stderr with the `maitred:` prefix, and `maitred result` is authoritative: any number maitred chose could also be a job's own exit.
- **Messages** go to stderr with the `maitred:` prefix, starting with the ticket id, so the record can always be found: queued at position 3, admitted after 4m10s, stopped at the live limits.
- **`submit` and `wait` exist for long queues and for adoption.** A caller may have a time limit (an agent harness's background call does), and a timeout sends TERM, which would stop a waiting or running job. A supervisor started by `submit` is a detached child of a sandboxed shell, so it stays sandboxed.
- **`caffeinate`** (or any wrapper that holds the machine awake) is refused in the argv. `maitred status` lists sleep assertions held by processes the machine config hasn't declared expected, such as an agent harness's own.

**Left out on purpose:** pause, stash and reordering (pueue has them). They need a daemon or a shared writer.

**For agents.** maitred ships a skill for Claude Code (when to use it, running heavy commands as a background call, the exit codes, waiting for and adopting orphans), the same text as an AGENTS.md snippet for other harnesses, and an optional PreToolUse hook that wraps matching commands automatically. `maitred install claude-code` writes them only after a person confirms, since a hook is a person's decision. It ships no MCP server: one would run outside the sandbox and execute jobs, the escape this design rules out.

## Feature discovery

The closest prior art, [nq](https://github.com/leahneukirchen/nq), already has the right shape: no daemon, one file lock per job, jobs ordered by timestamp. Its README notes, in one neutral line, that it runs jobs "with no regard to the system's load average", and for nq that costs nothing: its rules are "exclusive execution is maintained strictly" and "polling is not used", so one queue runs one job at a time and load is bounded by construction. Load only needs measuring once independent workers share a machine, which is maitred's case. GNU [parallel](https://www.gnu.org/software/parallel/parallel.html) admits on load and free memory but has no visible queue or classes. maitred is nq's model plus admission, and it should borrow nq's habit of waiting on locks rather than polling wherever admission allows.

Candidate features, from prior art:

| Feature | From | Verdict |
| --- | --- | --- |
| Admit on reservations, clamped to capacity so a request larger than the machine still runs alone instead of deadlocking | Kubernetes requests, [Bazel](https://bazel.build/reference/test-encyclopedia) `cpu:n`, task-spooler `-N` | v1: already the design, plus the clamp |
| The first job always gets in, whatever the load | GNU [make](https://www.gnu.org/software/make/manual/make.html) `-l` | v1: one rule in the admission function |
| Exclusive class, checked before, sampled during, verdict after (a quiet run another job touched is marked tainted, not failed) | Bazel `exclusive`, pyperf, hyperfine, Google benchmark | v1: the quiet class, with an after-the-fact verdict |
| Cancel by killing the waiting process; `status --json`; `wait` by ticket or label | nq, pueue, task-spooler | v1 |
| Fail fast: give up after a timeout with its own exit code instead of waiting forever | task-spooler `-B` | v1: agents need it |
| Sleep detection on every run: macOS `CLOCK_MONOTONIC` minus `CLOCK_UPTIME_RAW`, Linux `CLOCK_BOOTTIME` minus `CLOCK_MONOTONIC` | the platform clocks | v1, read directly: Rust's `Instant` differs by platform |
| Suspend the job (SIGSTOP) under memory pressure and resume it when the floor clears, before resorting to kill | parallel `--memsuspend`, Buck2 suspension | v2: gentler than the kill policy |
| Self-applied limits on the admitted job: `taskpolicy` on macOS, `systemd-run --user --scope` on Linux (the caller stays the parent) | systemd, macOS | v2, if they work from inside a sandbox (unverified) |
| Age priority, named singletons, caps per label | Slurm, ninja pools | v2 |
| An external admission hook (exit 0 start, 1 hold, 2 kill youngest) | parallel `--limit` | later, only if it runs in the waiting process |
| Opt-in power assertion for one job's lifetime | `caffeinate -i`, `-w` | later, explicit flag only; no tool does this on request today |
| Backfill by expected runtime | Slurm | no: needs runtime history first |
| GPU: watched by the live limits and the kill policy, then reserved (gpu=1, or GPU memory) as the first named resource. For local model inference and ML training, now common jobs | Slurm GRES, task-spooler -G | v1 watches GPU use where it is readable without root (see Platforms); reservations come after v1 |
| More kill signals: database locks, timeouts | none yet | later; the kill policy takes them as further thresholds |

**Telemetry is OpenTelemetry data from the start.** The sampler's samples and every record's measurements are stored as OTel metric data points, one JSON object per line: resource attributes (`host.name`, `os.type`, `host.arch`), metric name, unit, value and timestamp, with names from OTel's host-metrics conventions (`system.cpu.utilization`, `system.memory.usage`, `system.cpu.load_average.1m`) and a `maitred.*` namespace for its own (`maitred.queue.waiting`, `maitred.job.duration`). The machine's history and each project's records are exported separately (`maitred export --machine`, `--project`), each as OTLP-JSON with no translation, so any OTel collector can take them and every machine's data has the same shape by construction. Both have a configurable retention. Prometheus's textfile format was ruled out: it carries no timestamps.

Export goes through a small exporter interface between the stored data points and the output. v1 ships OTLP-JSON only, because that is where the ecosystem is aligning; other formats can be added later, built in or as plugins, without touching the sampler.

**Editing the class file:** settled; a change is a commit on the default branch, reviewed wherever the project reviews its config (see Class file).

**Sleep.** Detection is always on (above). Keeping the machine awake is an explicit, per-job flag, if it exists at all.

**Name.** maitred, pronounced "mate-urd": a maître d' takes reservations, keeps the waiting list and seats you when a table is free. The working name, `jobq`, is taken on crates.io by an unrelated 2020 crate; `maitred` was free there when checked on 2026-10-07.

## Open questions

**Decided (2026-10-07):**

- [x] Licence: MIT and Apache-2.0, the Rust convention.
- [x] Dependencies come from crates.io, which the development sandbox now allows.
- [x] The sampler runs observe-only for a week before admission is enforced, so the machine config starts from measured numbers.
- [x] Linux and containers are in v1 alongside macOS.

**To verify before the code depends on them:**

- [ ] The time limit on an agent harness's background call, and whether ending a session kills detached children (decides how much `submit` and `wait` matter).
- [ ] Whether file locks hold across a host and container boundary on each platform.
- [ ] Checking a process group's existence from inside a sandbox (`pgrep -g` or an equivalent), for orphaned groups.
- [ ] A supervisor reading its own job's CPU and memory from inside a sandbox (`proc_pid_rusage` on macOS, `/proc` on Linux).
- [ ] GPU use readable without root on each platform, Apple Silicon especially.
- [ ] The name on Homebrew (`maitred` was free on crates.io on 2026-10-07).
- [ ] How a worker resumed after a reboot finds its project's orphans: likely `status --project` and adopting them together.

## Notes

**[1] The two resets.** From the issue tracker of the first project to adopt the lock, gravedigger (a dead-code finder whose test runs dig large codebases), entries JOB-001, JOB-004, GEN-012 and GEN-013.

| Date | What was running | What failed |
| --- | --- | --- |
| 2026-09-28 | A full sweep of test codebases beside other jobs | Memory. The sweep capped memory with `ulimit -v`, which macOS doesn't enforce, and its backstop read memory through `ps`, which returned nothing inside the sandbox, so it saw 0 MB until the machine ran out and reset |
| 2026-09-30, 19:43 | 4 mutation-test runs and 3 full test suites from different sessions; load 60 to 110 on 8 cores | CPU. The window server stopped responding, macOS restarted it twice, and after 120 s the watchdog reset the machine. No memory kill came first |

A third episode the same week reached load 88 to 120 when two sessions dug at once; a timeout killed the job instead of the machine. Two lessons carry into the design: CPU alone can take a machine down, so the live limits include a CPU ceiling as well as a memory floor; and a memory reading that silently returns 0 is worse than none, so a missing or stale reading is treated as the most cautious case.

**[2] Thirty days under the lock** (2026-09-07 to 2026-10-07, read from the agents' session transcripts):

| Measure | Value |
| --- | --- |
| Jobs that held the lock | 164 |
| Refusals | 336, across 13 sessions |
| Polls of the lock's status in 14 days | 127 |
| Holds by jobs that never needed the lock (test suites) | 18 |
| Refusals caused by a single holder | 88 |
| Longest hold of a ~3 minute job | 00:13 to 07:04, across a machine sleep |

Most refused work would have fit: of 20 codebases dug under the lock, one was large enough to need it, and a typical dig used 1 to 2 cores and at most 1.2 GB.
