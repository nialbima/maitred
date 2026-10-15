<!--
  Synced from the maitred architecture doc on claude.ai (rev 76, 2026-10-15):
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

- **maitred-core** holds the types (ticket, snapshot, reservation, record), `decide`, `choose_victim`, the state machine's transitions, class-rule matching and the OTel data-point types. No file, clock, process or environment access: everything it needs arrives as an argument. That is what makes admission and the kill policy testable by property and replayable by the simulator.
- **maitred** holds the commands and the layers that face the operating system: `store` (the machine root and project files, slots, locks), `platform` (one backend each for macOS, Linux and cgroups: clocks, signals, sessions, measuring, sampling) and the command modules on top, including the hidden `__hold` and `__drain` entry points the supervisor re-executes itself as.

Every process is one of three kinds, and none outlives its purpose:

| Process | Started by | Runs as | Lives | Holds |
| --- | --- | --- | --- | --- |
| supervisor (`run`, or the detached child of `submit`) | a worker | the worker's user, in its sandbox | until its job's record is written | its ticket's lock; briefly, `admit.lock` |
| holder (`maitred __hold`) | the supervisor, once admitted | the worker's user, in the same sandbox | while any process of the job's session lives | the job's session, as its leader; while it is killing its job, its stop marker's lock; briefly, `admit.lock` |
| drain (`maitred __drain`) | a supervisor whose rule keeps leftovers | the worker's user | until the kept processes close their output | nothing |
| sampler (`watch`) | an administrator, or launchd or systemd as installed | the maitred user, never root | until stopped | `sampler.lock` |
| short commands (`status`, `cancel`, `wait`, `result`, `export`, `classes`) | anyone | the caller | one call | nothing for longer than a read |

Every helper is started by re-executing the maitred binary, never by a bare `fork`: the supervisor may have threads, and a child forked from a threaded process can deadlock on a lock another thread held at the fork.

## Ticket state machine

A ticket has three written states, an end, one marked state and one computed state. `waiting`, `admitted`, `running` and its end are written by its supervisor. `stopping` is marked by the job's holder, in a file of its own beside the ticket, because the holder is what carries out a kill (Kill policy). Orphaned is never written, only seen.

![ticket states: solid boxes are written by the ticket's supervisor, dashed ones are only seen](docs/ticket-states.svg)

| From | To | When | Written by |
| --- | --- | --- | --- |
| (none) | waiting | the ticket is created, locked and renamed into place, with the next `seq` | its supervisor, under `admit.lock` |
| waiting | admitted | `decide` admits it | its supervisor, under `admit.lock` |
| admitted | running | the holder is started and waits at its gate; it reports its pid, session and start time; the supervisor writes them; only then is the gate opened and the job forked and exec'd | its supervisor |
| running | stopping | `choose_victim` names it | its holder, under `admit.lock`: it creates and locks the stop marker |
| stopping | ended | the job's processes are gone and the sampler has published a sample whose interval began after that: record first, then the ticket, then the marker | the supervisor, then the holder |
| running | ended | the job exits, is cancelled or signalled: record first, then delete | its supervisor |
| waiting or admitted | ended | cancelled, timed out, signalled, or the spawn failed: record first, then delete | its supervisor |
| any | orphaned (computed) | its supervisor dies; the kernel frees the lock | nobody: readers see the free lock |
| orphaned, waiting | waiting | adopted (same project and uid, the command still matches, confirmed interactively) | the adopter, who now holds the lock |
| orphaned, admitted | waiting | adopted, keeping its `seq`. No job ran: a holder at its gate exits when its supervisor's end of the gate closes | the adopter |
| orphaned, running or stopping, record present | removed | the job had finished; only the deletion was lost | any later supervisor of the same uid in the same project (the sweep), or `wait` |
| orphaned, running or stopping, session alive | (unchanged) | adoption is refused: the job is still running, and adopting would run it twice. Its holder still carries out a kill | nobody |
| orphaned, running or stopping, session gone, no record | ended + new waiting ticket | adopted: the record says `interrupted`; the command is queued again as a new ticket | the adopter |
| orphaned | removed | past `orphan_ttl`, and not running with a live session: the sweep writes an `expired` record into the project first | the sweep of the next supervisor of the same uid in the same project |

Three invariants follow, and the tests check each: a ticket's file only changes while someone holds its lock; no job runs whose session isn't already in its ticket; and no ticket disappears before its record exists. A record is created exclusively (written under a temporary name, then linked into place, which fails if one exists), so two writers can never both write one. So `wait` on an id always ends: with the record, or, if there is neither ticket nor record, with the outcome `lost`.

## Files and the write protocol

Every file has one owner, one stated mode and one writer; a ticket is rewritten in place, into one of two slots, so a reader always finds one complete copy; and every lock is checked to still belong to the file at its path.

### Where things live, and who owns them

`maitred install machine`, run once by an administrator, creates a dedicated system user (`_maitred` on macOS, `maitred` on Linux), the machine root, the machine config and the sampler's service. Nothing below is created on first use by whoever happens to run first.

| Path | Owner | Mode | Written by |
| --- | --- | --- | --- |
| machine root: `/Users/Shared/maitred` (macOS), `/var/lib/maitred` (Linux) | maitred user | `0755` | nobody after install |
| `admit.lock`, `sampler.lock` | maitred user | `0644` (a lock needs only a read-only open) | never written; only locked |
| `tickets/` | maitred user | `1777`: any user creates; only a file's owner (or root) deletes it | supervisors, each its own ticket; holders, each its own stop marker |
| `tickets/<id>.tk` | the ticket's owner | `0644` | its supervisor |
| `tickets/<id>.stop` | the ticket's owner | `0644`, created exclusively, locked by the holder while it exists | its holder |
| `requests/` | maitred user | `1777`, like `tickets/` | nobody; holds one directory per ticket |
| `requests/<ticket-id>/` | the ticket's owner | `0700`, created before the ticket: only the same user can send a request | the owner (its `project` file); senders of the same uid |
| `pools/<pool>/<n>` | maitred user | `0755` dir, `0644` empty files, one per slot, made at install from the machine config | never written; a slot is taken by locking its file |
| `load/` and everything in it | maitred user | `0755` dir, `0644` files | the sampler only |
| machine config: `/Library/Application Support/maitred/config.toml`, `/etc/maitred/config.toml` | root | `0644` | an administrator |

These modes separate users. Between processes of one user, which is what agents of one person are, they separate nothing: any of them can delete or rewrite another's tickets, and maitred makes no claim otherwise (design, Principle and threat model). There is no per-user state outside the machine root and the projects: what the sweep needs to find a ticket's project sits in `requests/<ticket-id>/project`, readable only by the ticket's owner. `maitred install claude-code` adds exactly two machine paths to the agent sandbox's write allowlist, `tickets/` and `requests/`, after showing the change; the sampler's files stay unwritable. The Linux root lives under `/var/lib`, not `/var/tmp`, so no temporary-file cleaner ages out its lock files; the install also drops a `tmpfiles.d` exclusion for anyone who moves it. The machine root's path and the `--machine-config <path>` flag can be changed only in debug builds; both are compiled out of releases.

**No machine root, no queue.** When the machine root doesn't exist, `maitred -- <cmd>` warns once and runs the command straight through, with no ticket and no record, so scripts and hooks that call maitred keep working on CI runners and machines nobody has set up. A root that exists but is on a network filesystem or not owned as installed is an error (69): that is a broken install, not a missing one.

**Slot pools.** A pool is declared in the machine config with a name, a size and the environment variable that carries the number, such as `test_env`, 16, `TEST_ENV_NUMBER`. A slot is taken by holding an exclusive lock on its file, so a free lock is a free slot, with nothing to clean up: a job's slot is freed by the kernel when the last process holding it ends, however it ends. The supervisor takes the lowest free slot under `admit.lock` at admission, writes the number into its ticket, and hands the locked descriptor to the holder, which keeps it until the job's session is empty. So a running orphan whose session is alive still holds its slot, and two live jobs never share a number. Agents can't delete the slot files: they are the maitred user's, in a directory only it writes.

### Locks that can't be fooled

- **Liveness reads take a shared lock.** A reader tests a ticket with `flock(LOCK_SH | LOCK_NB)`: it fails while the owner holds its exclusive lock, and many readers can test at once without making a dead ticket look alive to each other. Only an adopter or the sweep takes `LOCK_EX` on someone else's ticket.
- **Every lock is re-checked against its path.** After taking any lock (`admit.lock`, `sampler.lock`, a ticket), the holder compares the locked descriptor's inode with the inode now at the path, and starts again on a mismatch. A file deleted and recreated underneath can't produce two owners.
- **`admit.lock` is taken with a deadline.** A supervisor tries every 50 ms; after 10 s it prints how long the lock has been held and keeps trying. flock can't say who holds it; `status` reports the wait.

### The ticket: two slots and a lock

A ticket is `tickets/<id>.tk`, mode `0644`, a file of two fixed 4 KiB slots:

```
magic "MTRDTICK" (8) | major u8 | minor u8 | class u8 | state u8 | seq u64 | cpu_millicores u32 | mem_mb u32
  | last_pass u64 | admitted_at u64 | sid i32 | pgid i32 | holder_start u64 | generation u64 | length u32 | crc32 u32
  | JSON payload | zero padding to 4096
```

- **The checksum covers everything after the magic but itself,** so a slot torn anywhere fails it.
- **The fixed header carries everything another reader needs to place, charge and judge the ticket:** class, state, `seq`, reservation, when it last ran a pass, when it was admitted, and its session and holder. So a reader that can't understand a newer payload still charges it at its own reservation, sees whether it has stalled, and can check its session (Versions). The magic never changes; `major` does.
- **Writing.** The owner writes generation g+1 into the slot that holds g-1, then `fdatasync` on Linux (`fsync` on macOS, which orders nothing on the disk: after a power loss both slots may fail, and the ticket then reads as unknown and unlocked, which the sweep removes). The slot holding g stays intact throughout.
- **Reading.** Read both slots in one 8 KiB read, keep those that pass, take the higher generation. If neither passes, read again, up to three times; a reader that still finds no valid slot treats the ticket as unknown (Versions).
- **Creating without a gap.** Create `requests/<id>/` (mode `0700`; if it already exists and isn't ours, pick a new id), write its `project` file, then create `tickets/.new-<id>`, lock it, write the first slot and rename it to `<id>.tk`. A rename keeps the inode, and the lock lives on the inode, so the ticket appears already locked.
- **Still mine?** On every pass a supervisor checks that the inode at its ticket's path is the one it holds locked, and that its request directory and `project` file are still there. If either was deleted (an agent tidying `tickets/` or `requests/`), it recreates it under `admit.lock`, keeping its `seq`, sets `recreated_at`, and warns; the record notes it. While any ticket's `recreated_at` is recent (within the stall limit), `decide` admits nobody and `choose_victim` chooses nobody, because other deleted tickets may not have come back yet.
- **Ids.** `<unix-ms, hex>-<64 random bits, hex>`. Queue order is `(seq, id)`, so two tickets that end up with one `seq` still sort the same way for every reader.

### The ticket's payload

```
{
  "v": "4.0",
  "id": "0192f3a1c2e0-9f3c2a7e5b1d4c88",
  "seq": 4183,
  "boot": "…",
  "pidns": "4026531836",
  "user": 501,
  "project": "7c41e09a5d2b… (a hash of the project's git common directory)",
  "class": "heavy",
  "reservation": { "cpu_millicores": 2000, "mem_mb": 1200 },
  "flags": ["edited-classes"],
  "state": "running",
  "waited_ms": 41200,
  "last_pass": "2026-10-07T21:14:02Z",
  "admitted_at": "2026-10-07T21:13:58Z",
  "recreated_at": null,
  "leader": { "pid": 48210, "sid": 48210, "pgid": 48211, "start": "…" },
  "use": { "cpu_millicores": 1850, "mem_mb": 760, "window": ["2026-10-07T21:14:00Z", "2026-10-07T21:14:02Z"] },
  "slots": { "test_env": 3 }
}
```

- `seq`: under `admit.lock`, one more than the larger of the largest `seq` in any ticket's header and the sampler's published high-water mark (`seq_high` in `load/now.json`), so deleting every ticket can't restart the count and put a newcomer ahead of tickets that come back.
- `boot`: read by the supervisor itself: Linux `/proc/sys/kernel/random/boot_id`; on macOS, the start time of process 1 from `sysctl` `KERN_PROC_PID`, which a sandbox allows. No dependence on the sampler.
- `pidns`: the pid namespace on Linux; empty on macOS. Process ids are only compared within one boot and namespace.
- `project`: the first 16 bytes of SHA-256 over the real path of the project's git common directory (or of its `.maitred/` outside git). Every worktree gets the same value, and two racing first runs can't disagree, since nothing is stored. The label is never shown to other users; `status` shows other users' tickets as user, class, reservation and state only.
- `waited_ms` is the owner's own clock. `last_pass` is when the owner last started a pass, waiting or running; it is written before contending for `admit.lock` and again on every poll interval of the wait for it, so a slow lock never makes a live supervisor look stalled.
- `leader`: once running, the holder's pid, the session it leads, the job's first process group and the holder's start time. A session counts as this job's only if the holder's start time still matches, which defeats pid reuse; a `pgid` or `sid` of 0 or 1 is never accepted.
- `use`: the job's measured use and the interval it was measured over, written by the supervisor on every running pass (Supervision). It is as trustworthy as the rest of the ticket, which is to say written by the same user as the job; the live limits rest on the sampler's totals, never on it.
- `flags`: how this run differs from an ordinary one, such as `edited-classes` (the class file differed from the committed copy) or `recreated` (the ticket was deleted under it).

### Requests

`requests/<ticket-id>/`, created by the ticket's owner with mode `0700` before the ticket itself, holds `project` (the real paths of the project's working tree and its git common directory) and the requests: `<request-id>.json`, written once (temporary name, then rename), with `kind` (`cancel`, `update` or `nested`), the change, the sender's project, working directory, ticket (if it has one) and pid, and a reason. The target's supervisor applies or rejects each one, notes it in the record, and deletes it. A `nested` note is how a nested run tells its parent it ran, so the parent's record can list it. Another user can't write here at all; a person who needs to cancel another user's job does it as that user (`sudo -u`) or as root.

**The stop marker.** `tickets/<id>.stop` is created exclusively by the job's holder when `choose_victim` names it, and holds the threshold and the time of the kill. The holder keeps it locked until the job is gone and the sample after has been published, then deletes it. A marker whose lock is free belongs to a dead holder, and readers ignore it.

### Project files

A project's records live in its working tree, `.maitred/records/<id>/`, which the install adds to `.gitignore`. Not the git directory: some agent sandboxes make `.git` read-only (Codex's does by default). Each worktree keeps its own records; `wait` and `result` look in every worktree that `git worktree list` names. Outside git, the project is the nearest directory holding `.maitred/`; with none, maitred refuses with a message saying to create one (78). If the records directory can't be written, maitred refuses before any ticket exists (73), since a run it can't record is a run `wait` can never finish.

| File | Holds | Written |
| --- | --- | --- |
| `job.json` | command line, working directory, environment keys, resolved class, the class file's and checker's hashes and whether each differed from the committed copy | once, at enqueue |
| `record.json` | the ticket's last payload, start and end, exit status or signal, how it ended (completed, cancelled with sender and reason, timed out, killed with its threshold, interrupted, holder lost, expired, lost), sleep seconds, overuse, nested runs, the samples around it, flags | once, before the ticket is deleted, created exclusively |
| `log` | head and tail of the job's output, capped (default 64 KiB each) | while the job runs, and by the drain afterwards |

### The sampler's files

All in `load/`, written only by the sampler as the maitred user:

- `now.json`: the latest sample, replaced by temporary name and rename.
- `history/<date>.jsonl`: OTel data points, one file per day, deleted after the retention period.
- `admissions.jsonl`: each admission the sampler saw, for settle; kept one hour.

### Versions

Every file carries a version. A ticket whose payload a reader can't understand (a newer minor or major) is placed and charged from its fixed header, which carries everything that needs. A ticket with no valid slot at all is unknown: while its lock is held it is charged at the heavy minimum and placed last; once its lock is free, it is charged nothing, and the sweep of a supervisor of the same uid deletes it (there is no project to write a record into). The magic and the fixed header never change within a major version; a new major is a migration with its own release note. Two binaries of different versions on one machine may still run different `decide` and `choose_victim` logic over the same files; the machine config can name a minimum version, below which a supervisor refuses to start.

## Admission

Admission is one pure function, `decide(snapshot, me) -> Decision`, in the core module: no I/O and no clock, so every waiter computes the same answer from the same snapshot, and tests and the simulator can drive it with any snapshot.

### One pass

A waiting supervisor, every `poll_interval` (default 2 s, with up to 20% jitter):

1. Writes `last_pass` into its own ticket (its own lock is enough) and checks the ticket and its request directory are still its own (Files).
2. Takes `admit.lock` (with the deadline and inode check above), rewriting `last_pass` every poll interval while it waits, so a long wait for the lock never makes it look stalled. Its signal thread defers a Ctrl-Z that arrives while it holds the lock until the lock is released, so a stopped supervisor never holds it.
3. Builds the snapshot: every ticket's header, payload and liveness (a shared try-lock each), every stop marker and whether its lock is held, `load/now.json` and its age, the last heavy admission (the latest of `load/admissions.jsonl` and the `admitted_at` of living heavy tickets), the machine config, and `now`.
4. Runs `decide`.
5. On **Admit**: writes its ticket as `admitted`, takes the lowest free slot of each pool its rule asks for and writes the numbers into the ticket, all still holding the lock, so the next snapshot already charges it; releases the lock; starts the job.
6. On **Wait(reason)**: writes `waited_ms` and the reason; releases the lock; prints the reason if it changed; sleeps.

### Inside `decide`

All CPU figures are millicores and all memory figures megabytes; the sampler converts its fractions before writing.

```
charged = living tickets that are admitted or running
        + orphaned running tickets of this boot and namespace whose session is alive   # reservation only
        + locked tickets with no readable slot (heavy minimum)
waiting = living waiting tickets that aren't stalled, by (seq, id)

if any ticket's recreated_at is recent:
    wait(tickets being recreated)
if load is stale, or a critical reading (cpu, memory) is missing:
    admit me only if charged is empty and I am first in waiting      # plain-lock fallback
    else wait(no reliable load)
if mem_available < mem_floor or cpu_pressure > cpu_ceiling or gpu_used > gpu_ceiling:
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
    else if no living ticket is charged and nothing admitted yet in this walk and allowed(t, ...):
        if t is me: admit                                               # first in: waives only the budget
        stop the walk
    else if t is quiet and is due (past quiet_max_wait, not past quiet_max_wait + quiet_hold):
        stop the walk                                                   # let the machine drain for it
    else if t is aged and allowed(t, ...) and its hold hasn't lapsed:
        free -= reserve(t)                                              # only the budget keeps it out: hold its place
wait(the first rule that held me)
```

| Term | Exact meaning |
| --- | --- |
| `reserve(t)` | `max(t.reservation, class_min[t.class])` per dimension, capped at capacity. It fits free only if, besides cores and memory, each pool it asks a slot of has a free one: a slot whose lock is free and that no ticket admitted earlier in this walk has taken on paper |
| `used` | a running ticket's use as the sampler copied it into the current sample (`jobs` in `load/now.json`), which it does only from a measurement whose window overlaps the sample's own interval; otherwise 0, so the ticket holds its whole reservation. Taking both terms from one sample keeps a job from falling out of both. Orphans and unknown tickets count as using nothing |
| `cpu_pressure` | runnable work per core, from the load file (Sampler); not utilisation, which reads 100% for one healthy build on every core |
| allowed | **light:** always. **heavy:** fewer than `max_running.heavy` charged or admitted on paper, and `now - last_heavy >= settle`. **quiet:** no living ticket charged or admitted on paper, and `quiet_streak >= 2` in the load file; or it has waited past `quiet_max_wait`, in which case the streak isn't needed and its record is marked `tainted` |
| aged | waited past `aging` (any class). An aged ticket that its class allows but the budget doesn't holds its reservation out of `free`, so later tickets can only fill around it, and it gets in as soon as the budget it holds is really free. The hold lapses after `aging_hold` more, so a ticket that can never fit doesn't block everyone behind it for ever. One that its class rules keep out (settle, `max_running`) holds nothing, since holding budget wouldn't let it in sooner |
| due quiet | past `quiet_max_wait`, a quiet ticket stops the walk so the machine drains for it; after `quiet_hold` more it stops doing so, and waits for the first moment no living ticket is charged |
| first in | when no living ticket is charged, the first ticket that its class allows gets in even if its reservation exceeds the budget. A charged orphan doesn't count: its load is in the sample, and an orphan that never ends must not keep every large job out for ever. It never overrides a class rule or the live limits |
| stalled | a waiting ticket whose `last_pass` is older than 5 poll intervals: its supervisor is stopped or stuck, so it neither takes budget nor blocks anyone; it is admitted again when it next runs a pass. Nobody is judged stalled until the stall limit has passed since the machine last woke (`woke_at` in the load file), so after a sleep nobody jumps the queue while the others are still waking |
| orphan charge | a running orphan whose session is alive is charged its reservation, but its class's barrier doesn't apply: an orphaned quiet job doesn't stop the queue, and `status` says why it is charged |

### Settle and the minimums

The minimum reservation per class bounds a burst within one pass; settle spaces heavy starts across passes, so each new job's ramp-up shows in the samples before the next one starts. The load figures themselves are fresh (CPU over the last 2 s interval, memory now): the lag settle covers is the job's own ramp-up, not measurement. Both numbers come from the observe-only data via the simulator. With no reliable load, admission is one job at a time anyway.

### Kill policy

Part of v1. The kill policy is carried out by holders, not supervisors: a holder is never stopped by job control (it is the session's leader, not in the job's foreground group), and it outlives its supervisor, so a job whose supervisor is stopped, stuck or dead can still be chosen and stopped. Each running job's holder, every `poll_interval`:

1. **Trigger.** The load file is fresh and isn't the first sample after a sleep, no live stop marker exists (one whose lock is held), no ticket was recently recreated, and one of: `mem_available < kill_floor` (below the admission floor); `over_cpu_streak >= cpu_kill_samples`; `over_gpu_streak >= gpu_kill_samples`. The sampler keeps both streaks, counting consecutive samples over `cpu_kill_ceiling` (CPU pressure) and `gpu_kill_ceiling`, and resets them after a sleep, so a few seconds of saturation, or the burst every machine makes on waking, never trips. More triggers (database locks, timeouts) are further thresholds of the same shape.
2. **Choose, under `admit.lock`.** `choose_victim(snapshot, trigger)` is pure, like `decide`, and reads per-job use only from the sample (`jobs` in `load/now.json`), never from tickets that change between passes, so every holder computes the same victim from the same sample. Candidates are the running tickets of this boot and namespace whose holder is alive (its pid and start time still match), living or orphaned. For memory or CPU: the candidate furthest over its reservation on that resource, if any is over; otherwise the most recently admitted heavy candidate, then light, and quiet only past the critical threshold (`mem_critical`, or the CPU streak at twice its limit). For GPU, v1 has no per-job figure: the recency order alone. If the victim is this holder's own job, it creates and locks its stop marker before releasing `admit.lock`; if not, it releases and does nothing, and the victim's holder reaches the same answer on its own pass.
3. **Act.** Only the victim's holder signals: TERM to every process of the session, then CONT, so a stopped job gets to run its TERM handler, then KILL after `grace`. It keeps the marker until the session's processes are gone and the sampler has published a sample whose interval began after that, so the next choice sees the machine without it; if the load file goes stale meanwhile, it waits at most 3 intervals more. It reports the kill to its supervisor, which writes the record (`killed: memory`, `killed: cpu`, `killed: gpu`); if the supervisor is gone, the holder writes the record itself (records are created exclusively, so there is never a second). Then the marker is deleted. Nothing retries automatically.
4. **Who is the cause.** Every kill record carries how much of the machine's use was maitred's (the sum of the jobs' use in the sample). When most of the pressure came from unadmitted load, the kill still happens, because stopping a maitred job is the only room maitred can make, and `status` and the record say where the pressure came from.

If a holder itself is killed (by `pkill maitred`, or the system's out-of-memory killer), its supervisor sees the report pipe close, stops the session itself and records `holder lost`. A job that has lost both its holder and its supervisor is out of the kill policy's reach; `status` lists its session for a person.

## Supervision and cleanup

A supervisor is one process with one job: from enqueue to record, it is the only thing that changes its ticket, and only it and its holder signal its job.

### From enqueue to record

1. **Classify.** Find the project and its class file (Class files), split shell commands, match, run a checker if the rule names one. A `none` rule or a keep-awake wrapper refuses here, before any ticket exists, and so does a records directory that can't be written.
2. **Nesting check.** If the caller's session id, boot and pid namespace equal those of a living `running` ticket's session (from its fixed header), the caller is inside an admitted job. Its class must be no stricter than that ticket's: then it runs directly, inside the parent's reservation, with no ticket of its own, and leaves a `nested` note in the parent's request directory so the parent's record lists it. A stricter nested command is refused with a message naming both classes. A session can only be joined by its own members, so nothing outside the job passes the check. Nested runs are measured as part of the parent's session, so fanning out past the parent's reservation is overuse like any other.
3. **Prepare.** Write `job.json` in the project's records, create the request directory, then create the ticket under `admit.lock` and print its id first: `maitred: 0192f3a1c2e0-9f3c… queued (heavy, position 3)`. `submit` does exactly this, then re-executes itself as the detached supervisor, handing over the locked ticket's descriptor without close-on-exec, so the lock, which lives on the open file, carries over and the ticket never looks orphaned. The child calls `setsid`, points its stdin, stdout and stderr at `/dev/null` (its job's output goes to the log), and acknowledges over a pipe; only then does the parent print the id and exit. So `id=$(maitred submit …)` returns at once, and a timeout that signals the caller's process group doesn't reach the job.
4. **Wait** in the admission loop, reading its own requests on every pass.
5. **Spawn behind a gate.** Start `maitred __hold` with std `Command`, passing two pipes: the gate's read end as fd 3 and a report pipe's write end as fd 4. `pre_exec` places them with `dup2` and clears close-on-exec on each explicitly (a `dup2` onto the number a descriptor already has leaves the flag set), empties the signal mask and resets the signals maitred handles to their defaults, since a child inherits both; none of it blocks. On Linux the holder is started from `/proc/self/exe`, so it is the same binary even if maitred was upgraded meanwhile; on macOS, where that isn't possible, the holder checks a protocol version from the supervisor and refuses a mismatch with a message. The holder calls `setsid`, takes the pseudo-terminal as its controlling terminal in terminal mode, reports its pid, session and start time on fd 4, and blocks reading the gate. The supervisor writes the ticket as `running` with those, hands over the locked descriptor of any slot it took (as fd 5 and up), then writes to the gate. The job's environment carries each slot's number in the variable its pool names. Only then does the holder fork the job into a new process group of its session (made the terminal's foreground group in terminal mode) and exec the command. If the supervisor dies first, the holder reads end-of-file and exits without running anything.
6. **Connect the terminal** (below), and copy output into the log.
7. **Run its passes.** The supervisor writes `last_pass`, measures the job and writes `use`, applies its requests, and reads the holder's reports: the job stopped or continued, the kill policy chose it, the job's exit status, the session emptied. The holder runs the kill policy's passes (Kill policy).
8. **Finish.** When the job's main process exits, the holder reports its status and applies the rule's `leftovers` option to the rest of the session: `stop` sends TERM, then CONT, then KILL after the grace period; `keep` leaves them, and the holder stays as the session's leader while any of them lives, so the terminal isn't hung up under them, while a drain takes their output (below). The supervisor writes `record.json`, deletes the ticket and exits with the job's status, or dies by the signal that ended it.

If the report pipe closes while the job runs, the holder is gone: the supervisor stops the session itself, which it can, since the job is its own user's and started by it, and records `holder lost`.

**Measuring the job.** On every running pass, the supervisor finds its session's processes and sums them. On Linux, the session field of `/proc/<pid>/stat` finds them. On macOS, `sysctl` lists every process (`KERN_PROC_ALL`; there is no working selector by session), maitred keeps its own user's, and `getsid` each: about 0.3 ms for 800 processes, inside the sandbox. CPU is cumulative CPU time, including that of children already reaped, differenced over the pass, so short-lived build steps count; on macOS, `proc_pid_rusage` gives it in Mach time units, converted with `mach_timebase_info` (on Apple Silicon, reading them as nanoseconds undercounts by a factor of about 42). Memory is, on Linux, `Pss_Anon` plus `Pss_Shmem` from `/proc/<pid>/smaps_rollup`, not the whole proportional size, whose clean file pages `MemAvailable` already counts as free; a process that refuses it (not dumpable) is counted at its resident size and the figure is marked estimated. On macOS it is the physical footprint, which counts shared pages once and excludes clean file pages. The processes run as the supervisor's own user, which is what lets it read them without privileges. Processes that leave the session aren't measured; they show only in the sampler's totals, as unadmitted load.

### The job's terminal

| Caller | The job gets | So |
| --- | --- | --- |
| stdout is a terminal, and a pseudo-terminal can be opened | a new pseudo-terminal, the controlling terminal of the holder's session, with the job's group in the foreground. Each of the job's stdin, stdout and stderr is the pseudo-terminal where the caller's is a terminal, and the caller's own file where it isn't (`2>err.log`, `< in`), as sudo does. The caller's terminal goes into raw mode and maitred relays both ways, copying output to the log | colour, progress bars, password prompts, `/dev/tty`, window size and Ctrl-C/Ctrl-\ behave as without maitred; redirected streams stay redirected |
| anything else (an agent harness, CI, a pipe, or a sandbox that refuses pseudo-terminals) | pipes, relayed and logged; the caller's stdin passed through unchanged. When the caller's stdout and stderr are the same file (`2>&1`), the job gets one pipe for both, so their order is kept | what the job would have had without maitred: no terminal |

The window size is copied at start and on every SIGWINCH. Relaying stops when the job's main process has exited and the session is empty or handed to a drain. The caller's terminal settings are restored by a guard that runs on every exit path, including a panic and every signal maitred handles.

**Ctrl-Z in terminal mode.** The caller's terminal is raw, so ^Z travels through the pseudo-terminal and stops the job, not maitred. The holder sees the stop and reports it; the supervisor restores the caller's terminal and stops itself, so the caller's shell gets its prompt back. On `fg` the supervisor is continued, puts the terminal back into raw mode, copies the window size and continues the job. This is what sudo does.

**A supervisor killed with KILL in terminal mode takes its job with it.** Closing the pseudo-terminal's master hangs up the session, the same as closing a terminal window would without maitred; the holder treats the hangup as the end, stops the session and exits. Only a raw-mode terminal is left behind, as with any raw-mode program, and `reset` clears it. In pipe mode the job runs on under its holder.

**The drain.** When a rule keeps leftovers, they may still hold the job's output. Before exiting, the supervisor starts `maitred __drain` with that output, which keeps reading it into the log and exits when every writer has closed it. The drain closes the caller's stdin, stdout and stderr first: kept processes are logged, not relayed, so the caller sees end-of-file when the job ends, not when the server does. It holds no ticket and no lock, and never signals anything. Without it, a kept server would get SIGPIPE on its next write, or the supervisor would wait forever. In terminal mode it holds the pseudo-terminal's master, so if the drain is killed, the kept processes are hung up.

### Signals

| Event | While waiting | While running |
| --- | --- | --- |
| TERM or HUP to the supervisor | record `abandoned`, delete the ticket, die by the signal | TERM to every process of the session, then CONT, then KILL after `grace`; record `stopped by signal`; die by the signal |
| INT or QUIT to the supervisor | as TERM | in terminal mode, Ctrl-C and Ctrl-\ reach the job through the terminal, never through maitred. A signal sent to the supervisor itself is passed to the session once, with no KILL after it, so a job that handles INT keeps doing so; when the job has ended, maitred dies by the same signal, so a shell loop around it stops |
| TSTP (Ctrl-Z), CONT | stop or resume the supervisor; deferred while it holds `admit.lock` | pipe mode: pass to the session, then stop or resume the supervisor itself. Terminal mode: see Ctrl-Z above |
| HUP when maitred was started under `nohup` | ignored, as `nohup` asked | ignored |
| cancel request accepted | record `cancelled` with sender and reason, delete the ticket | TERM, CONT, KILL after `grace`; record `cancelled` |
| chosen by the kill policy | not applicable | the holder marks it, then TERM, CONT, KILL after `grace`; record `killed` with the threshold |
| supervisor killed with KILL | orphan, waiting | pipe mode: orphan, running; the job runs on, and its holder still carries out a kill. Terminal mode: the job is hung up (above) |
| holder lost | not applicable | the supervisor stops the session; record `holder lost` |

Signals are handled on one thread, which is how a Ctrl-Z that arrives while the supervisor holds `admit.lock` is deferred until the lock is released. SIGSTOP can't be deferred: a supervisor stopped that way while holding `admit.lock` stalls admission and the kill policy until it is continued (Risks). `grace` (default 10 s), `poll_interval` and `waited_ms` are measured on a clock that keeps counting through sleep, so a grace period that spans a sleep has expired on waking.

### Requests and scope

Only the ticket's own uid can write into its request directory. A same-uid request is honoured when the sender's project (that of its working directory) is the ticket's; a request for another project is honoured only with `--other-project`, which the CLI accepts only from an interactive terminal. Both checks are advisory: a sender can `cd` into the other project, and an agent that builds itself a pseudo-terminal passes `isatty`. They keep agents from doing it by habit, and the record names the sender's working directory, project, ticket and pid either way. Another user's ticket can only be cancelled as that user or as root. Cancelling an orphan fails (69): nobody is left to apply the request.

### Orphans

- **Seen** by any reader whose shared try-lock succeeds. Nothing is written; `status` shows the ticket's last payload and how long its lock has been free.
- **Charged** only when running, from this boot and namespace, with its session still alive: judged by the holder (its pid answers `kill(pid, 0)` with success or EPERM, another user's process a sandboxed reader can't inspect, and its start time still matches where it is readable), never by a process group, which a job's process can leave. A charged orphan that can't be checked further stays charged, and `status` flags it for its owner or root (`maitred clean --all`).
- **Adopted** with `maitred wait <id>`: same project and uid; the command in `job.json` must still match the project's class file; confirmed at an interactive terminal (the same `isatty` limit); refused while its session is alive. The table in Ticket state machine gives each outcome.
- **Not waited on blindly.** A `wait` that isn't at a terminal never adopts. On an orphan whose session is gone it exits at once (69) and says the job ended without a record, so it is safe to run again. On one whose session is alive it waits until the session is gone, then exits the same way and says the outcome is unknown; it never returns while the job still runs, because the obvious next step, running it again, would run it twice.
- **Expired** after `orphan_ttl` (default 24 h): the sweep writes an `expired` record into the project (its path is in the request directory) and deletes the ticket.

### The sweep

Each admission pass, after deciding, spends at most a few milliseconds removing what it may: expired orphans of its own uid and its own project (record first; another project's records may not be writable from this sandbox, so its own supervisors sweep it), tickets of the same scope whose job had already finished (record present), unknown tickets of its own uid with a free lock, `.new-*` files older than a minute whose lock is free, stop markers whose lock is free, and request directories older than a minute (by the time in their id) whose ticket no longer exists. Another user's debris is that user's to clean, or the administrator's (`maitred clean --all`, as root).

## Sampler

`maitred watch` runs as the maitred user, never root, takes one sample every `sample_interval` (default 2 s), replaces `load/now.json`, appends the history, and records the admissions it sees; it aims to cost under 1% of one core. It reads machine totals only: measuring a job is its supervisor's work, because an unprivileged sampler can't read other users' processes on macOS.

### What it reads

|  | Machine load | Memory | GPU | Sleep and power |
| --- | --- | --- | --- | --- |
| macOS | CPU ticks from the host's CPU load info, differenced; CPU pressure as runnable threads per core, from the scheduler's load figures (the 1-minute load average is the only one known to be readable without root, and it lags: to be verified) | the host's VM statistics and the kernel's memory-pressure level | device utilisation from the accelerator's IOKit performance statistics (unverified without root); memory is the shared pool | `pmset -g assertions` |
| Linux | `/proc/stat` differenced; CPU pressure from `/proc/pressure/cpu` (`some avg10`) | `/proc/meminfo` (`MemAvailable`); `/proc/pressure/memory` | NVIDIA through NVML; AMD through `gpu_busy_percent` and the VRAM counters in sysfs | logind inhibitor locks, where they exist |
| Container (cgroup v2) | `cpu.stat` differenced; `cpu.pressure` | working set: `memory.current` minus `inactive_file` from `memory.stat` | as Linux, for devices passed in | none |

**Containers.** The sampler decides it is in a container from its own cgroup (`/proc/self/cgroup` and the files under `/sys/fs/cgroup` it names), never from process 1's namespaces, which a service may not be allowed to read; it samples the cgroup it runs in, so it must run inside the container it rations. Capacity is `cpu.max` quota over period, or the size of `cpuset.cpus.effective`, or the host's CPU count when both say `max`; memory is `memory.max`, or the host's `MemAvailable` when it says `max`. A system service in its own cgroup on a bare host is told apart from a container by `memory.max` and `cpu.max` both saying `max`: then the host's figures are used. A cgroup v1 host is read from its v1 files where they exist, otherwise as the host with a warning that limits are unknown. A capacity under one core clamps every class minimum to the capacity.

**Per-job figures, frozen per sample.** The sampler can't measure jobs, but each sample carries every running ticket's use as its supervisor last wrote it, copied only when the measurement's window overlaps the sample's own interval (`jobs` in `load/now.json`). So `decide` and `choose_victim` take a job's figure and the machine's from the same moment, and every reader of one sample sees the same figures. The load file names no command and no process.

**Sleep.** When the sampler sees the machine slept between two samples, it records the time it woke (`woke_at`), resets its streaks, and marks the first sample after as not fresh, so nothing is admitted or killed on the burst every machine makes on waking.

**Critical readings.** CPU use and available memory are critical. If either can't be read, the sample says so, and admission falls back to one job at a time, exactly as when the sampler is down: never a permanent wait. CPU pressure, GPU and power assertions are not: a missing CPU pressure figure means no CPU limit or CPU kill, and a missing GPU reading the same for GPU, and `status` says so.

### `load/now.json`

```
{
  "v": "4.0",
  "sampled_at": "2026-10-07T21:14:02.118Z",
  "interval_ms": 2000,
  "fresh": true,
  "woke_at": "2026-10-07T08:02:11Z",
  "capacity": { "cpu_millicores": 8000, "mem_mb": 16384 },
  "cpu_used_millicores": 4960,
  "cpu_pressure": 0.42,
  "mem_available_mb": 5120,
  "gpu": { "utilization": 0.12, "mem_used_mb": 0 },
  "quiet_streak": 0,
  "over_cpu_streak": 0,
  "over_gpu_streak": 0,
  "pressure": { "cpu_some_avg10": 4.2, "mem_some_avg10": 0.0, "level": "normal" },
  "jobs": { "0192f3a1c2e0-9f3c2a7e5b1d4c88": { "cpu_millicores": 1850, "mem_mb": 760 } },
  "seq_high": 4183,
  "sleep_assertions": { "count": 2, "unexpected": 1 },
  "missing": []
}
```

- `quiet_streak`, `over_cpu_streak` and `over_gpu_streak` count consecutive samples under the quiet threshold and over the CPU-pressure and GPU kill ceilings, so `decide` and `choose_victim` need only this one file. All three restart after a sleep.
- `seq_high` is the largest `seq` the sampler has seen in any ticket, so a new ticket's `seq` never goes backwards when tickets are deleted.
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

A class file is read from the working tree, parsed on every run (it is small; there is no cache to poison), and matched against every simple command in what the worker runs; anything maitred can't vouch for is heavy, and every edit is recorded.

### Reading the class file

1. **The project** is found by walking up from the directory each command runs in to a git repository, identified by its common directory, so all worktrees are one project. The class file is `.maitred/classes` at the root of that working tree. A worker chooses its working directory, so it chooses its project; records and `status` show each project's path.
2. **Edited or not.** maitred runs `git` (the binary found at install, with every `GIT_*` variable removed) to get the committed copy's blob id (`HEAD:.maitred/classes`) and the working file's (`git hash-object`), and the same for each checker a matching rule names. Every `job.json` and record carries both, and `edited-classes` or `edited-checker` when they differ. This is for seeing, not stopping: nothing rests on git's answer, so it isn't hardened further (design, Class file).
3. **No class file** makes every command heavy, with a warning and the flag `no-class-file`.
4. **Trying a change** is `classes match --file <draft> -- <cmd>`, which shows how commands would be classified under the draft. No run ever uses a draft.
5. **An invalid file** (a rule without a why, a pattern that doesn't compile, an unknown option or pool) makes every command heavy, with a warning naming the bad line. Nothing is refused for it.

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
| `slot` | the name of a pool the machine config declares | none | can be added, never removed | takes one slot from that pool; the job's environment carries its number in the variable the pool names |
| `leftovers` | `stop`, `keep` | `stop` | `stop` | what happens to processes still in the job's session when its main process exits: stopped (TERM, CONT, then KILL after the grace period), or left running. Left running, they are no longer the ticket's, and the sampler counts them as unadmitted load, so the budget and the live limits still see them |

```
# name      | pattern                         | class   | why                                        | options
e2e         | ^bin/e2e( |$)                   | heavy   | boots the app server and drives a browser  | leftovers=keep mem=3G slot=test_env
specs       | ^bin/rspec( |$)                 | light   | its cache database and ports come from a slot | slot=test_env
dataset     | ^bin/process-dataset( |$)       | heavy   | cost depends on the dataset                | checker=bin/dataset-cost
```

The command line can override an option only in its stricter direction, as with `--class`: `--leftovers stop` on a `keep` rule, a higher `--cpu`, an extra `--slot`, never the reverse. Anything else the rule sets stays as written.

### Matching

1. **Split shell commands.** When the command is a shell running a string (`sh -c`, `bash -c`, `zsh -c`), the string is lexed and split on `&&`, `||`, `;`, `|`, `&` and newlines into simple commands. Each is matched in the directory it will run in: `cd`, `pushd`, `env -C`, `git -C` and `make -C` change it, and a `cd` to a path the lexer can't resolve (a variable) makes the rest of the string unparseable. The strictest class among them wins. When several match, their reservations combine: the largest for commands run one after another, the sum for a pipeline or `&`, N times for `xargs -P N`; every slot any of them asks for; the strictest `leftovers`; and each named checker runs.
2. **Unparseable parts** (command substitution, subshells, functions, `eval`, heredoc bodies, an unknown launcher) are still searched: every sequence of words in them that looks like a command is matched. If any rule other than `none` matches, the job is at least that rule's class, so what the lexer can't follow fails closed.
3. **Strip wrappers** from each simple command, repeatedly: leading `NAME=value` assignments, then wrappers recognised by name wherever they live (`/usr/bin/time` is `time`), each with its own option grammar for both the GNU and BSD versions: `env` (including `-i`, `-u`, `-C`, `-S`), `exec` (including `-a`), `command` (but not `command -v` or `-V`, which look a command up rather than run it), `nohup`, `time` (including `-p`), `nice` (including `-n N` and `-N`), `timeout` and `gtimeout` (including `-k` and `--signal`), `xargs` (including `-0`, `-n`, `-P`, `-I`, `-J`, `-a`, `-d`), `setsid`, `stdbuf`, `ionice`, `taskpolicy`, `arch`, and whatever the machine config adds.
4. **Normalise the program.** A path into the project (`./bin/x`, `bin/x`, or its absolute path) is rewritten relative to the project root, so all three match `^bin/x`.
5. **Strip launchers** the machine config lists (`bundle exec`, `bin/bundle exec`, `npx`, `pnpm exec`, `yarn`, `uv run`, `poetry run`, `ruby -S`, and the like), after normalising, so a launcher run from `bin/` is recognised; then normalise again.
6. **Join** its argv with single spaces, quoting any argument that contains whitespace or quotes.
7. **Match.** The first rule whose pattern matches wins; no match means heavy.
8. **Keep-awake wrappers** (`caffeinate`, `systemd-inhibit`, and whatever the machine config adds), recognised by name as the program of any simple command after stripping, refuse the job; the word as an argument (`rg caffeinate`) does not. This catches the honest case; a script that holds the machine awake internally is caught only afterwards, by the sampler's count of unexpected sleep assertions.

### Checkers

A checker runs before the ticket exists, as the worker, in the project root, with the job's argv as its arguments, a clean environment and a 5 s timeout. It prints one line: `<class> [cpu=<cores>] [mem=<size>]`. Anything else, a non-zero exit or a timeout gives the rule's own class and reservation, and a warning. Its output can raise a reservation, never lower it below the class minimum. Its hash, and whether it differs from the committed copy, go in the record.

### `classes check` and hooks

`classes check [path]` validates the file: every rule parses, every pattern compiles, every rule has a why, every pool is declared on this machine, checkers exist and are executable. It also lints: a rule that an earlier one always shadows, and a pattern without a leading `^`. It exits non-zero on errors, for CI.

`classes match [--shell] [--file <draft>] -- <cmd>` prints, as JSON, the class each simple command would get and whether it matched a rule or fell through to the unlisted default. A hook calls it with `--shell` on the raw command string. It wraps the whole string in maitred when any part matched a rule that isn't `none`, including inside parts the lexer couldn't parse. It refuses (77) a string that runs a `none` command alongside a queued one, and one that backgrounds a matched command with `&`. It never touches a string whose program is maitred itself, so `maitred status` is always reachable. It leaves strings with no match alone, since wrapping every unknown command would make `ls` queue. When it wraps, it runs the string under the shell the harness would have used (`maitred -- "$SHELL" -c '<string>'`), so the harness's shell syntax and word splitting still apply.

## Command line

One binary, `maitred`, with subcommands; every command that reports state has `--json`, and anything maitred decides on its own is said on stderr with the `maitred:` prefix.

| Command | Flags | Exits with |
| --- | --- | --- |
| `maitred [run] -- <cmd>` | `--label`, `--class` (stricter only), `--leftovers stop`, `--cpu` and `--mem` (higher only), `--slot <pool>`, `--timeout <dur>`, `--log-cap <size>`, `--no-log`, `--quiet` | the job's status, or death by the signal that ended it; or a maitred status (below). Once a ticket exists, the first line on stderr is its id, even with `--quiet`; a refusal before that names no id. With no machine root installed, runs the command directly after one warning |
| `maitred submit -- <cmd>` | as `run` | 0 once the detached supervisor has taken over the ticket; prints the id |
| `maitred wait <id>` / `--label <l>` | `--timeout <dur>` | the job's status from its record; `lost` if there is neither ticket nor record. On an orphan: at an interactive terminal, offers adoption; otherwise exits 69 once its session is gone, saying whether it ended before `wait` was called |
| `maitred status` | `--project`, `--json`, `--watch` | 0 |
| `maitred result <id>` | `--json` | 0 if the record exists |
| `maitred cancel <id>` | `--reason <text>` (required), `--other-project` (interactive terminal only) | 0 once the request is written; 69 on an orphan |
| `maitred update <id>` | `--label`, `--class` (stricter only) | 0 once the request is written |
| `maitred classes check [path]` / `match [--shell] [--file <draft>] -- <cmd>` / `propose` | `--json` | non-zero on an invalid file |
| `maitred export` | `--machine` or `--project`, `--since`, `--format otlp-json` | 0 |
| `maitred watch` | `--once`, `--observe-only` | runs until signalled; refuses to run as any user but maitred's, and as root |
| `maitred clean` | `--dry-run`; `--all` (root only) | 0 |
| `maitred install machine` | `--dry-run`, `--service` | 0 once the maitred user, root, slot files, config and (with `--service`) the sampler's launchd or systemd unit exist; needs an administrator |
| `maitred install claude-code` | `--user`, `--check` | 0 once the skill, the hook, the sandbox allowlist (tickets and requests only) and the write-deny entries for the class file and checkers are written, after confirmation |

### Exit statuses maitred chooses

These use the BSD `sysexits` values where one fits. None is unique, because a job may exit with any number; `maitred result` is authoritative, and stderr names the cause. The skill tells agents to read the `maitred:` line, not the number: 75 in particular means a timed-out wait here, and something else in older lock scripts.

| Status | When |
| --- | --- |
| 64 | usage error |
| 69 | the machine root is on a network filesystem or not owned as installed; `wait` found the run `lost`; a non-interactive `wait` found an orphan; a cancel named an orphan |
| 73 | the project's records directory can't be written |
| 75 | `--timeout` passed before admission |
| 77 | refused by a rule: `none`, a keep-awake wrapper, a stricter nested command, scope, a backgrounded command in a hooked string |
| 78 | no project: outside git, with no `.maitred/` above the working directory |
| 128 + n | maitred stopped the job with signal n (the kill policy, a cancel). When maitred itself was signalled, or the job died by a signal, maitred dies by that signal instead, so the caller sees what a shell would |

### Messages

One line each, on stderr, only when something changes: `maitred: queued (heavy, position 3)`, `maitred: waiting: settle (8 s)`, `maitred: admitted after 4m10s`, `maitred: killed at the memory threshold (free memory 410 MB < 600 MB)`. `--quiet` drops all but the ticket id and errors.

## Packages

maitred leans on the standard library and two thin system crates, and writes its own code where no crate does the job; every choice below was checked against the crate's source on 2026-10-07, and anything not checked says so.

| Need | Choice | Why | Rejected |
| --- | --- | --- | --- |
| flock, try-lock | std `File::lock` / `try_lock` (stable since Rust 1.89) | calls `flock(2)` on Linux and macOS; no dependency. std says the implementation "may change", so a test pins it (two handles in one process: works with flock, fails with fcntl locks) | rustix `fcntl_lock` (record locks, which drop when any descriptor closes); fd-lock, fs4, nix (all fine, all redundant) |
| Filesystem check | rustix `fstatfs` | `f_type` on Linux, `f_fstypename` on macOS | nix (splits the two across cfgs) |
| Starting helpers | std `Command` re-executing maitred (`__hold`, `__drain`, the detached supervisor of `submit`), from `/proc/self/exe` on Linux; `pre_exec` only places descriptors with `dup2`, clears close-on-exec on each, empties the signal mask and resets handled signals | `Command::spawn` returns only after the child has exec'd, so nothing may block in `pre_exec`; a bare `fork` of a threaded process can deadlock | blocking on the gate in `pre_exec` (deadlocks); process-wrap (no wait with timeout, pulls in nix), command-group (unmaintained since 2023) |
| Sessions, signals, pseudo-terminals | rustix `setsid`, `getsid`, `kill_process`, and its `pty` and `termios` modules (`openpty`, `TIOCSCTTY`, `tcsetpgrp`, `tcsetattr`); a raw `fork` only in the single-threaded holder | everything needed, no extra runtime (the pty functions not yet checked against the source) | nix |
| Forwarding signals | signal-hook, on one signal thread | the usual crate for handling TERM/INT/HUP safely (unverified: not opened); one thread is what lets a Ctrl-Z be deferred while `admit.lock` is held | a hand-written handler |
| Clocks | libc `clock_gettime` behind a small per-OS module; `mach_timebase_info` on macOS | rustix can't name `CLOCK_UPTIME_RAW`; std `Instant` stops during sleep on both platforms; `proc_pid_rusage` reports CPU time in Mach units, about 42 times nanoseconds on Apple Silicon | `Instant` |
| Measuring a job, Linux | procfs-core for `/proc/<pid>/stat` (session, CPU, reaped children's CPU) and `smaps_rollup` (`Pss_Anon`, `Pss_Shmem`) | the session field is what finds the job's processes; anonymous and shared proportional size is what `MemAvailable` doesn't already count | sysinfo (its `group_id()` is the user's gid, not the process group; no proportional size) |
| Measuring a job, macOS | libc `sysctl` (`KERN_PROC_ALL`, kept to the supervisor's own user, then `getsid` each: `KERN_PROC_SESSION` is declared but not implemented) and `proc_pid_rusage` (CPU time including reaped children, physical footprint) | same-user processes, readable inside the sandbox; about 0.3 ms per pass for 800 processes | sysinfo, for the same reasons |
| Machine sampling | Linux: `/proc/stat`, `/proc/meminfo`, `/proc/pressure`, cgroup v2 files read directly. macOS: libc `host_statistics64`, `sysctlbyname` | totals only, no process access | sysinfo |
| GPU | nvml-wrapper (NVIDIA; loads the driver's library at run time, so no build dependency); sysfs for AMD; IOKit's accelerator statistics on macOS through objc2-io-kit | best effort, all unverified without root | powermetrics (needs root) |
| Boot id, macOS | libc `sysctl` `KERN_PROC_PID` for process 1's start time | allowed in a sandbox, unlike `proc_pidinfo` on process 1 or `kern.boottime` | `kern.bootsessionuuid` (refused in a sandbox) |
| Power assertions, macOS | parse `pmset -g assertions`, every sample | no FFI; the output format isn't documented as stable, so the parser fails loudly | objc2-io-kit (raw CoreFoundation dictionaries; kept as the fallback) |
| Git | run `git` | three small reads: the common directory, and the committed and working blob ids of the class file and checkers; no build cost | gix (128 crates), git2 (needs a C compiler) |
| OTLP-JSON | about ten hand-written serde types | exact conformance (64-bit integers as strings) and 35 fewer crates; opentelemetry-proto stays as a test-only dependency to check our output against | opentelemetry-proto at run time (writes `asInt` as a number, links the SDK crates) |
| Command line | clap (derive) | standard; 19 crates |  |
| Files | serde, serde_json, crc32fast | the slot checksum and every JSON file |  |
| Ids and hashes | getrandom for ids; sha2 for the project id | 64 random bits per id; a stable hash of the project's path | rand |
| Class rules | regex, shell-words | linear-time patterns; POSIX argv quoting for matching |  |
| Logging | tracing, tracing-subscriber | debug output behind a flag |  |
| Tests | assert_cmd, proptest, rexpect (Unix-only, fine here); opentelemetry-proto as a dev-dependency | the cross-process lock test, property tests of `decide` and `choose_victim`, terminal prompts |  |

### What we write ourselves

- **Measuring a session** on both platforms: no crate groups processes by session.
- **The network-filesystem table.** Only NFS's magic number is exported by any crate; SMB, FUSE, 9p, virtiofs and Docker Desktop's file sharing need their own table, and tests on real mounts.
- **The waiting loop with a timeout and grace periods**, on our own clocks.
- **The holder**: `setsid`, the gate, the report pipe, the fork into a foreground group, reaping, `leftovers`, slot descriptors, and the kill policy's passes.
- **The shell lexer and the wrappers' option grammars**, for GNU and BSD.
- **The OTLP-JSON types.**

### What the reviews and the survey changed in this document

- The boot id is read by each supervisor itself, from sources a sandbox allows (Linux `boot_id`; the start time of process 1 through `sysctl` on macOS), not from the sampler.
- Whether an orphan's session still exists is judged by its holder's pid and start time, never by a process group, which a job's process can leave; EPERM counts as alive.
- Jobs are measured by their supervisors, because the sampler, as an unprivileged user, can't read other users' processes on macOS; macOS has no working selector for a session's processes, so they are found by a scan.
- The gate moved out of `pre_exec` into the holder, because `Command::spawn` waits for exec.
- The holder carries out kills, because it is never stopped by job control and outlives its supervisor.

## Testing

The pure core is tested exhaustively and by property; everything that touches the operating system is tested with real processes, on both platforms, because that is where this kind of tool breaks.

| Layer | How | What it proves |
| --- | --- | --- |
| `decide` | unit tests per rule; property tests over random snapshots | nothing starts past the live limits or beside a running quiet job; admitted reservations never exceed what is free except by first-in; class rules are never waived; an unmeasured job holds its whole reservation; no two admissions take one slot; the answer depends only on the snapshot |
| `choose_victim` | unit tests per trigger; property tests | one victim per snapshot, and the same one from every holder for one sample; never a job whose holder is gone; nobody chosen while a live stop marker exists or a ticket was just recreated |
| Starvation and the herd | a deterministic simulator: synthetic tickets and load traces, many waiters, simulated time, sleeps, kills and deleted tickets | every class is admitted eventually (aging, backfill, `quiet_max_wait`, the lapse of a hold); a burst never exceeds the minimums; settle spaces heavy starts; order follows `(seq, id)`, not poll timing; a breach stops one job at a time |
| Tuning | the simulator replaying observe-only samples, from every machine Nick collects | the machine config's starting numbers |
| The ticket file | a writer killed at every byte offset of a slot write; readers in a tight loop; on macOS and Linux | a reader always finds a complete generation, never a torn or older one |
| Locks and liveness | many real `maitred` processes on a temporary root, killed with KILL at random points; lock files, whole tickets and request directories deleted underneath; a stopped supervisor; Ctrl-Z while holding `admit.lock` | a dead owner always reads as an orphan, a live one never; no two owners of one lock; a deleted ticket comes back with its `seq` and nobody is admitted meanwhile; a stopped supervisor neither holds budget nor freezes the queue |
| Slot pools | many waiters on a small pool; holders and supervisors killed at random; orphans whose session lives on | two live jobs never hold one number; a slot is free again as soon as its job's last process ends |
| Measurement | real jobs of known size (a process that allocates N MB and spins K cores, with children, short-lived children, shared pages, mapped files and a forked server), on Intel and Apple Silicon | each supervisor's `use` is within a stated margin of the truth on both platforms; shared pages count once; clean file pages don't count |
| Supervision | real children: every signal at every stage, a job that ignores TERM, a stopped job that handles TERM, a REPL that handles INT, a kept server, a child that calls `setsid` and one that calls `setpgid`, a nested `maitred` and a nested fan-out, `submit` captured with `$(…)`, the holder killed, a binary upgraded mid-run | forwarding, the gate, the holder and its reports, the grace KILL, the drain, nesting by session and class, signals reaching the whole session |
| Terminal | a real pseudo-terminal driven by a test harness: colour detection, a password prompt, Ctrl-C in a shell loop, Ctrl-Z and fg, resize, redirected stderr, a kept server after the job exits, maitred killed mid-run | the job behaves as it would without maitred; the caller's shell gets control back on Ctrl-Z; the caller's terminal is restored |
| Class files | fixtures; an edited class file and checker; shell strings with operators, `cd` into another project, `./bin`, absolute paths, wrappers with options (`nice -5`, `timeout -k 5 600`, `xargs -0 -P 4`), launchers, unparseable strings, backgrounded commands | each case is matched, wrapped, refused or flagged as intended |
| Platforms | GitHub Actions on Linux (real VMs, so flock, cgroups and signals behave as on a Linux box) and macOS; Linux containers with `--memory`/`--cpus` limits, including one shared by several host sessions enqueuing with `docker compose exec`; a local Linux VM for exploring by hand; sleep detection and GPU readings on real hardware | the sampler backends, the filesystem check, the cgroup edge cases, the shared-container case |
| Inside an agent sandbox | in CI, a sandbox profile close to the agent harness's; periodically, a real headless Claude Code session running maitred end to end | nothing needs what a sandbox denies; pipes are used when pseudo-terminals are refused; a supervisor can measure its own job |
| Agent conformance | scenarios that tempt a shortcut (run the heavy command directly, chain it after a light one, edit the class file, tidy `tickets/`, cancel another project's job, kill the supervisor), each run repeatedly against a headless agent with the skill installed; and Nick building gravedigger with maitred | how often agents cooperate, as a number that has to stay high, and that every shortcut shows up in `status` or a record |
| Regressions from review | every confirmed finding of the adversarial reviews becomes a test | none of them comes back |

## Risks and open questions

Three adversarial reviews (2026-10-07) found about 30, 25 and 47 confirmed problems; this revision answers them. What remains open, ranked by how much rests on it:

1. **maitred coordinates; it doesn't contain.** Load from outside maitred (a command the hook missed, work handed to a process outside the sandbox, a job's escaped daemon) is invisible to admission until it shows in the samples, and the kill policy can only stop maitred's own jobs, so under that load it stops jobs that cooperated. `status` and the records name it. Containment belongs to the operating system around the agent (design, Containment).
2. **Same-user processes can rewrite each other's maitred files.** Tickets, requests, stop markers and the `use` figures are writable by every process of the same user, so a deliberately hostile agent can delete or hand-write tickets, or understate its job's use. The sampler's files, the slot files, the live limits and the kill thresholds are out of reach.
3. **Settle, the minimums, the pressure ceilings and the kill thresholds are tuned numbers.** The simulator and observe-only data from several machines are the evidence planned.
4. **CPU pressure on macOS.** No fast figure is known to be readable without root; the 1-minute load average lags by design. Without a good one, macOS gets the CPU limits late, or not at all.
5. **Self-measurement inside a sandbox** is checked on macOS under this harness's profile only. If a sandbox refuses it, the job holds its whole reservation, which is safe but wastes the budget.
6. **GPU readings without root** are unverified on every platform, Apple Silicon especially. Without them there is no GPU limit.
7. **Processes that leave the session** (daemons that call `setsid`) escape measurement, signalling and cleanup; containing them needs cgroups on Linux and has no macOS equivalent.
8. **Anyone holding `admit.lock` stalls admission and the kill policy,** since victims are chosen under it. Any user can lock it (it must be readable to be locked), and a supervisor or holder stopped with SIGSTOP while holding it does the same. `status` reports the wait, but nothing can name the holder. Inherent to flock.
9. **A first-in job bigger than free memory** is admitted and then killed at the memory threshold; the skill tells agents not to retry such a kill blindly.
10. **Two maitred versions on one machine** run different `decide` and `choose_victim` over the same files; the machine config's minimum version is the guard.
11. **A running job's pid is in its ticket,** so another user can read its command line from `/proc/<pid>/cmdline` on Linux, as they could with `ps`. The ticket adds no new exposure, but doesn't reduce it either.
12. **Pseudo-terminals inside sandboxes.** Where they are refused, a job run by a person through a sandboxed shell gets pipes: no colour, no prompts. Agents are unaffected.
13. **Power loss mid-write** on macOS can leave a ticket with no valid slot; it reads as unknown and unlocked, which the sweep removes.
14. **Polling cost** with many waiters; waking on the load file changing is open.
15. **Checker latency**: up to 5 s on every enqueue in a project with checkers.
16. **The hook fails closed,** so a string it half-recognises is queued even when it needn't be. How often that happens in agent-written commands is to be measured during observe-only.
17. **Named resources beyond slot pools** (database connections, requests per host): the reservation map is ready; admission isn't.
18. **One queue per container.** A host and its containers never share a queue until file locks are shown to hold across that boundary, so a shared test container is rationed from inside, and the host's own load is outside its view.
