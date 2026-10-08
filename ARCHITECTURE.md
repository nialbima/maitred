<!--
  Synced from the maitred architecture doc on claude.ai (rev 67, 2026-10-07):
  https://claude.ai/code/artifact/00385e4d-eabc-4436-b2d3-6016ffd43e58
  Draft and revise there; this file is regenerated from it, so edits made only here are lost.
  The drawings are docs/modules.svg and docs/ticket-states.svg, exported from the doc's drawings.
-->

# maitred architecture

2026-10-07 · Nicholas Allen Bickford Marshall

## Scope

This document turns the maitred design into something buildable: exact states, file formats, algorithms and crates. The [design](https://claude.ai/code/artifact/b0753fb5-a907-459b-8266-2b18f0401e5b) (`DESIGN.md`) says what maitred is and why; nothing here changes it. Where building forces a design change, it goes back to the design doc first.

This document decides:

- how the binary is divided into modules, and which of them may touch the operating system;
- every state a ticket can be in, every transition, and which single writer makes each one;
- the byte-level format of every file, and how a locked file is written in place without a reader ever seeing half of it;
- the admission function and the kill policy, step by step, including the parts the design left to here (settle, minimums, aging, victim choice);
- how a supervisor runs, measures, stops, nests and cleans up;
- what the sampler reads on each platform and how it maps to OpenTelemetry;
- the crate chosen for each of those, and why.

It does not decide the numbers in the machine config. Those come from a week of the sampler running observe-only on a real machine.

## Process model and modules

maitred is a Cargo workspace of two crates: `maitred-core`, which decides and touches nothing, and `maitred`, the one binary, which does all the reading, writing, locking and spawning around it.

![maitred's modules: commands on top, the pure core and two OS-facing layers below](docs/modules.svg)

- **maitred-core** holds the types (ticket, snapshot, reservation, record), `decide`, the state machine's transitions, class-rule matching and the OTel data-point types. No file, clock, process or environment access: everything it needs arrives as an argument. That is what makes admission testable by property and replayable by the simulator.
- **maitred** holds the commands and the layers that face the operating system: `store` (the machine root and project files, slots, locks), `platform` (one backend each for macOS, Linux and cgroups: clocks, signals, process groups, sampling) and the command modules on top.

Every process is one of three kinds, and none outlives its purpose:

| Process | Started by | Runs as | Lives | Holds |
| --- | --- | --- | --- | --- |
| supervisor (`run`, or the detached child of `submit`) | a worker | the worker's user, in its sandbox | until its job's record is written | its ticket's lock; briefly, `admit.lock` |
| holder (`maitred __hold`) | the supervisor, once admitted | the worker's user | while any process of the job's session lives | the job's session, as its leader; never a lock |
| drain | a supervisor whose rule keeps leftovers | the worker's user | until the kept processes close their output | nothing |
| sampler (`watch`) | an administrator, or launchd or systemd as installed | the maitred user, never root | until stopped | `sampler.lock` |
| short commands (`status`, `cancel`, `wait`, `result`, `export`, `classes`) | anyone | the caller | one call | nothing for longer than a read |

## Ticket state machine

A ticket has four written states, an end and one computed state: `waiting`, `admitted`, `running`, `stopping` and its end are written by its supervisor; orphaned is never written, only seen.

![ticket states: solid boxes are written by the ticket's supervisor, dashed ones are only seen](docs/ticket-states.svg)

| From | To | When | Written by |
| --- | --- | --- | --- |
| (none) | waiting | the ticket is created, locked and renamed into place, with the next `seq` | its supervisor, under `admit.lock` |
| waiting | admitted | `decide` admits it | its supervisor, under `admit.lock` |
| admitted | running | the holder is spawned and waits at its gate; its pid, session and start time are written; only then is the gate opened and the job forked and exec'd | its supervisor |
| running | stopping | the kill policy chose it | its supervisor, under `admit.lock` |
| stopping | ended | the job's group is gone and the sampler has published a sample taken after the kill: record first, then delete | its supervisor |
| running | ended | the job exits, is cancelled or signalled: record first, then delete | its supervisor |
| waiting or admitted | ended | cancelled, timed out, signalled, or the spawn failed: record first, then delete | its supervisor |
| any | orphaned (computed) | its supervisor dies; the kernel frees the lock | nobody: readers see the free lock |
| orphaned, waiting | waiting | adopted (same project and uid, the command still matches, confirmed interactively) | the adopter, who now holds the lock |
| orphaned, admitted | waiting | adopted, keeping its `seq`. No job ran: a holder at its gate exits when its supervisor's end of the gate closes | the adopter |
| orphaned, running or stopping, record present | removed | the job had finished; only the deletion was lost | any later supervisor of the same uid (the sweep), or `wait` |
| orphaned, running or stopping, group alive | (unchanged) | adoption is refused: the job is still running, and adopting would run it twice | nobody |
| orphaned, running or stopping, group gone, no record | ended + new waiting ticket | adopted: the record says `interrupted`; the command is queued again as a new ticket | the adopter |
| orphaned | removed | past `orphan_ttl`, and not running with a live group: the sweep writes an `expired` record into the project first | the sweep of the next supervisor of the same uid |

Three invariants follow, and the tests check each: a ticket's file only changes while someone holds its lock; no job runs whose session isn't already in its ticket; and no ticket disappears before its record exists. A record is created exclusively (written under a temporary name, then linked into place, which fails if one exists), so two writers can never both write one. So `wait` on an id always ends: with the record, or, if there is neither ticket nor record, with the outcome `lost`.

## Files and the write protocol

Every file has one owner, one stated mode and one writer; a ticket is rewritten in place, into one of two slots, so a reader always finds one complete copy; and every lock is checked to still belong to the file at its path.

### Where things live, and who owns them

`maitred install machine`, run once by an administrator, creates a dedicated system user (`_maitred` on macOS, `maitred` on Linux), the machine root, the machine config and the sampler's service. Nothing below is created on first use by whoever happens to run first.

| Path | Owner | Mode | Written by |
| --- | --- | --- | --- |
| machine root: `/Users/Shared/maitred` (macOS), `/var/lib/maitred` (Linux) | maitred user | `0755` | nobody after install |
| `admit.lock`, `sampler.lock` | maitred user | `0644` (a lock needs only a read-only open) | never written; only locked |
| `tickets/` | maitred user | `1777`: any user creates; only a file's owner (or root) deletes it | supervisors, each its own ticket |
| `requests/` | maitred user | `1777`, like `tickets/` | nobody; holds one directory per ticket |
| `requests/<ticket-id>/` | the ticket's owner | `0700`, created before the ticket: only the same user can send a request | the owner (its `project` file); senders of the same uid |
| `load/` and everything in it | maitred user | `0755` dir, `0644` files | the sampler only |
| machine config: `/Library/Application Support/maitred/config.toml`, `/etc/maitred/config.toml` | root | `0644` | an administrator |

There is no per-user state outside the machine root and the projects: the one thing the sweep needs to find a ticket's project, its path, sits in `requests/<ticket-id>/project`, readable only by the ticket's owner. `maitred install claude-code` adds exactly two machine paths to the agent sandbox's write allowlist, `tickets/` and `requests/`, after showing the change; the sampler's files stay unwritable. The Linux root lives under `/var/lib`, not `/var/tmp`, so no temporary-file cleaner ages out its lock files; the install also drops a `tmpfiles.d` exclusion for anyone who moves it. The machine root's path and the `--machine-config <path>` flag can be changed only in debug builds; both are compiled out of releases.

### Locks that can't be fooled

- **Liveness reads take a shared lock.** A reader tests a ticket with `flock(LOCK_SH | LOCK_NB)`: it fails while the owner holds its exclusive lock, and many readers can test at once without making a dead ticket look alive to each other. Only an adopter or the sweep takes `LOCK_EX` on someone else's ticket.
- **Every lock is re-checked against its path.** After taking any lock (`admit.lock`, `sampler.lock`, a ticket), the holder compares the locked descriptor's inode with the inode now at the path, and starts again on a mismatch. A file deleted and recreated underneath can't produce two owners.
- **`admit.lock` is taken with a deadline.** A supervisor tries every 50 ms; after 10 s it prints how long the lock has been held and keeps trying. flock can't say who holds it; `status` reports the wait.

### The ticket: two slots and a lock

A ticket is `tickets/<id>.tk`, mode `0644`, a file of two fixed 4 KiB slots:

```
magic "MTRDTK03" (8) | v u16 | class u8 | state u8 | seq u64 | generation u64 | length u32 | crc32 u32 | JSON payload | zero padding to 4096
```

- **The checksum covers everything after the magic but itself,** so a slot torn anywhere fails it. `v`, `class`, `state` and `seq` sit in the fixed header so that a reader that can't understand a newer payload still knows enough to place and charge the ticket (Versions).
- **Writing.** The owner writes generation g+1 into the slot that holds g-1, then `fdatasync` on Linux (`fsync` on macOS, which orders nothing on the disk: after a power loss both slots may fail, and the ticket then reads as unknown and unlocked, which the sweep removes). The slot holding g stays intact throughout.
- **Reading.** Read both slots in one 8 KiB read, keep those that pass, take the higher generation. If neither passes, read again, up to three times; a reader that still finds no valid slot treats the ticket as unknown (Versions).
- **Creating without a gap.** Create `requests/<id>/` (mode `0700`; if it already exists and isn't ours, pick a new id), write its `project` file, then create `tickets/.new-<id>`, lock it, write the first slot and rename it to `<id>.tk`. A rename keeps the inode, and the lock lives on the inode, so the ticket appears already locked.
- **Still mine?** On every pass a supervisor checks that the inode at its ticket's path is the one it holds locked. If the file was deleted (an agent tidying `tickets/`), it recreates it under `admit.lock`, keeping its `seq`, and warns; the record notes it.
- **Ids.** `<unix-ms, hex>-<64 random bits, hex>`. Nothing sorts by them.

### The ticket's payload

```
{
  "v": 3,
  "id": "0192f3a1c2e0-9f3c2a7e5b1d4c88",
  "seq": 4183,
  "boot": "…",
  "pidns": "4026531836",
  "user": 501,
  "project": "7c41e09a… (the project's random id)",
  "class": "heavy",
  "reservation": { "cpu_millicores": 2000, "mem_mb": 1200 },
  "flags": ["uncommitted-classes"],
  "state": "running",
  "waited_ms": 41200,
  "last_pass": "2026-10-07T21:14:02Z",
  "admitted_at": "2026-10-07T21:13:58Z",
  "leader": { "pid": 48210, "sid": 48210, "pgid": 48211, "start": "…" },
  "use": { "cpu_millicores": 1850, "mem_mb": 760, "measured_at": "2026-10-07T21:14:02Z" },
  "stopping": null
}
```

- `seq`: under `admit.lock`, one more than the largest `seq` of any ticket, read from the fixed header, so an unknown payload doesn't hide it.
- `boot`: read by the supervisor itself: Linux `/proc/sys/kernel/random/boot_id`; on macOS, the start time of process 1 from `sysctl` `KERN_PROC_PID`, which a sandbox allows. No dependence on the sampler.
- `pidns`: the pid namespace on Linux; empty on macOS. Process ids are only compared within one boot and namespace.
- `project`: a random id kept in the project (`<git common dir>/maitred/id`, created on first use), so another user can't tell which project it is. The label is never shown to other users; `status` shows other users' tickets as user, class, reservation and state only.
- `waited_ms` is the owner's own clock. `last_pass` is when the owner last started a pass, waiting or running; it is written before contending for `admit.lock`, so a slow lock never makes a live supervisor look stalled.
- `leader`: once running, the holder's pid, the session it leads, the job's process group and the holder's start time. A session counts as this job's only if the holder's start time still matches, which defeats pid reuse; a `pgid` of 0 or 1 is never accepted.
- `use`: the job's measured use, written by the supervisor on every running pass (Supervision). It is as trustworthy as the rest of the ticket, which is to say written by the same user as the job; the live limits rest on the sampler's totals, never on it.
- `stopping`: once the kill policy has chosen this ticket, the threshold and the time of the kill.

### Requests

`requests/<ticket-id>/`, created by the ticket's owner with mode `0700` before the ticket itself, holds the project's path (`project`) and the requests: `<request-id>.json`, written once (temporary name, then rename), with `kind` (`cancel`, `update` or `nested`), the change, the sender's project, label and pid, and a reason. The target's supervisor applies or rejects each one, notes it in the record, and deletes it. A `nested` note is how a nested run tells its parent it ran, so the parent's record can list it. Another user can't write here at all; a person who needs to cancel another user's job does it as that user (`sudo -u`) or as root.

### Project files

A project's records live in its git common directory, `<git common dir>/maitred/records/<id>/`, so every worktree shares them, git never tracks them, and they are as private as the repository itself. Outside git they live in `.maitred/records/`.

| File | Holds | Written |
| --- | --- | --- |
| `job.json` | command line, working directory, environment keys, resolved class, the class file's commit | once, at enqueue |
| `record.json` | the ticket's last payload, start and end, exit status or signal, how it ended (completed, cancelled with sender and reason, timed out, killed with its threshold, interrupted, expired, lost), sleep seconds, overuse, nested runs, the samples around it, flags | once, before the ticket is deleted, created exclusively |
| `log` | head and tail of the job's output, capped (default 64 KiB each) | while the job runs, and by the drain afterwards |

### The sampler's files

All in `load/`, written only by the sampler as the maitred user:

- `now.json`: the latest sample, replaced by temporary name and rename.
- `history/<date>.jsonl`: OTel data points, one file per day, deleted after the retention period.
- `admissions.jsonl`: each admission the sampler saw, for settle; kept one hour.

### Versions

Every file carries `v`. A ticket whose payload a reader can't understand (a newer version) is placed and charged from its fixed header. A ticket with no valid slot at all is unknown: while its lock is held it is charged at the heavy minimum and placed last; once its lock is free, it is charged nothing, and the sweep of a supervisor of the same uid deletes it (there is no project to write a record into). Formats only change in ways one older version can read, and the fixed header never changes within a major version.

## Admission

Admission is one pure function, `decide(snapshot, me) -> Decision`, in the core module: no I/O and no clock, so every waiter computes the same answer from the same snapshot, and tests and the simulator can drive it with any snapshot.

### One pass

A waiting supervisor, every `poll_interval` (default 2 s, with up to 20% jitter):

1. Writes `last_pass` into its own ticket (its own lock is enough) and checks the ticket is still its own (Files).
2. Blocks TSTP, TTIN and TTOU, then takes `admit.lock` (with the deadline and inode check above). A Ctrl-Z that arrives now is delivered once the lock is released, so a stopped supervisor never holds it.
3. Builds the snapshot: every ticket's header, payload and liveness (a shared try-lock each), `load/now.json` and its age, the last heavy admission (the latest of `load/admissions.jsonl` and the `admitted_at` of living heavy tickets), the machine config, and `now`.
4. Runs `decide`. If its own previous pass is older than the stall limit (it slept, or was stopped), this pass only refreshes: it can't admit itself, so after a machine wakes nobody jumps the queue while everyone else still looks stalled.
5. On **Admit**: writes its ticket as `admitted`, still holding the lock, so the next snapshot already charges it; releases the lock; starts the job.
6. On **Wait(reason)**: writes `waited_ms` and the reason; releases the lock; prints the reason if it changed; sleeps.

### Inside `decide`

All CPU figures are millicores and all memory figures megabytes; the sampler converts its fractions before writing.

```
charged = living tickets that are admitted, running or stopping
        + orphaned running or stopping tickets of this boot and namespace whose session is alive   # reservation only
        + locked tickets with no readable slot (heavy minimum)
waiting = living waiting tickets whose last_pass is recent (stalled ones are skipped), by seq

if load is stale, or a critical reading (cpu, memory) is missing:
    admit me only if charged is empty and I am first in waiting      # plain-lock fallback
    else wait(no reliable load)
if mem_available < mem_floor or cpu_used > cpu_ceiling or gpu_used > gpu_ceiling:
    wait(live limits)
if any living charged ticket is quiet:
    wait(quiet job running)

free.cpu = capacity.cpu - cpu_used        - Σ charged max(0, reserve.cpu - used.cpu)
free.mem = mem_available                  - Σ charged max(0, reserve.mem - used.mem)
counts   = running heavy count; last_heavy = last heavy admission

for t in waiting:
    if allowed(t, counts, last_heavy, load) and reserve(t) fits free:
        if t is me: admit
        free -= reserve(t); update counts and last_heavy as if t were admitted
    else if charged is empty and nothing admitted yet in this walk and allowed(t, ...):
        if t is me: admit                                               # first in: waives only the budget
        stop the walk
    else if t is quiet and has waited past quiet_max_wait:
        stop the walk                                                   # let the machine drain for it
    else if t is aged and allowed(t, ...):
        free -= reserve(t)                                              # only the budget keeps it out: hold its place
wait(the first rule that held me)
```

| Term | Exact meaning |
| --- | --- |
| `reserve(t)` | `max(t.reservation, class_min[t.class])` per dimension, capped at capacity |
| `used` | a living ticket's own `use`, if it was measured within the last 3 sample intervals; otherwise 0, so the ticket holds its whole reservation. Orphans and unknown tickets count as using nothing |
| allowed | **light:** always. **heavy:** fewer than `max_running.heavy` charged or admitted on paper, and `now - last_heavy >= settle`. **quiet:** nothing charged or admitted on paper, and `quiet_streak >= 2` in the load file; or it has waited past `quiet_max_wait`, in which case the streak isn't needed and its record is marked `tainted` |
| aged | waited past `aging` (any class). An aged ticket that its class allows but the budget doesn't holds its reservation out of `free`, so later tickets can only fill around it, and it gets in as soon as the budget it holds is really free. One that its class rules keep out (settle, `max_running`) holds nothing, since holding budget wouldn't let it in sooner |
| first in | when nothing maitred admitted is running, the first ticket that its class allows gets in even if its reservation exceeds the budget. It never overrides a class rule or the live limits |
| stalled | a waiting ticket whose `last_pass` is older than 5 poll intervals: its supervisor is stopped or stuck, so it neither takes budget nor blocks anyone; it is admitted again when it next runs a pass |
| orphan charge | a running orphan whose session is alive is charged its reservation, but its class's barrier doesn't apply: an orphaned quiet job doesn't stop the queue, and `status` says why it is charged |

### Settle and the minimums

The minimum reservation per class bounds a burst within one pass; settle spaces heavy starts across passes, so each new job's ramp-up shows in the samples before the next one starts. The load figures themselves are fresh (CPU over the last 2 s interval, memory now): the lag settle covers is the job's own ramp-up, not measurement. Both numbers come from the observe-only data via the simulator. With no reliable load, admission is one job at a time anyway.

### Kill policy

Part of v1. A running supervisor's pass, every `poll_interval`: write `last_pass`, measure its job and write `use` (Supervision), apply its requests, and then:

1. **Trigger.** The load file is fresh, no living ticket is `stopping`, and one of: `mem_available < kill_floor` (below the admission floor); `over_cpu_streak >= cpu_kill_samples`; `over_gpu_streak >= gpu_kill_samples`. The sampler keeps both streaks, counting consecutive samples over `cpu_kill_ceiling` and `gpu_kill_ceiling`, so a few seconds of saturation never trips. More triggers (database locks, timeouts) are further thresholds of the same shape.
2. **Choose, under `admit.lock`.** `choose_victim(snapshot, trigger)` is pure, like `decide`. Candidates are living running tickets whose `last_pass` is recent. For memory or CPU: the candidate furthest over its reservation on that resource, if any is over; otherwise the most recently admitted heavy candidate, then light, and quiet only past the critical threshold (`mem_critical`, or the CPU streak at twice its limit). For GPU, v1 has no per-job figure: the recency order alone. If the victim is this supervisor, it writes its ticket as `stopping` (threshold, kill time) before releasing the lock; if not, it releases and does nothing, and the victim's supervisor reaches the same answer on its own pass.
3. **Act.** Only the victim's own supervisor signals: TERM to its group, KILL after `grace`. It keeps the ticket `stopping` until the group is gone and the sampler has published a sample taken after the kill, so the next choice sees the machine without it; if the load file goes stale meanwhile, it waits at most 3 intervals more. Then the record (`killed: memory`, `killed: cpu`, `killed: gpu`), then the deletion. Nothing retries automatically.
4. **Who is the cause.** Every kill record carries how much of the machine's use was maitred's (the sum of the charged tickets' `use`). When most of the pressure came from outside maitred, the kill still happens, because stopping a maitred job is the only room maitred can make, and `status` and the record say where the pressure came from.

A chosen victim that stops running passes (Ctrl-Z, a stuck supervisor) drops out of the candidates on the next choice, and `status` lists it for a person. An orphan marked `stopping` is not living, so it never holds the policy up.

## Supervision and cleanup

A supervisor is one process with one job: from enqueue to record, it is the only thing that changes its ticket and the only thing that signals its job.

### From enqueue to record

1. **Classify.** Find the project and its class file (Class files), split shell commands, match, run a checker if the rule names one. A `none` rule or a keep-awake wrapper refuses here, before any ticket exists.
2. **Nesting check.** If the caller's session id, boot and pid namespace equal those of a living `running` ticket's leader, the caller is inside an admitted job. Its class must be no stricter than that ticket's: then it runs directly, inside the parent's reservation, with no ticket of its own, and leaves a `nested` note in the parent's request directory so the parent's record lists it. A stricter nested command is refused with a message naming both classes. A session can only be joined by its own members, so nothing outside the job passes the check. Nested runs are measured as part of the parent's session, so fanning out past the parent's reservation is overuse like any other.
3. **Prepare.** Write `job.json` in the project's records, create the request directory, then create the ticket under `admit.lock` and print its id first: `maitred: 0192f3a1c2e0-9f3c… queued (heavy, position 3)`. `submit` does exactly this and then forks: the lock lives on the open file, which the detached child inherits, so the ticket never looks orphaned in between; the parent exits once the child is running.
4. **Wait** in the admission loop, reading its own requests on every pass.
5. **Spawn behind a gate.** Start `maitred __hold` with std `Command`, passing the gate's read end as fd 3 (a `dup2` in `pre_exec`, which never blocks). The holder calls `setsid`, takes the pseudo-terminal as its controlling terminal in terminal mode, reports its pid, session and start time, and blocks reading the gate. The supervisor writes the ticket as `running` with those, then writes to the gate. Only then does the holder fork the job into a new process group of its session (made the terminal's foreground group in terminal mode) and exec the command. If the supervisor dies first, the holder reads end-of-file and exits without running anything.
6. **Connect the terminal** (below), and copy output into the log.
7. **Run its passes** (Kill policy): `last_pass`, measurement, requests, the kill check.
8. **Finish.** When the job's main process exits, the holder applies the rule's `leftovers` option to the rest of the session: `stop` sends TERM, then KILL after the grace period; `keep` leaves them, and the holder stays as the session's leader while any of them lives, so the terminal isn't hung up under them, while a drain takes their output (below). The supervisor writes `record.json`, deletes the ticket and exits with the job's status.

**Measuring the job.** On every running pass, the supervisor sums its session's processes: on Linux, the session field of `/proc/<pid>/stat` finds them, CPU time comes from the same file and memory from the proportional set size in `/proc/<pid>/smaps_rollup`; on macOS, `sysctl` lists the session's processes and `proc_pid_rusage` gives CPU time and the physical footprint. Proportional size and physical footprint count shared pages once, unlike summed resident size, which would overstate the job and starve the queue. The processes run as the supervisor's own user, which is what lets it read them without privileges (to be verified inside each sandbox). Processes that leave the session aren't measured; they show only in the sampler's totals.

### The job's terminal

| Caller's stdin and stdout | The job gets | So |
| --- | --- | --- |
| both terminals, and a pseudo-terminal can be opened | a new pseudo-terminal, the controlling terminal of the holder's session, with the job's group in the foreground; the caller's terminal goes into raw mode and maitred relays both ways, copying output to the log | colour, progress bars, password prompts, `/dev/tty`, window size and Ctrl-C/Ctrl-Z/Ctrl-\ behave as without maitred: the job's group has a parent in another group of its session (the holder), so the kernel delivers the stop signals instead of discarding them; stdout and stderr arrive interleaved, as on a terminal |
| anything else (an agent harness, CI, a pipe, or a sandbox that refuses pseudo-terminals) | pipes, relayed and logged; the caller's stdin passed through unchanged. When the caller's stdout and stderr are the same file (`2>&1`), the job gets one pipe for both, so their order is kept | what the job would have had without maitred: no terminal |

The window size is copied at start and on every SIGWINCH. Relaying stops when the job's main process has exited and the session is empty or handed to a drain. The caller's terminal settings are restored by a guard that runs on every exit path, including a panic and every signal maitred handles; only a supervisor killed with KILL leaves raw mode behind, as any raw-mode program does, and `reset` clears it.

**The drain.** When a rule keeps leftovers, they may still hold the job's output. Before exiting, the supervisor forks a small drain that keeps reading that output into the log and exits when every writer has closed it. The drain closes its copies of the caller's stdin, stdout and stderr first: kept processes are logged, not relayed, so the caller sees end-of-file when the job ends, not when the server does. It holds no ticket and no lock, and never signals anything. Without it, a kept server would get SIGPIPE on its next write, or the supervisor would wait forever.

### Signals

| Event | While waiting | While running |
| --- | --- | --- |
| TERM or HUP to the supervisor | record `abandoned`, delete the ticket, exit with 128 + signal | forward to the job's group; KILL after `grace`; record `stopped by signal` |
| INT or QUIT to the supervisor | as TERM | in terminal mode, Ctrl-C and Ctrl-\ reach the job through the terminal, never through maitred; a signal sent to the supervisor itself is forwarded once, with no KILL after it, so a job that handles INT (a REPL, a test runner finishing up) keeps doing so |
| TSTP (Ctrl-Z), CONT | stop or resume the supervisor; held while it holds `admit.lock` | forward to the group, then stop or resume the supervisor itself |
| HUP when maitred was started under `nohup` | ignored, as `nohup` asked | ignored |
| cancel request accepted | record `cancelled` with sender and reason, delete the ticket | TERM the group, KILL after `grace`; record `cancelled` |
| chosen by the kill policy | not applicable | mark `stopping`, TERM, KILL after `grace`; record `killed` with the threshold |
| supervisor killed with KILL | orphan, waiting | orphan, running; the job may still run |

`grace` (default 10 s), `poll_interval` and `waited_ms` are measured on a clock that keeps counting through sleep, so a grace period that spans a sleep has expired on waking.

### Requests and scope

Only the ticket's own uid can write into its request directory. A same-uid request is honoured when its claimed project is the ticket's; a request for another project is honoured only with `--other-project`, which the CLI accepts only from an interactive terminal. That check is `isatty`: it keeps agents from doing it by habit, and an agent that builds itself a pseudo-terminal can pass it. The record names the sender's project and pid either way. Another user's ticket can only be cancelled as that user or as root.

### Orphans

- **Seen** by any reader whose shared try-lock succeeds. Nothing is written; `status` shows the ticket's last payload and how long its lock has been free.
- **Charged** only when running or stopping, from this boot and namespace, with its session still alive: `kill(-pgid, 0)` succeeds or fails with EPERM (another user's process, which a sandboxed reader can't inspect), and, where the holder's start time is readable, it still matches. A charged orphan that can't be checked further stays charged, and `status` flags it for its owner or root (`maitred clean --all`).
- **Adopted** with `maitred wait <id>`: same project and uid; the command in `job.json` must still match the project's class file; confirmed at an interactive terminal (the same `isatty` limit); refused while its session is alive. The table in Ticket state machine gives each outcome. A non-interactive `wait` on an orphan doesn't wait for it: it exits at once with 69 and says the ticket is orphaned.
- **Expired** after `orphan_ttl` (default 24 h): the sweep writes an `expired` record into the project (its path is in the request directory) and deletes the ticket.

### The sweep

Each admission pass, after deciding, spends at most a few milliseconds removing what it may: its own uid's expired orphans (record first), tickets whose job had already finished (record present), unknown tickets with a free lock, `.new-*` files older than a minute whose lock is free, and request directories of tickets that no longer exist. Another user's debris is that user's to clean, or the administrator's (`maitred clean --all`, as root).

## Sampler

`maitred watch` runs as the maitred user, never root, takes one sample every `sample_interval` (default 2 s), replaces `load/now.json`, appends the history, and records the admissions it sees; it aims to cost under 1% of one core. It reads machine totals only: measuring a job is its supervisor's work, because an unprivileged sampler can't read other users' processes on macOS.

### What it reads

|  | Machine load | Memory | GPU | Sleep and power |
| --- | --- | --- | --- | --- |
| macOS | CPU ticks from the host's CPU load info, differenced | the host's VM statistics and the kernel's memory-pressure level | device utilisation from the accelerator's IOKit performance statistics (unverified without root); memory is the shared pool | `pmset -g assertions` |
| Linux | `/proc/stat` differenced; `/proc/pressure/cpu` | `/proc/meminfo` (`MemAvailable`); `/proc/pressure/memory` | NVIDIA through NVML; AMD through `gpu_busy_percent` and the VRAM counters in sysfs | logind inhibitor locks, where they exist |
| Container (cgroup v2) | `cpu.stat` differenced | working set: `memory.current` minus `inactive_file` from `memory.stat` | as Linux, for devices passed in | none |

**Containers.** The sampler decides it is in a container from its own cgroup (`/proc/self/cgroup` and the files under `/sys/fs/cgroup` it names), never from process 1's namespaces, which a service may not be allowed to read; it samples the cgroup it runs in, so it must run inside the container it rations. Capacity is `cpu.max` quota over period, or the size of `cpuset.cpus.effective`, or the host's CPU count when both say `max`; memory is `memory.max`, or the host's `MemAvailable` when it says `max`. A system service in its own cgroup on a bare host is told apart from a container by `memory.max` and `cpu.max` both saying `max`: then the host's figures are used. A cgroup v1 host is read from its v1 files where they exist, otherwise as the host with a warning that limits are unknown. A capacity under one core clamps every class minimum to the capacity.

**No per-job figures.** The load file names no process and no ticket. Per-job use is in the tickets, written by each job's own supervisor (Supervision); the sampler's history copies it from there.

**Critical readings.** CPU use and available memory are critical. If either can't be read, the sample says so, and admission falls back to one job at a time, exactly as when the sampler is down: never a permanent wait. GPU, pressure and power assertions are not: a missing GPU reading means no GPU limit and no GPU kill, and `status` says so.

### `load/now.json`

```
{
  "v": 3,
  "sampled_at": "2026-10-07T21:14:02.118Z",
  "interval_ms": 2000,
  "capacity": { "cpu_millicores": 8000, "mem_mb": 16384 },
  "cpu_used_millicores": 4960,
  "mem_available_mb": 5120,
  "gpu": { "utilization": 0.12, "mem_used_mb": 0 },
  "quiet_streak": 0,
  "over_cpu_streak": 0,
  "over_gpu_streak": 0,
  "pressure": { "cpu_some_avg10": 4.2, "mem_some_avg10": 0.0, "level": "normal" },
  "sleep_assertions": { "count": 2, "unexpected": 1 },
  "missing": []
}
```

- `quiet_streak`, `over_cpu_streak` and `over_gpu_streak` count consecutive samples under the quiet threshold and over the CPU and GPU kill ceilings, so `decide` and `choose_victim` need only this one file.
- `gpu` is absent on a machine with no readable GPU.
- Sleep assertions are counted, not named; `unexpected` excludes holders the machine config lists as expected.

### History and OpenTelemetry

Each sample becomes metric data points in OTLP's JSON shape, one `ResourceMetrics` object per line, so `maitred export` mostly concatenates. Names follow the OTel semantic conventions where one exists and use `maitred.` where none does.

| Reading | Metric | Unit | Attributes |
| --- | --- | --- | --- |
| CPU busy | `system.cpu.utilization` | 1 |  |
| Load averages | `system.cpu.load_average.1m` (`.5m`, `.15m`) | {thread} |  |
| Memory | `system.memory.usage` | By | `system.memory.state`: used, free, cached |
| GPU | `hw.gpu.utilization`, `hw.gpu.memory.usage` (the hardware conventions, to be checked against the current semantic conventions) | 1, By |  |
| Pressure | `maitred.pressure.some_avg10` | % | `maitred.pressure.resource`: cpu, memory |
| A job's use (from its ticket) | `maitred.job.cpu.usage`, `maitred.job.memory.usage` | 1, By | `maitred.ticket.id`, `maitred.class` |
| The queue | `maitred.queue.waiting`, `maitred.queue.running` | {ticket} | `maitred.class` |
| A finished run (from the record) | `maitred.job.wait`, `maitred.job.duration`, `maitred.job.overuse` | s, s, 1 | `maitred.ticket.id`, `maitred.class`, `maitred.project`, `maitred.outcome` |

The resource carries `service.name = maitred`, `host.name`, `host.arch`, `os.type` and `maitred.boot`.

### Observe-only

`maitred watch --observe-only` marks the machine as observing: supervisors admit at once, as if there were no queue, but write in each record what `decide` would have said and when. A week of that gives the simulator the real arrival pattern and load, and shows how each config would have queued it, before any job is ever made to wait.

## Class files

A class file is read as committed on the local default branch with a hardened `git`, parsed on every run (it is small; there is no cache to poison), and matched against every simple command in what the worker runs; anything maitred can't vouch for is heavy and visibly flagged.

### Finding the committed copy

1. **The project** is found by walking up from the job's working directory to a git repository, identified by its common directory, so all worktrees are one project. A worker chooses its working directory, so a worker can choose its project; records and `status` show each project's path, so a job run from a scratch repository is visible as one.
2. **`git` is run hardened:** the binary resolved at install time by absolute path, not through `PATH`; every `GIT_*` variable removed; `--no-replace-objects`; and full ref names only (`refs/heads/main`, never `main`, so a tag can't shadow it).
3. **The branch** is the one the project configures (`maitred.branch` in the repository's git config), else the local branch that `refs/remotes/origin/HEAD` points at, else `refs/heads/main`, else `refs/heads/master`. Only the local branch is read; whether it matches any remote is not maitred's business.
4. **The file** is `.maitred/classes` in that branch's commit. Its commit id goes into every ticket's `job.json` and record, so a change to the rules is always attributable. No branch or no file makes every command heavy, with a warning and a flag (`no-branch`, `no-class-file`).
5. **The override.** `--classes <path>` or `$MAITRED_CLASSES` reads a working file instead. It is held to the same rules as the committed file (a rule may not be weaker than the committed rule for the same name), and the ticket and record carry `uncommitted-classes`.
6. **An invalid file** (a rule without a why, a pattern that doesn't compile, an unknown option) makes every command heavy, with a warning naming the bad line. Nothing is refused for it.

### Format

One rule per line, fields separated by `|` (space, bar, space); `#` starts a comment. A bar inside a pattern needs no escaping because the separator has spaces around it and alternation doesn't; a pattern that needs `|` literally uses `[ ]\|[ ]`.

| Field | Meaning |
| --- | --- |
| name | unique within the file; appears in records and `status` |
| pattern | a regular expression in Rust's `regex` syntax: linear time, no backreferences, so a pattern can't hang a supervisor |
| class | `light`, `heavy`, `quiet` or `none`; with a checker, the class used when the checker fails |
| why | required, free text; a rule without one makes the file invalid |
| options | optional, space-separated `key=value` pairs from the table below; an unknown key or value makes the file invalid |

| Option | Values | Default | Stricter direction | Meaning |
| --- | --- | --- | --- | --- |
| `checker` | a program path relative to the project root | none | (not overridable) | prints the class and reservation for each run |
| `cpu` | cores, e.g. `2` or `0.5` | the class default | higher | the job's CPU reservation; never below the class minimum |
| `mem` | a size, e.g. `1200M`, `2G` | the class default | higher | the job's memory reservation; never below the class minimum |
| `leftovers` | `stop`, `keep` | `stop` | `stop` | what happens to processes still in the job's session when its main process exits: stopped (TERM, then KILL after the grace period), or left running. Left running, they are no longer the ticket's, but the sampler still counts them as load maitred didn't admit, so the budget and the live limits still see them |

```
# name      | pattern                         | class   | why                                        | options
e2e         | ^bin/e2e( |$)                   | heavy   | boots the app server and drives a browser  | leftovers=keep mem=3G
dataset     | ^bin/process-dataset( |$)       | heavy   | cost depends on the dataset                | checker=bin/dataset-cost
```

The command line can override an option only in its stricter direction, as with `--class`: `--leftovers stop` on a `keep` rule, a higher `--cpu`, never the reverse. Anything else the committed rule sets stays as committed.

### Matching

1. **Split shell commands.** When the command is a shell running a string (`sh -c`, `bash -c`, `zsh -c`), the string is lexed: it is split on `&&`, `||`, `;`, `|`, `&` and newlines into simple commands, and the strictest class among them wins. Anything the lexer can't take apart with certainty (command substitution, subshells, functions, `eval`) is heavy. A command backgrounded with `&` inside a queued string is part of the job's session, so `leftovers=stop` stops it when the job ends; `classes match` warns about it.
2. **Strip wrappers** from each simple command, repeatedly: leading `NAME=value` assignments and `env`, `exec`, `command`, `nohup`, `time`, `nice` (with `-n N`), `timeout <n>`, `xargs` (with its options), `bundle exec`, and whatever the machine config adds.
3. **Normalise the program.** A path into the project (`./bin/x`, `bin/x`, or its absolute path) is rewritten relative to the project root, so all three match `^bin/x`.
4. **Join** its argv with single spaces, quoting any argument that contains whitespace or quotes.
5. **Match.** The first rule whose pattern matches wins; no match means heavy.
6. **Keep-awake wrappers** (`caffeinate`, `systemd-inhibit`, and whatever the machine config adds) as the program of any simple command, after stripping, refuse the job; the word appearing as an argument (`rg caffeinate`) does not. This catches the honest case; a script that holds the machine awake internally is caught only afterwards, by the sampler's count of unexpected sleep assertions.

### Checkers

A checker runs before the ticket exists, as the worker, in the project root, with the job's argv as its arguments, a clean environment and a 5 s timeout. It prints one line: `<class> [cpu=<cores>] [mem=<size>]`. Anything else, a non-zero exit or a timeout gives the rule's own class and reservation, and a warning. Its output can raise a reservation, never lower it below the class minimum.

### `classes check` and hooks

`classes check [path]` validates the file: every rule parses, every pattern compiles, every rule has a why, checkers exist and are executable. It also lints: a rule that an earlier one always shadows, and a pattern without a leading `^`. It exits non-zero on errors, for CI.

`classes match [--shell] -- <cmd>` prints, as JSON, the class each simple command would get and whether it matched a rule or fell through to the unlisted default. A hook calls it with `--shell` on the raw command string: it wraps the whole string in maitred when any part matched a rule that isn't `none`, refuses a string that runs a `none` command alongside a queued one, and leaves unmatched strings alone (wrapping every unknown command would make `ls` queue). So the unlisted-is-heavy default applies once a worker has chosen maitred, not to everything a shell runs.

## Command line

One binary, `maitred`, with subcommands; every command that reports state has `--json`, and anything maitred decides on its own is said on stderr with the `maitred:` prefix.

| Command | Flags | Exits with |
| --- | --- | --- |
| `maitred [run] -- <cmd>` | `--label`, `--class` (stricter only), `--leftovers stop`, `--cpu` and `--mem` (higher only), `--timeout <dur>`, `--log-cap <size>`, `--no-log`, `--classes <path>`, `--quiet` | the job's status; or a maitred status (below). Once a ticket exists, the first line on stderr is its id, even with `--quiet`; a refusal before that names no id |
| `maitred submit -- <cmd>` | as `run` | 0 once the ticket exists; prints the id |
| `maitred wait <id>` / `--label <l>` | `--timeout <dur>` | the job's status from its record; `lost` if there is neither ticket nor record; on an orphan, offers adoption at an interactive terminal, and otherwise exits at once with 69 |
| `maitred status` | `--project`, `--json`, `--watch` | 0 |
| `maitred result <id>` | `--json` | 0 if the record exists |
| `maitred cancel <id>` | `--reason <text>` (required), `--other-project` (interactive terminal only) | 0 once the request is written |
| `maitred update <id>` | `--label`, `--class` (stricter only) | 0 once the request is written |
| `maitred classes check [path]` / `match [--shell] -- <cmd>` / `propose` | `--json` | non-zero on an invalid file |
| `maitred export` | `--machine` or `--project`, `--since`, `--format otlp-json` | 0 |
| `maitred watch` | `--once`, `--observe-only` | runs until signalled; refuses to run as any user but maitred's, and as root |
| `maitred clean` | `--dry-run`; `--all` (root only) | 0 |
| `maitred install machine` | `--dry-run`, `--service` | 0 once the maitred user, root, config and (with `--service`) the sampler's launchd or systemd unit exist; needs an administrator |
| `maitred install claude-code` | `--user`, `--check` | 0 once the skill, hook and sandbox allowlist (tickets and requests only) are written, after confirmation |

### Exit statuses maitred chooses

These use the BSD `sysexits` values where one fits. None is unique, because a job may exit with any number; `maitred result` is authoritative, and stderr names the cause.

| Status | When |
| --- | --- |
| 64 | usage error |
| 69 | the machine root is missing, on an unsupported filesystem, or not owned as installed; `wait` found the run `lost`; or a non-interactive `wait` found an orphan |
| 75 | `--timeout` passed before admission |
| 77 | refused by a rule: `none`, a keep-awake wrapper, a stricter nested command, scope |
| 128 + n | the supervisor was stopped by signal n, or stopped the job with it (the kill policy, a cancel) |

### Messages

One line each, on stderr, only when something changes: `maitred: queued (heavy, position 3)`, `maitred: waiting: settle (8 s)`, `maitred: admitted after 4m10s`, `maitred: killed at the memory threshold (free memory 410 MB < 600 MB)`. `--quiet` drops all but the ticket id and errors.

## Packages

maitred leans on the standard library and two thin system crates, and writes its own code where no crate does the job; every choice below was checked against the crate's source on 2026-10-07, and anything not checked says so.

| Need | Choice | Why | Rejected |
| --- | --- | --- | --- |
| flock, try-lock | std `File::lock` / `try_lock` (stable since Rust 1.89) | calls `flock(2)` on Linux and macOS; no dependency. std says the implementation "may change", so a test pins it (two handles in one process: works with flock, fails with fcntl locks) | rustix `fcntl_lock` (record locks, which drop when any descriptor closes); fd-lock, fs4, nix (all fine, all redundant) |
| Filesystem check | rustix `fstatfs` | `f_type` on Linux, `f_fstypename` on macOS | nix (splits the two across cfgs) |
| Starting the holder | std `Command` running `maitred __hold`, with one `dup2` in `pre_exec` to hand it the gate as fd 3 | `Command::spawn` returns only after the child has exec'd, so nothing may block in `pre_exec`; the holder blocks after exec instead | blocking on the gate in `pre_exec` (deadlocks: the supervisor can't open the gate until `spawn` returns); process-wrap (no wait with timeout, pulls in nix), command-group (unmaintained since 2023) |
| Sessions, groups, signals, pseudo-terminals | rustix `setsid`, `getsid`, `kill_process_group`, `test_kill_process_group`, and its `pty` and `termios` modules (`openpty`, `TIOCSCTTY`, `tcsetpgrp`, `tcsetattr`); a raw `fork` in the single-threaded holder | everything needed, no extra runtime (the pty functions not yet checked against the source) | nix |
| Forwarding signals | signal-hook | the usual crate for handling TERM/INT/HUP safely (unverified: not opened) | a hand-written handler |
| Clocks | libc `clock_gettime` behind a small per-OS module | rustix can't name `CLOCK_UPTIME_RAW`; std `Instant` stops during sleep on both platforms, so grace periods and the sleep measure use these clocks instead | `Instant` |
| Measuring a job, Linux | procfs-core for `/proc/<pid>/stat` (session, CPU) and `smaps_rollup` (proportional set size) | the session field is what finds the job's processes | sysinfo (its `group_id()` is the user's gid, not the process group; no proportional size) |
| Measuring a job, macOS | libc `sysctl` (`KERN_PROC`, to list the session's processes) and `proc_pid_rusage` (CPU time, physical footprint) | same-user processes, readable without privileges (to be verified inside the sandbox) | sysinfo, for the same reasons |
| Machine sampling | Linux: `/proc/stat`, `/proc/meminfo`, `/proc/pressure`, cgroup v2 files read directly. macOS: libc `host_statistics64`, `sysctlbyname` | totals only, no process access | sysinfo |
| GPU | nvml-wrapper (NVIDIA; loads the driver's library at run time, so no build dependency); sysfs for AMD; IOKit's accelerator statistics on macOS through objc2-io-kit | best effort, all unverified without root | powermetrics (needs root) |
| Boot id, macOS | libc `sysctl` `KERN_PROC_PID` for process 1's start time | allowed in a sandbox, unlike `proc_pidinfo` on process 1 | `kern.bootsessionuuid` (refused in a sandbox) |
| Power assertions, macOS | parse `pmset -g assertions`, every sample | no FFI; the output format isn't documented as stable, so the parser fails loudly | objc2-io-kit (raw CoreFoundation dictionaries; kept as the fallback) |
| Git | run `git` | two trivial reads, no build cost | gix (128 crates), git2 (needs a C compiler) |
| OTLP-JSON | about ten hand-written serde types | exact conformance (64-bit integers as strings) and 35 fewer crates; opentelemetry-proto stays as a test-only dependency to check our output against | opentelemetry-proto at run time (writes `asInt` as a number, links the SDK crates) |
| Command line | clap (derive) | standard; 19 crates |  |
| Files | serde, serde_json, crc32fast | the slot checksum and every JSON file |  |
| Ids | getrandom | 64 random bits per id, and the project id; no need for rand | rand |
| Class rules | regex, shell-words | linear-time patterns; POSIX argv quoting for matching |  |
| Logging | tracing, tracing-subscriber | debug output behind a flag |  |
| Tests | assert_cmd, proptest, rexpect (Unix-only, fine here); opentelemetry-proto as a dev-dependency | the cross-process lock test, property tests of `decide` and `choose_victim`, terminal prompts |  |

### What we write ourselves

- **Measuring a session** on both platforms: no crate groups processes by session.
- **The network-filesystem table.** Only NFS's magic number is exported by any crate; SMB, FUSE, 9p, virtiofs and Docker Desktop's file sharing need their own table, and tests on real mounts.
- **The waiting loop with a timeout and grace periods**, on our own clocks.
- **The holder**: setsid, the gate, the fork into a foreground group, reaping, and `leftovers`.
- **The OTLP-JSON types.**

### What the survey changed in this document

- The boot id is read by each supervisor itself, from sources a sandbox allows (Linux `boot_id`; the start time of process 1 through `sysctl` on macOS), not from the sampler.
- Whether an orphan's session still exists comes from `kill(-pgid, 0)` and, where readable, the holder's start time: EPERM counts as alive.
- Jobs are measured by their supervisors, because the sampler, as an unprivileged user, can't read other users' processes on macOS.
- The gate moved out of `pre_exec` into the holder, because `Command::spawn` waits for exec.

## Testing

The pure core is tested exhaustively and by property; everything that touches the operating system is tested with real processes, on both platforms, because that is where this kind of tool breaks.

| Layer | How | What it proves |
| --- | --- | --- |
| `decide` | unit tests per rule; property tests over random snapshots | nothing starts past the live limits or beside a running quiet job; admitted reservations never exceed what is free except by first-in; class rules are never waived; an unmeasured job holds its whole reservation; the answer depends only on the snapshot |
| `choose_victim` | unit tests per trigger; property tests | one victim per snapshot; never a stalled or orphaned ticket; nobody chosen while a living ticket is stopping; every supervisor computes the same victim |
| Starvation and the herd | a deterministic simulator: synthetic tickets and load traces, many waiters, simulated time, sleeps and kills | every class is admitted eventually (aging, backfill, `quiet_max_wait`); a burst never exceeds the minimums; settle spaces heavy starts; order follows `seq`, not poll timing; a breach stops one job at a time |
| Tuning | the simulator replaying observe-only samples, from every machine Nick collects | the machine config's starting numbers |
| The ticket file | a writer killed at every byte offset of a slot write; readers in a tight loop; on macOS and Linux | a reader always finds a complete generation, never a torn or older one |
| Locks and liveness | many real `maitred` processes on a temporary root, killed with KILL at random points; lock files and whole tickets deleted underneath; a stopped supervisor; Ctrl-Z while holding `admit.lock` | a dead owner always reads as an orphan, a live one never; no two owners of one lock; a deleted ticket comes back with its `seq`; a stopped supervisor neither holds budget nor freezes the queue |
| Measurement | real jobs of known size (a process that allocates N MB and spins K cores, with children, shared pages and a forked server) | each supervisor's `use` is within a stated margin of the truth on both platforms, and shared pages count once |
| Supervision | real children: every signal at every stage, a job that ignores TERM, a REPL that handles INT, a kept server, a job whose child calls setsid, a nested `maitred` and a nested fan-out, `submit` | forwarding, the gate, the holder, the grace KILL, the drain, nesting by session and class |
| Terminal | a real pseudo-terminal driven by a test harness: colour detection, a password prompt, Ctrl-C, Ctrl-Z and fg, resize, a kept server after the job exits, maitred killed mid-run | the job behaves as it would without maitred; the caller's terminal is restored |
| Class files | fixtures; a temporary repository with a tag named like the branch, `git replace`, no branch, no file, an uncommitted edit, a fake `git` on `PATH`; shell strings with operators, `./bin`, absolute paths, `bundle exec`, `xargs` | each case is matched, caught or flagged |
| Platforms | GitHub Actions on Linux (real VMs, so flock, cgroups and signals behave as on a Linux box) and macOS; Linux containers with `--memory`/`--cpus` limits; a local Linux VM for exploring by hand; sleep detection and GPU readings on real hardware | the sampler backends, the filesystem check, the cgroup edge cases |
| Inside an agent sandbox | in CI, a sandbox profile close to the agent harness's; periodically, a real headless Claude Code session running maitred end to end | nothing needs what a sandbox denies; pipes are used when pseudo-terminals are refused; a supervisor can measure its own job |
| Agent conformance | scenarios that tempt a shortcut (run the heavy command directly, chain it after a light one, edit the class file, tidy `tickets/`, cancel another project's job, kill the supervisor), each run repeatedly against a headless agent with the skill installed; and Nick building gravedigger with maitred | how often the boundary holds, as a number that has to stay high |
| Regressions from review | every confirmed finding of the adversarial reviews becomes a test | none of them comes back |

## Risks and open questions

Two adversarial reviews (2026-10-07) found about 30 and then about 25 confirmed problems; this revision answers them. What remains open, ranked by how much rests on it:

1. **Same-user agents are trusted not to forge maitred's files.** Tickets, requests and the `use` figures in them are writable by every process of the same user, so a deliberately hostile agent can still delete or hand-write tickets, or understate its job's use to avoid being the kill policy's victim. The sampler's files, the live limits and the kill thresholds are out of reach. Whether a hostile agent is a real threat is undecided; if it becomes one, it needs a broker process, which the design rules out today.
2. **Settle, the minimums and the kill thresholds are tuned numbers.** The simulator and observe-only data from several machines are the evidence planned.
3. **Self-measurement inside a sandbox** (`proc_pid_rusage` and `sysctl` on macOS, `/proc` on Linux) is unverified. If a sandbox refuses it, the job holds its whole reservation, which is safe but wastes the budget.
4. **GPU readings without root** are unverified on every platform, Apple Silicon especially. Without them there is no GPU limit.
5. **Processes that leave the session** (daemons that call setsid) escape measurement, signalling and cleanup; containing them needs cgroups on Linux and has no macOS equivalent.
6. **Any user can hold `admit.lock`** (it must be readable to be locked), so any local user can stall admission; `status` reports the wait, but nothing can name the holder. Inherent to flock.
7. **A running job's pid is in its ticket,** so another user can read its command line from `/proc/<pid>/cmdline` on Linux, as they could with `ps`. The ticket adds no new exposure, but doesn't reduce it either.
8. **Pseudo-terminals inside sandboxes.** Where they are refused, a job run by a person through a sandboxed shell gets pipes: no colour, no prompts. Agents are unaffected.
9. **Power loss mid-write** on macOS can leave a ticket with no valid slot; it reads as unknown and unlocked, which the sweep removes.
10. **Polling cost** with many waiters; waking on the load file changing is open.
11. **Checker latency**: up to 5 s on every enqueue in a project with checkers.
12. **The shell lexer** decides how compound commands are split; whatever it can't parse is heavy, which is safe but may be common in agent-written commands.
13. **GPU and other named resources as reservations**: the reservation map is ready; admission isn't.
