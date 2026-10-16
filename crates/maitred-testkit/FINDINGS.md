# Findings from the design reviews

Every finding of maitred's adversarial design reviews, rounds 1 to 4, with what the test kit does about it
(ARCHITECTURE.md, Test kit: Regressions). One line per finding:

```
<report> <id> | <what goes wrong> | <disposition>
```

The review reports themselves aren't in this repository; the id is only a label, and each line states its failure
in its own words. Round 1's findings appear under round 2's ids: round 2 rechecked each one by number (its `A` and
`AF` items), and every numbered round-1 finding is among them. Dispositions:

- `scenario <name> <sim|real|both> phase <n>`: the failure can still be observed under the current design, so a
  scenario checks it. Several scenarios are named with commas between them, and `phase1_classification` is the
  list of phase 1's classification cases. `sim` runs on the simulator, `real` needs real processes, `both` runs on both; the phase is the
  Build order phase whose code it covers.
- `folded into <report> <id>`: the same observable failure as that line, whose scenario covers both.
- `impossible by construction: <why>`: the current design leaves no route to the failure.
- `not a defect: <why>`: the report refuted it, or the design states the behaviour as intended.
- `out of v1: <where>`: the feature it concerns isn't in v1.

Sorted 2026-10-15 against ARCHITECTURE.md rev 88, with round 1's text checked against rev 96. A scenario's name may
change when it is written; this file is updated with it.

## Counts

| Report | Findings | scenario | folded into | impossible by construction | not a defect | out of v1 |
| --- | --- | --- | --- | --- | --- | --- |
| r2-admission | 21 | 14 | 4 | 1 | 2 | 0 |
| r2-concurrency | 27 | 16 | 4 | 3 | 4 | 0 |
| r2-platforms | 27 | 13 | 10 | 0 | 3 | 1 |
| r2-threat | 24 | 14 | 4 | 1 | 5 | 0 |
| r3-admission | 21 | 18 | 0 | 1 | 2 | 0 |
| r3-processes | 19 | 15 | 2 | 1 | 1 | 0 |
| r3-threat | 26 | 16 | 0 | 4 | 6 | 0 |
| r4-classify | 24 | 20 | 0 | 1 | 3 | 0 |
| r4-processes | 18 | 16 | 0 | 0 | 0 | 2 |
| r4-state | 21 | 20 | 0 | 1 | 0 | 0 |
| **all** | **228** | **162** | **24** | **13** | **26** | **3** |

## r2-admission

r2-admission A1 | a job using more memory than it reserved hands the excess back to the budget, so a job is admitted into memory that isn't free | scenario excess_use_isnt_given_back sim phase 1
r2-admission A2 | a load reading the sampler can't take makes every job wait for ever, while a stale load file would still admit one job at a time | folded into r2-platforms A5
r2-admission A3 | a job that needs a quiet machine is started on a busy machine because it is first in line with nothing else running, or, waiting at the head of the queue, keeps every other job out for ever | scenario quiet_job_waits_for_a_quiet_machine_without_holding_up_others sim phase 1
r2-admission A4 | a large job that has waited long but is kept out by the limit on running large jobs makes every small job behind it wait for hours, as if the queue were a single lock | folded into r2-admission B6
r2-admission A5 | a light job that asks for more than the class minimum never starts while smaller light jobs keep arriving and taking the room | scenario large_light_job_gets_in_under_steady_small_arrivals sim phase 1
r2-admission A6 | two large jobs start together past the limit on running large jobs, and which job starts first depends on which waiter happens to check first, not on queue order | scenario heavy_limit_counts_admissions_on_paper sim phase 1
r2-admission A7 | a job left running after its supervisor was killed keeps its reservation for up to a day, and no other job can start meanwhile | folded into r2-admission B8
r2-admission A8 | a job that needs a quiet machine, having waited long, holds back a light job queued ahead of it | impossible by construction: Inside `decide` walks the waiting tickets in queue order, a due quiet ticket only stops the walk at its own place, and the light class is always allowed, so no job queued earlier is held back by it
r2-admission A9 | a stopped or stuck waiting supervisor at the head of the queue takes budget and blocks every job behind it, and a stopped running supervisor can't be told apart from a working one | scenario stopped_supervisor_waiting_or_running_is_seen_as_stalled real phase 6
r2-admission A10 | on a breach the job chosen to be killed is one that no process will act on, or a second job is killed on a reading taken before the first was gone | folded into r2-admission B4
r2-admission A11 | the design and the architecture disagree on what a checker prints and on whether each class has its own limit on running jobs | not a defect: DESIGN's Class file and Admission sections now give the same checker output (`<class> [cpu=] [mem=]`) and per-class `max_running` as ARCHITECTURE's Checkers and Inside `decide`
r2-admission B1 | jobs whose use can't be measured are treated as using all they reserved, so they hold nothing back and a burst is admitted far past the machine's cores | scenario unmeasured_job_holds_its_whole_reservation sim phase 1
r2-admission B2 | memory shared between a job's processes is counted once per process, so the job looks bigger than it is and too many jobs are admitted beside it | scenario shared_pages_counted_once_in_job_use real phase 3
r2-admission B3 | a ticket nobody can read, including one written by a newer version, keeps a reservation for ever after its owner is gone, so the machine's budget shrinks permanently | scenario unreadable_ticket_charged_only_while_its_owner_lives both phase 5
r2-admission B4 | one breach of a kill threshold kills two or more jobs, or kills jobs one after another while the load reading is down | scenario one_breach_kills_one_job both phase 8
r2-admission B5 | a running job whose supervisor was stopped with Ctrl-Z is chosen to be killed and then nothing is killed | scenario kill_reaches_job_whose_supervisor_is_stopped real phase 8
r2-admission B6 | a long-waiting job that its class rules keep out still holds budget for hours, keeping smaller jobs out for nothing | scenario aged_ticket_kept_out_by_its_class_holds_nothing,class_blocked_ticket_doesnt_drain_when_its_hold_lapses sim phase 1
r2-admission B7 | a memory or CPU breach kills the wrong job or none: an honest heavy job before a light one using more than it reserved, or nothing when most of the load comes from outside maitred | scenario memory_breach_kills_the_job_furthest_over_and_only_one sim phase 1
r2-admission B8 | a long-running orphaned job, quiet ones especially, keeps the whole queue from moving while `status` doesn't say why | scenario orphaned_quiet_job_doesnt_stop_the_queue sim phase 1
r2-admission B9 | when many waiters contend for the queue's lock, live waiters look stalled and lose their place | scenario waiter_slow_to_get_queue_lock_not_stalled real phase 6
r2-admission B10 | the design's account of admission (budget, class rules, first-in, the light rule, the kill policy, checker output, limits on running jobs) disagrees with the architecture | not a defect: DESIGN's Admission section and its Kill policy paragraph under Supervision now state the same rules as ARCHITECTURE's Admission

## r2-concurrency

r2-concurrency A1 | the load reading goes stale for good because a sampler started by a second user can't replace the first user's load file, so every waiter falls back to one job at a time | impossible by construction: Where things live, and who owns them makes `load/` the maitred user's and written by the sampler only, and Command line says `watch` refuses to run as any other user, so only one user ever writes the load file
r2-concurrency A2 | a person can't cancel another user's job, and nothing said who may create entries in the shared requests directory | not a defect: ARCHITECTURE's Requests and scope says another user's ticket is cancelled as that user or as root, and Where things live, and who owns them now gives `requests/` its owner and mode (`1777`, created at install)
r2-concurrency A3 | after a lock file is deleted and recreated, two processes each hold the same lock, so two waiters admit at once, or an adopter supervises a ticket no one else can see | scenario deleted_lock_file_never_gives_two_owners real phase 2
r2-concurrency A4 | an orphan whose supervisor died between admission and start can be neither adopted nor removed, and a job started just before such a death runs behind a ticket that names no session, uncharged and out of reach | scenario supervisor_killed_between_admission_and_start_leaves_no_stuck_or_uncharged_job real phase 5
r2-concurrency A5 | `wait` on a job never returns because neither its ticket nor its record exists, or a finished job's true record is overwritten and the job is run again | scenario wait_always_ends_and_finished_record_never_overwritten real phase 5
r2-concurrency A6 | after a reboot a ticket takes the previous boot's id, so its running job looks left over from before the reboot, isn't charged, and may be swept while it runs | impossible by construction: The ticket's payload (`boot`) has each supervisor read the boot id itself from the kernel (Linux `boot_id`, the start time of process 1 on macOS), never from the sampler's file, which survives a reboot
r2-concurrency A7 | a dead job's ticket looks alive while another reader is checking it, so its reservation keeps other jobs out or it blocks the queue | scenario liveness_reader_never_makes_dead_ticket_look_alive real phase 2
r2-concurrency A8 | a job left running after its supervisor died loses its reservation while it sits idle, and new jobs fill its room before it picks up again | folded into r2-platforms B9
r2-concurrency A9 | while `submit` hands its ticket to the detached supervisor, the ticket can look abandoned or be lost | scenario submit_handover_never_leaves_ticket_orphaned real phase 5
r2-concurrency A10 | a supervisor stopped with Ctrl-Z while holding the queue's lock freezes admission for every user until it is resumed | scenario ctrl_z_while_holding_queue_lock_is_deferred real phase 6
r2-concurrency A11 | a torn read makes a ticket go back a generation or read as unknown, and a ticket whose details exceed the room in its fixed-size slot can't be written whole | scenario ticket_never_read_older_or_partial_at_any_payload_size real phase 2
r2-concurrency A12 | a ticket written by a newer maitred is charged less than its reservation or given no defined place in the queue, and two tickets can get the same queue number | folded into r3-admission A18
r2-concurrency A13 | a job's record written in one worktree can't be found from another worktree of the same project | scenario wait_finds_record_in_another_worktree real phase 5
r2-concurrency AF4 | a job's child that left the job's session runs maitred, waits in the queue behind its own parent, and the two wait on each other | scenario setsid_child_queues_and_parent_wait_ends_by_timeout real phase 5
r2-concurrency AF8 | a system cleaner deletes long-untouched lock files, so two processes each hold the admission lock at once | folded into r2-concurrency A3
r2-concurrency AF10 | a dead orphan stays charged, or a forced kill hits an unrelated process group, because a process or group number was reused | scenario reused_process_number_never_charged_or_signalled real phase 5
r2-concurrency AF7 | any user can hold the queue's lock and stop admission for everyone | not a defect: ARCHITECTURE's Risks and open questions (item 8) accepts it as inherent to flock
r2-concurrency B1 | starting a job hangs for ever, because the step that waits for the go-ahead blocks the call that starts the helper process | scenario job_start_does_not_wait_on_go_ahead real phase 5
r2-concurrency B2 | two first runs in one project disagree on the project's identity or corrupt a per-user index, and the index sits outside what the agent sandbox may write | impossible by construction: Where things live, and who owns them says there is no per-user state outside the machine root and the projects, and The ticket's payload (`project`) makes the project id a hash computed from the repository's path, never stored
r2-concurrency B3 | a ticket nobody can read keeps a reservation for ever, and an upgrade that writes tickets an older binary can't read stalls the queue | folded into r2-admission B3
r2-concurrency B4 | on macOS an orphaned job of another user, whose start time can't be read, keeps its reservation for ever | not a defect: ARCHITECTURE's Orphans ("Charged") says a charged orphan that can't be checked further stays charged and `status` flags it for its owner or root
r2-concurrency B5 | adopting an orphan whose job is still running runs the job twice and drops its reservation | scenario adoption_refused_while_orphan_job_runs real phase 5
r2-concurrency B6 | an orphan left marked as being killed turns off the kill policy for a day | scenario dead_holders_stop_mark_ignored both phase 8
r2-concurrency B7 | after a sleep or a clock change every waiter looks stalled and a later ticket is admitted ahead of earlier ones | scenario nobody_jumps_the_queue_after_the_machine_wakes sim phase 1
r2-concurrency B8 | another user takes a ticket's request directory first, or cleanup removes it in a race with its owner, so cancel requests are lost or spoofed | scenario request_directory_squat_and_sweep_race real phase 5
r2-concurrency B9 | a reader sees a half-written ticket when two writes land between its reads, or a ticket is lost on power failure on macOS | scenario reader_never_sees_torn_generation real phase 2
r2-concurrency B10 | the design and the architecture disagree on who runs the sampler and on which users' leftovers cleanup removes | not a defect: DESIGN's Platforms now says the sampler runs only as the maitred user, optionally under launchd or systemd, matching ARCHITECTURE's Sampler and The sweep

## r2-platforms

r2-platforms A1 | a sandboxed agent's run fails because maitred writes a per-user file outside what the sandbox may write | folded into r2-threat B7
r2-platforms A2 | a process left running after the job, kept by its rule or escaped with `setsid`, still holds the job's output open, so either the supervisor never exits and keeps its reservation, or the kept process dies of a broken pipe on its next write | scenario process_holding_job_output_neither_hangs_supervisor_nor_dies real phase 5
r2-platforms A3 | Ctrl-Z doesn't stop the job | folded into r2-platforms B2
r2-platforms A4 | a job's child that left the job's session runs maitred and waits for ever behind its own parent | folded into r2-concurrency AF4
r2-platforms A5 | a sampler that runs but can't read memory or CPU keeps the load file fresh, so every job waits for ever instead of being admitted one at a time | scenario missing_memory_reading_falls_back_to_one_at_a_time sim phase 1
r2-platforms A6 | a ticket made inside an agent sandbox carries no boot id, because the sandbox refuses the read, so its orphan stays charged for a day | scenario supervisor_reads_boot_id_inside_agent_sandbox real phase 9
r2-platforms A7 | on macOS the sampler, running as its own user, can't read any job's CPU or memory | folded into r2-admission B1
r2-platforms A8 | a system cleaner deletes the long-untouched lock files under `/var/tmp`, so two processes each hold the admission or sampler lock | folded into r2-concurrency A3
r2-platforms A9 | inside a container the capacity or free memory is read wrong (no limit set, cgroup v1, a CPU set, page cache counted as used, a quota under one core), so too many jobs start or none ever does | scenario cgroup_capacity_and_working_set_read_at_their_edges real phase 3
r2-platforms A10 | an orphaned job that is still running is treated as gone because its leader's start time can't be checked | folded into r2-platforms B9
r2-platforms A11 | a harness that kills the caller's process group to end a timed-out call kills maitred but not the job, which runs on in its own session | not a defect: Signals ("supervisor killed with KILL") and The job's output and input state that a job whose supervisor is killed runs on under its holder as a running orphan, which `status` shows
r2-platforms A12 | closing the terminal kills a job the user started under `nohup` | scenario nohup_job_survives_terminal_hangup real phase 5
r2-platforms A13 | a supervisor stopped while holding the queue's lock freezes admission until it is resumed | folded into r2-concurrency A10
r2-platforms A14 | a job can't be given a terminal inside the agent sandbox, which doesn't allow opening one | out of v1: ARCHITECTURE's The job's output and input (jobs get pipes, never a terminal), and Build order ("Not in v1: terminal mode")
r2-platforms B1 | starting a job hangs for ever, because the step that waits for the go-ahead blocks the call that starts the helper process | folded into r2-concurrency B1
r2-platforms B2 | Ctrl-Z does nothing to the job, because the system discards stop signals sent to a process group with no parent outside it | scenario ctrl_z_stops_job_and_fg_resumes_it real phase 5
r2-platforms B3 | processes a rule says to keep running die when the job's main process exits | scenario kept_leftovers_survive_job_exit real phase 5
r2-platforms B4 | on macOS a sandboxed supervisor can't read the boot id, so it can't tell an old orphan from a current one | not a defect: the report refuted it as written, and ARCHITECTURE's Packages ("Boot id, macOS") and The ticket's payload (`boot`) use the sandbox-allowed read the report proposed
r2-platforms B5 | on macOS no job's CPU or memory can be read, so every job counts as using nothing and too many are admitted | folded into r2-admission B1
r2-platforms B6 | Ctrl-Z while a supervisor holds the queue's lock freezes the whole machine's queue | folded into r2-concurrency A10
r2-platforms B7 | the caller's terminal is left in a changed state when maitred exits by some paths | scenario caller_terminal_unchanged_on_every_exit real phase 5
r2-platforms B8 | the sampler wrongly decides whether it is in a container, so it rations the wrong capacity | scenario container_detected_from_own_cgroup real phase 3
r2-platforms B9 | an orphaned job that is still running is treated as gone, and its reservation freed, because its leader can't be checked from another user or sandbox | scenario orphan_with_uncheckable_leader_stays_charged real phase 5
r2-platforms B10 | Ctrl-C ends with a forced kill that destroys jobs which handle it, such as interactive shells, and jobs meant to ignore it are made to obey it | scenario ctrl_c_passed_once_and_ignores_kept real phase 5
r2-platforms B11 | a caller that sends output and errors to one file sees them out of order | scenario shared_output_and_error_keep_order real phase 5
r2-platforms B12 | the caller's output stays open after the job ends, while kept processes are still being logged, so the caller waits for them | scenario drain_releases_caller_output real phase 5
r2-platforms B13 | the design and the architecture disagree on how the sampler is kept running, the design has no install step, and containers have no service manager to run it | not a defect: DESIGN's Platforms now describes `maitred install machine`, the sampler as the maitred user under launchd or systemd, and the container's entrypoint, matching ARCHITECTURE's Process model and modules and Sampler

## r2-threat

r2-threat A1 | an agent writes its own ticket with a smaller reservation, and the machine is overloaded by jobs admitted on that figure | not a defect: ARCHITECTURE's Risks and open questions (item 2) accepts that same-user processes can rewrite each other's tickets, and DESIGN's Principle and threat model says maitred coordinates and doesn't contain
r2-threat A2 | the kill policy acts on memory only, its CPU claim is false, and an honest heavy job is killed first | folded into r2-admission B7
r2-threat A3 | in `maitred -- A && B` only A is queued, and a heavy B runs without the queue | scenario phase1_classification sim phase 1
r2-threat A4 | moving a branch to a commit with a different class file goes unnoticed | folded into r2-threat B9
r2-threat A5 | an agent runs a heavy job from a throwaway repository whose class file calls it light, so it runs as light with no flag and its records land in the throwaway project | not a defect: Class files, Reading the class file step 1 states that a worker chooses its project by its working directory and that records and `status` show each project's path, and DESIGN's Principle and threat model says maitred coordinates and doesn't contain
r2-threat A6 | inside a light job, a nested maitred run of a heavy command runs straight through with no class, reservation or record of its own | scenario nested_run_of_stricter_class_is_refused real phase 5
r2-threat A7 | any user can hold the queue's lock and stop admission for everyone | not a defect: ARCHITECTURE's Risks and open questions (item 8) accepts it as inherent to flock
r2-threat A8 | a non-interactive `wait` on an orphaned job hangs for up to a day | scenario noninteractive_wait_on_orphan_exits_or_waits_for_its_job real phase 5
r2-threat A9 | one broken line in the class file makes maitred refuse every heavy job, or leaves the hook with no rules so it refuses nothing | scenario phase1_classification sim phase 1
r2-threat A10 | the hook either queues every unlisted command, `ls` included, or protects nothing | folded into r2-threat B2
r2-threat A11 | another user can read a running job's command line through the process id in its ticket | not a defect: ARCHITECTURE's Risks and open questions (item 14) accepts it as no more than `ps` already shows
r2-threat A12 | a release build accepts a machine-config flag, so a supervisor admits itself under limits it chose | folded into r2-threat B6
r2-threat A13 | with `--quiet` the caller gets no ticket id to find the record by, and a refusal before the ticket exists is confused with a job's failure | scenario quiet_run_still_prints_ticket_id real phase 5
r2-threat A14 | a command that only mentions a keep-awake program as an argument, such as `rg caffeinate`, is refused | scenario phase1_classification sim phase 1
r2-threat B1 | cloning a local repository makes an edited class file count as the reviewed one, so its looser classes go unflagged | not a defect: ARCHITECTURE's Class files (Reading the class file, step 2) compares against the committed and default-branch copies only to show edits, "for seeing, not stopping", and DESIGN's Feature discovery settles the class file as plain config with edits recorded
r2-threat B2 | the hook either refuses every unlisted command, `ls` included, or protects nothing | scenario phase1_classification sim phase 1
r2-threat B3 | a heavy program run as `./bin/x`, by absolute path, or behind `bundle exec`, `exec`, `xargs` or `nice -n` slips past the hook | scenario phase1_classification sim phase 1
r2-threat B4 | an agent tidying up deletes every ticket, and nobody notices that the queue lost them | scenario deleted_tickets_recreated_and_nobody_admitted_meanwhile both phase 6
r2-threat B5 | an environment variable picks a class file looser than the project's | scenario class_file_not_overridable_from_environment real phase 5
r2-threat B6 | a release build still lets the machine root be moved | scenario release_build_has_no_debug_overrides real phase 2
r2-threat B7 | a sandboxed agent's run fails because maitred writes somewhere the installed sandbox allowlist doesn't cover | scenario every_write_inside_sandbox_allowlist real phase 9
r2-threat B8 | a job fans out into any number of nested maitred runs, past its own reservation | scenario nested_fan_out_counts_as_parent_overuse both phase 8
r2-threat B9 | the class file is compared against the wrong branch or remote, so an edit isn't flagged | scenario default_branch_resolved_from_origin_head real phase 5
r2-threat B10 | a hook-wrapped `a & b` kills the backgrounded `a` when `b` ends | impossible by construction: ARCHITECTURE's Class files ("classes check, classes guard, the guard line and the hook") says the hook only refuses and rewrites nothing, so `a` runs in the caller's own shell, outside any job's session

## r3-admission

r3-admission A1 | a second job is killed for one breach because the wait after the first kill ends on a reading still taken while the first job was alive | scenario kill_second_victim_waits_for_reading_taken_after_first_is_gone both phase 8
r3-admission A2 | a breach goes on with no further kill for as long as the chosen job's own controlling process is suspended | scenario kill_policy_not_frozen_while_victims_controller_is_stopped real phase 8
r3-admission A3 | the machine stays overloaded and no job is killed because each running job's check names a different job as the one to stop | scenario kill_victim_acts_when_job_figures_oscillate_between_readings sim phase 8
r3-admission A4 | jobs that stay within their reservations are killed one after another while the job actually using the memory, whose launching process is stopped or gone, keeps running | scenario kill_chooses_over_reservation_orphan_not_in_budget_jobs both phase 8
r3-admission A5 | a single build using every core it reserved, alone on the machine, is killed for using the CPU | scenario kill_cpu_not_triggered_by_one_build_saturating_its_reserved_cores sim phase 8
r3-admission A6 | while anyone holds the admission lock, no job is killed however far memory or CPU goes over the kill thresholds | not a defect: "Risks and open questions" item 8 states that whoever holds the admission lock stalls admission and the kill policy, as inherent to flock, and "Signals" says the same for a process stopped with SIGSTOP while holding it
r3-admission A7 | a killed job's children that moved to another process group survive the kill and keep the memory the job was ranked by | impossible by construction: "Kill policy" step 3 signals every process of the session and waits until the session's processes are gone, the same set "Measuring the job" sums, so what is ranked and what is killed are one set of processes
r3-admission A8 | a job larger than the free memory starts when nothing else is running, then is killed for low memory, and a retry repeats the cycle | not a defect: "Risks and open questions" item 10 states that a job bigger than free memory that gets in with nothing else running is admitted and then killed at the memory threshold, and that the skill tells agents not to retry it blindly (DESIGN, "Nobody waits on their own size")
r3-admission A9 | more jobs start than the CPU allows because a running job's use was read at a quieter moment than the machine's total | scenario admission_not_overcommitted_when_job_use_and_machine_reading_differ_in_time real phase 3
r3-admission A10 | a job's memory from files it has mapped is counted both as used by the job and as free, so more jobs start than memory allows | scenario measure_mapped_file_pages_not_counted_as_job_memory real phase 3
r3-admission A11 | a build that runs many short-lived compiler processes is measured at a fraction of the CPU it uses, so it is ranked as innocent when choosing whom to kill | scenario measure_fork_heavy_build_counts_short_lived_children_cpu real phase 3
r3-admission A12 | after Ctrl-Z, waiting jobs start in the suspended job's cores, and the CPU is overcommitted when it resumes | scenario suspended_job_keeps_its_cpu_reservation_held real phase 6
r3-admission A13 | a job queued later starts before jobs queued earlier that spent a long time waiting for the admission lock | scenario long_admission_lock_wait_does_not_let_newcomer_jump_queue real phase 6
r3-admission A14 | after the machine wakes from sleep, a job jumps ahead of earlier jobs whose waiting processes have not run yet | scenario nobody_jumps_the_queue_after_the_machine_wakes sim phase 1
r3-admission A15 | right after the machine wakes, a job is killed, or a job that needs a quiet machine is started, because of counts carried over from before the sleep | scenario kill_and_quiet_counts_restart_after_sleep sim phase 8
r3-admission A16 | a job that needs a quiet machine, once it has waited long enough, blocks every job queued behind it for as long as some long-running or orphaned job is still using its reservation | scenario due_quiet_job_isnt_held_off_by_an_orphan sim phase 1
r3-admission A17 | a large job that can never fit keeps its share of the machine set aside for ever, so nothing queued behind it starts | scenario job_that_can_never_fit_doesnt_hold_the_queue_for_ever sim phase 1
r3-admission A18 | an older maitred undercharges a job queued by a newer one and cannot tell whether that job's waiting process is suspended | scenario older_reader_charges_newer_ticket_its_full_reservation_and_judges_its_stall real phase 2
r3-admission A19 | with the load reading unavailable, no job ever starts because one queued job's file cannot be read by the others | scenario fallback_not_blocked_by_newer_version_or_unreadable_locked_ticket sim phase 1
r3-admission A20 | after the queue's files are deleted and restored, a job queued later runs ahead of jobs queued before it, and different waiters disagree on the order | scenario queue_order_kept_after_ticket_files_deleted_and_restored real phase 6
r3-admission A21 | while a running job's file is deleted, a second job is killed for the same breach, or new jobs start in the deleted job's reservation | scenario deleted_stop_marker_does_not_allow_second_victim real phase 8

## r3-processes

r3-processes P1 | on macOS, maitred can't find the processes of a job's session, so the job is measured as using nothing and its leftover processes are never stopped | scenario macos_session_scan_finds_every_job_process_in_sandbox real phase 3
r3-processes P2 | Ctrl-Z stops the job but the caller's shell never gets its prompt back (and, with a pseudo-terminal, the terminal is left in raw mode) | folded into r4-processes P6 (the prompt; jobs get pipes in v1, and the raw-mode half is out of v1: Build order, "Not in v1: terminal mode")
r3-processes P3 | maitred has no way to learn the job's exit status, stops or end from the process that started it, so it exits with the wrong status or never notices the job ended, worst when leftover processes are kept | scenario supervisor_exits_with_job_status_while_leftovers_kept real phase 5
r3-processes P4 | a process in the job that moves itself into its own process group keeps running after the job is stopped, cancelled or killed, uncharged, and can let the job be run a second time | scenario setpgid_child_stopped_with_job_and_keeps_ticket real phase 5
r3-processes P5 | killing maitred with KILL also takes the job down (it dies writing to output nobody reads), instead of leaving it running as an orphan | scenario job_survives_supervisor_kill_and_output_still_logged real phase 5
r3-processes P6 | `id=$(maitred submit ...)` blocks until the job ends and captures the job's output, and a harness's timeout kills the detached job with the caller | scenario submit_captured_by_command_substitution_returns_at_once real phase 5
r3-processes P7 | a helper process forked from maitred without exec can hang on a lock, keep a finished job's ticket looking alive while a kept server runs, or die with the caller's process group and break the kept server's output | scenario drain_survives_caller_group_kill_without_holding_ticket real phase 5
r3-processes P8 | Ctrl-C on a shell loop around maitred ends only the current iteration, because maitred exits with a number instead of dying by the signal | folded into r4-processes P7 (one scenario; it checks both that maitred dies by the signal and that the loop gets it)
r3-processes P9 | on Apple Silicon every job's CPU use reads about 42 times too low, so CPU overuse is never seen and the CPU kill falls back to the most recent job | scenario apple_silicon_cpu_use_matches_burn real phase 3
r3-processes P10 | a build made of many short-lived compiler processes reads as nearly idle, because processes that exit between passes are never counted | scenario short_lived_children_counted_in_cpu_use real phase 3
r3-processes P11 | the job never starts because the process meant to run it finds its go-ahead channel already closed when that channel happened to be created on descriptor 3 | impossible by construction: Supervision and cleanup, "From enqueue to record" step 5 moves every descriptor above the highest target before placing it, so a descriptor is never placed onto its own number and close-on-exec is always cleared
r3-processes P12 | the job starts with signals blocked or ignored that it inherited from maitred, so a forwarded TERM or INT never reaches it and only KILL stops it | scenario job_starts_with_empty_mask_and_default_dispositions real phase 5
r3-processes P13 | Ctrl-Z stops maitred while it holds the machine-wide admission lock, so no job on the machine is admitted (or killed) until it is resumed | scenario ctrl_z_while_holding_admit_lock_defers_stop real phase 6
r3-processes P14 | a stopped job that handles TERM is killed with KILL after the grace period without ever getting to clean up | scenario stopped_job_with_term_handler_cleans_up_before_kill real phase 5
r3-processes P15 | a job queued before maitred's binary is upgraded fails to start when admitted, or starts under a helper of the new version that misreads the old one | scenario binary_upgraded_mid_wait_job_still_starts real phase 5
r3-processes P16 | when the process that started the job is killed, the job runs on unsupervised while maitred records it as finished and frees its reservation | scenario holder_killed_supervisor_stops_session_and_keeps_ticket real phase 8
r3-processes P17 | a job whose output is redirected or piped gets pipes even for streams that are terminals, so its colour, progress bars and terminal checks change | not a defect: "The job's output and input" states that in v1 the job always gets pipes, never a terminal, and DESIGN's Supervision says a job that needs a real terminal (colour on by default) should run without maitred
r3-processes P18 | on Linux, memory of a non-dumpable or setuid process in the job silently drops out of the job's use | scenario non_dumpable_process_memory_still_counted real phase 3
r3-processes P19 | a later sandboxed call can't signal or probe an orphaned job left by an earlier call, so whether that job is still alive rests on its start time alone | scenario orphan_liveness_judged_across_sandboxes real phase 5

## r3-threat

r3-threat T1 | a rule weakened by a local commit on the default branch runs with no flag, while an uncommitted edit is flagged | not a defect: DESIGN Class file treats the class file as ordinary project config changed through the project's own review, and ARCHITECTURE Reading the class file flags only working-tree differences from the committed copies ("for seeing, not stopping"); `maitred install claude-code` write-denies the class file, so a sandboxed agent can't make the edit such a commit needs
r3-threat T2 | a stricter rule merged upstream is not in force on a machine whose local default branch was never updated | not a defect: Reading the class file takes the rules from the working tree, the checkout's own config (DESIGN Class file), so the local default branch no longer decides which rules apply, and a checkout without the rule runs without it as intended
r3-threat T3 | a tag named like the default branch is read as the default branch's class file when the branch itself is missing, so a changed class file is compared against the wrong copy | scenario default_branch_class_file_ignores_tag_named_like_branch real phase 5
r3-threat T4 | an agent that points the remote's default-branch pointer at its own branch makes its committed class-file change look unedited, and no record says which branch was compared | scenario retargeted_remote_default_branch_still_flags_edited_classes real phase 5
r3-threat T5 | a lighter or unqueued class for a heavy command gets past the "no weaker than committed" check by adding, renaming or widening a rule in a separate override file | impossible by construction: Reading the class file says no run ever uses a draft file, so there is no override file to compare; edits to the class file itself are flagged
r3-threat T6 | an edited checker in the working tree turns a heavy job light with nothing in the record saying the checker changed | scenario edited_checker_is_flagged_in_job_and_record real phase 5
r3-threat T7 | a script that a run-directly rule names is edited to do heavy work and then runs unqueued with no record | not a defect: DESIGN Principle and threat model accepts that an agent can edit a script a rule names, and ARCHITECTURE Risks and open questions item 1 says such load is outside admission until it shows in the samples
r3-threat T8 | a heavy command written with command substitution, a subshell or after a heredoc runs unqueued because the hook gives up on the string | scenario phase1_classification sim phase 1
r3-threat T9 | a heavy command run after changing into another project's directory, or by absolute path from inside a nested clone, is checked against the wrong project's rules and runs unqueued | scenario heavy_program_reached_by_cd_or_absolute_path_is_queued_by_its_guard real phase 5
r3-threat T10 | a heavy command behind a wrapper or launcher maitred doesn't strip (nice -5, timeout -k, gtimeout, env -C, bundle exec from bin/) runs unqueued, and a lookup like command -v is queued as if it ran the program | scenario phase1_classification sim phase 1
r3-threat T11 | a backgrounded heavy command that the hook wrapped is killed seconds after starting because the wrapping shell exited at once | impossible by construction: the hook paragraph under `classes check`, `classes guard`, the guard line and the hook says the hook only refuses and rewrites nothing, so it never wraps an agent's string in a shell
r3-threat T12 | a command string that runs several queued programs gets the class, slots or cleanup of only one of them | scenario phase1_classification sim phase 1
r3-threat T13 | a hooked command behaves differently because the hook re-ran it under a different shell than the one the agent used | impossible by construction: the hook paragraph under `classes check`, `classes guard`, the guard line and the hook says the hook never rewrites or re-runs a command, so what runs is the agent's own string in its own shell
r3-threat T14 | a run from a checkout whose git directory or records directory can't be written fails in an unspecified way, possibly after its ticket exists | scenario unwritable_records_directory_refuses_with_73_before_any_ticket real phase 4
r3-threat T15 | an orphan from another project can't be expired by a sandboxed supervisor, so it is never removed and every supervisor retries it on every pass | scenario sweep_expires_only_its_own_projects_orphans real phase 5
r3-threat T16 | an agent deleting the request directories leaves tickets that can't be cancelled and orphans that never get their expired record | scenario deleted_request_directory_is_recreated_and_orphan_still_gets_record real phase 5
r3-threat T17 | a new ticket's request directory is deleted by another supervisor's cleanup while the new ticket waits for the admission lock, so the ticket appears with no request directory | scenario request_directory_survives_until_its_ticket_is_created real phase 5
r3-threat T18 | an orphan whose worktree was removed has no reachable place for its expired record | scenario orphan_of_removed_worktree_still_gets_record real phase 5
r3-threat T19 | two racing first runs, or a copied repository, give one project two identities or two projects one identity, breaking cancel scope and adoption | impossible by construction: The ticket's payload says the project id is a hash of the real path of the git common directory with nothing stored, so racing runs agree and a copy at another path is another project
r3-threat T20 | a job run outside git with no project marker has nowhere to put its record and fails in an unspecified way | scenario run_outside_any_project_refuses_with_78 real phase 4
r3-threat T21 | the file modes look like they keep one agent from touching another's tickets, though every agent runs as the same user | not a defect: Where things live, and who owns them says the modes separate users only and that maitred makes no claim between processes of one user; Risks and open questions item 2 repeats it
r3-threat T22 | every queued command fails on a machine where maitred was never installed (CI runners, a contributor's laptop, a fresh container) | scenario missing_machine_root_runs_command_directly_with_one_warning real phase 4
r3-threat T23 | an agent cancels another project's job without the other-project flag by first changing into that project's directory | not a defect: Requests and scope says both scope checks are advisory, that a sender can change directory, and that the record names the sender's working directory, project, ticket and pid
r3-threat T24 | cancelling an orphaned job reports success while the job keeps running and stays charged | scenario cancel_on_orphan_exits_69 real phase 5
r3-threat T25 | a non-interactive wait returns at once on an orphan whose job is still running, so the agent resubmits and the job runs twice | scenario noninteractive_wait_on_running_orphan_blocks_until_job_ends real phase 5
r3-threat T26 | a job's own exit status 75 is read as maitred timing out before admission | not a defect: Exit statuses maitred chooses says no status is unique, maitred result is authoritative, and the skill tells agents to read the maitred: line, naming 75 in particular

## r4-classify

r4-classify C1 | a light command followed by a pipe into an ordinary helper such as `tail` is queued as heavy, and its reservation is inflated by the helper | scenario phase1_classification sim phase 1
r4-classify C2 | a commit or note whose heredoc text mentions a queued program is itself queued or refused | scenario phase1_classification sim phase 1
r4-classify C3 | a queued program after a bash or zsh operator such as `|&`, `&|` or `&!` is missed, or `&>` splits a command in the wrong place | scenario phase1_classification sim phase 1
r4-classify C4 | a queued program inside a loop, conditional, `case` branch or braces runs unqueued because nothing sees it as a command | scenario phase1_classification sim phase 1
r4-classify C5 | a heavy program placed after a maitred command in the same string, such as `maitred status && bin/mutant run`, runs unqueued | scenario phase1_classification sim phase 1
r4-classify C6 | a heavy program started through another program (`hyperfine`, `git bisect run`, `find -exec`, `bash bin/x`) or through a path built from an expansion runs unqueued | scenario guarded_program_queued_when_started_indirectly both phase 5
r4-classify C7 | a heavy program run by absolute path, or after changing into another project, runs unqueued because the caller's project has no rule for it | scenario guarded_program_run_from_another_project_is_queued real phase 5
r4-classify C8 | a class file that asks for a pool one machine doesn't have, or has one malformed or newer row, sends every command of every agent in the project to the heavy queue | scenario phase1_classification sim phase 1
r4-classify C9 | a command the hook says to queue is then refused by maitred because its checker answered differently or ran in the wrong directory, so it can never run | scenario checker_runs_in_job_directory_and_cannot_refuse real phase 5
r4-classify C10 | the agent's command behaves differently once wrapped: a directory change doesn't persist, its shell functions are missing, or a different shell runs it | impossible by construction: the hook never rewrites a command, it only refuses and names the command to run (Class files, "`classes check`, `classes guard`, the guard line and the hook"), so what runs is exactly what the agent typed
r4-classify C11 | a queued job waiting inside a foreground call is killed by the harness's call timeout, and the retry goes to the back of the queue | not a defect: the Signals table states that TERM while waiting records `abandoned` and deletes the ticket, the hook only refuses and names a background call (Class files, "`classes check`, `classes guard`, the guard line and the hook"), and the skill's rules under Build order say to run maitred as a background call
r4-classify C12 | a command the person's permission rules deny runs anyway because it is hidden inside a maitred call, and a docker call loses its sandbox exemption | not a defect: DESIGN's Principle and threat model defines "adds no power" at the sandbox, and the hook only refuses (Class files, "`classes check`, `classes guard`, the guard line and the hook"), so maitred hides no command from the harness; an agent has to type the inner command itself, and a refusal for a container queue names a command that starts with `docker compose exec`
r4-classify C13 | a command that changes into a project before running a queued program can't be queued at all, because the queued call has no project or more than one | scenario refused_cd_string_has_a_runnable_queued_form real phase 9
r4-classify C14 | commands combined in one call get the wrong size or slots: parallel suites share one slot number, sequential ones take two, and `make -C dir release` isn't recognised | scenario phase1_classification both phase 1
r4-classify C15 | a nested queued run that needs a slot its parent doesn't hold runs without one and collides with another suite | scenario nested_run_needing_unheld_slot_is_refused real phase 5
r4-classify C16 | a test suite started on the host through `docker compose exec` runs in the shared container with no ticket and no slot | scenario container_exec_of_guarded_program_queues_in_container real phase 7
r4-classify C17 | inside a shared container where jobs run as root, a job can delete the machine root or rewrite the machine config, so later runs go unqueued | not a defect: Risks and open questions item 2 states that nothing is out of reach inside a container that runs maitred as root, and DESIGN's Platforms ("Root inside a container that is its own machine") accepts it with `status` saying so
r4-classify C18 | a ticket id issued inside the container reads as lost from the host while the job still runs, so the job is run a second time | scenario host_wait_on_container_ticket_id_does_not_report_lost real phase 7
r4-classify C19 | killing the host's `docker compose exec` client leaves the queued or running job going inside the container while the caller thinks it stopped | scenario compose_exec_client_killed_stops_job_in_container real phase 7
r4-classify C20 | when git can't read the repository inside a container, or the class file is missing at the commit, the run is refused or its record wrongly says whether the class file was edited | scenario git_failure_marks_classes_unverified_and_runs real phase 5
r4-classify C21 | a shared container with no resource limits of its own rations the whole Docker VM, and an image built without the install silently runs every job unqueued | scenario unlimited_or_uninstalled_container_is_reported real phase 3
r4-classify C22 | the slot number a pool hands out doesn't match the test tool's own numbering, so a suite uses a database that was never created or collides with another | scenario slot_numbers_match_test_env_convention real phase 7
r4-classify C23 | with no machine root, `submit` gives an id that `wait` reports lost, and the hook still refuses commands it has nowhere to queue | scenario no_machine_root_submit_refuses_and_hook_allows real phase 4
r4-classify C24 | a class file edited and then committed on the agent's own branch is no longer flagged as edited | scenario class_file_committed_on_branch_still_flagged_edited real phase 5

## r4-processes

r4-processes P1 | a job run under maitred starts with broken-pipe signals ignored, so a pipeline into `head` inside it prints errors and fails where it wouldn't without maitred | scenario job_starts_with_default_signals_and_empty_mask real phase 5
r4-processes P2 | a Ctrl-C, Ctrl-Z or TERM meant for the job also hits maitred's process that leads the job's session, so the job is recorded as `holder lost` instead of handled | scenario session_signals_never_reach_the_holder real phase 5
r4-processes P3 | a process the job leaves running keeps maitred's internal pipes open after the session's leading maitred process dies, hiding that death | scenario job_inherits_no_maitred_descriptor_but_its_slots real phase 5
r4-processes P4 | with `submit`, a daemon left by the job keeps a dead supervisor's ticket looking alive, so it is never swept or adopted | scenario submit_daemon_does_not_keep_ticket_locked real phase 5
r4-processes P5 | in terminal mode on macOS, killing maitred with KILL leaves the job running against a terminal nobody reads | out of v1: The job's output and input
r4-processes P6 | Ctrl-Z at a terminal stops maitred but not the script or `make` that called it, so the shell never gets its prompt back | scenario ctrl_z_returns_prompt_through_wrapping_script real phase 5
r4-processes P7 | Ctrl-C at a terminal ends the job but not a script's loop around maitred, so every iteration needs its own Ctrl-C | scenario ctrl_c_stops_shell_loop_around_maitred real phase 5
r4-processes P8 | maitred run in the background (`maitred -- x &`, or `bg` after Ctrl-Z) stops itself at once instead of running the job in the background | scenario background_maitred_runs_without_stopping_or_touching_terminal real phase 5
r4-processes P9 | in terminal mode the job stops itself at start, so it prints nothing and maitred suspends | out of v1: The job's output and input
r4-processes P10 | with stdout redirected, the job is handed the user's terminal as stdin outside the shell's job control, so a background job reads keystrokes typed for the shell | scenario background_job_never_reads_shell_keystrokes real phase 5
r4-processes P11 | an exited but unreaped session-leading maitred process still counts as alive, so once its job is picked as the victim no later breach kills anything | scenario zombie_holder_is_never_a_victim real phase 8
r4-processes P12 | the CPU time measured for a job drops when one of its processes outlives its parent, so the job's use goes negative or a pass's real CPU is hidden | scenario cpu_total_never_drops_when_parent_exits_first real phase 3
r4-processes P13 | placing several inherited descriptors one after another overwrites one with another, so two live jobs can share a slot number | scenario two_slot_job_gets_each_slot_under_its_own_number real phase 5
r4-processes P14 | a job killed after its maitred supervisor has died is left without a record | scenario holder_tells_dead_supervisor_from_stopped_one real phase 8
r4-processes P15 | upgrading maitred while a job waits makes the job fail to start once admitted | scenario binary_upgraded_mid_wait_still_runs_and_drains real phase 5
r4-processes P16 | the process that keeps reading a kept server's output stays in the caller's process group, so a Ctrl-C or harness timeout aimed at the caller kills it and the kept server dies on its next write | scenario kept_server_survives_signal_to_caller_group real phase 5
r4-processes P17 | on Linux maitred's helper processes show as `exe`, so `ps`, `pgrep` and `pkill maitred` miss them | scenario helpers_named_maitred_in_process_list real phase 5
r4-processes P18 | on Linux kernels without the per-process anonymous and shared memory fields, a job's memory can't be measured | scenario memory_falls_back_and_marks_estimated_without_pss_fields real phase 3

## r4-state

r4-state S1 | after one CPU spike, a second and third job are killed while the averaged pressure figure is still falling from the first | scenario kill_cpu_streak_rebuilds_after_victim_gone both phase 8
r4-state S2 | on a memory or CPU breach, a job using well under its share is killed while the job actually using far more goes untouched because no current figure was published for it | scenario kill_unmeasured_overusing_job_is_not_passed_over both phase 8
r4-state S3 | an agent tidying the queue directory removes the sign that a kill is under way, and a second job is killed for the same breach | scenario kill_deleted_stop_marker_gives_no_second_victim real phase 8
r4-state S4 | a killed job with a process that cannot die (stuck in the kernel, or running as another user) turns off all further kills for as long as that process lives | scenario kill_unkillable_process_releases_policy_after_kill_wait real phase 8
r4-state S5 | the job chosen to be killed belongs to a helper that is blocked, so nothing is killed and the breach goes on unanswered | scenario kill_blocked_victim_helper_does_not_leave_breach_unanswered real phase 8
r4-state S6 | a second test suite is handed a test-environment number that a running suite, or a server it left behind, is still using | scenario slot_stays_taken_while_job_or_its_daemon_lives real phase 6
r4-state S7 | a command run from inside another job runs without its own test-environment number and shares the database and ports of a suite already running | scenario nested_run_without_parent_slot_is_refused real phase 5
r4-state S8 | servers kept running after their test runs use up every test-environment number, so later jobs either start without a number or wait forever with nothing saying who holds the numbers | scenario first_in_waits_for_slot_held_by_kept_leftovers both phase 6
r4-state S9 | a job is admitted on the strength of a test-environment number that turns out to be taken, and is left admitted with no number to run under | scenario slot_held_outside_any_ticket_never_admits_job_without_number real phase 6
r4-state S10 | a background submit from inside a running job escapes the job, so its work is unqueued, unmeasured and out of reach of cancel and kills | scenario submit_inside_job_is_refused real phase 5
r4-state S11 | after an agent deletes the queue entry of a job whose supervising process is stopped or dead, that job keeps running uncounted and can no longer be chosen for a kill, and others are admitted into its share | scenario deleted_ticket_of_orphaned_job_stays_charged_and_killable real phase 8
r4-state S12 | right after the machine wakes, or after its clock jumps, a job that arrived later is admitted ahead of jobs that were waiting before it | scenario wake_or_clock_step_does_not_let_later_arrival_jump_queue both phase 6
r4-state S13 | a large or quiet job that has waited past its hold time never starts while smaller jobs keep arriving | scenario held_job_admitted_eventually_under_steady_light_traffic sim phase 6
r4-state S14 | with the load-sampling service stopped, deleting the queue's entries lets a newcomer go ahead of jobs that were already waiting | scenario queue_order_kept_after_deletion_with_sampler_down both phase 6
r4-state S15 | deleting only the per-job request folders pauses all admissions and kills for several seconds, and indefinitely if repeated | scenario deleting_request_dirs_does_not_pause_admission_or_kills real phase 6
r4-state S16a | two jobs admitted in the same millisecond can make different helpers pick different victims, so two jobs are killed or none | scenario kill_recency_tie_broken_same_way_by_every_holder sim phase 8
r4-state S16b | with the load-sampling service stopped, an orphaned job that never ends blocks every admission | scenario sampler_down_orphan_does_not_block_all_admission both phase 6
r4-state S16c | maitred's own message about waiting for its lock appears in the job's output and log | scenario holder_lock_wait_message_not_in_job_output real phase 8
r4-state S16d | running maitred clean at the wrong moment removes the sign that a kill is under way, and a second job is killed for the same breach | scenario clean_never_removes_stop_marker_being_created real phase 8
r4-state S16e | a command run from inside a job whose queue entry was deleted or lost its supervisor queues behind its own parent, and both wait until the timeout | scenario nested_call_under_orphaned_or_deleted_parent_does_not_deadlock real phase 5
r4-state S16f | a maitred of another version reading the queue does not notice that an entry was just recreated, and admits or kills while others may still be missing | impossible by construction: "The ticket: two slots and a lock" puts recreated_at in the fixed header, which "Versions" says never changes within a major version, so every reader of the same major sees it
