------------------------------ MODULE LoopRun ------------------------------
(***************************************************************************)
(* The Loop Engine overlapping-attempts race, as one protocol.             *)
(* Property: docs/PRD.md:726-730. See formal/README.md for the mapping     *)
(* from each action to the Rust it mirrors, the bounds, and what is left   *)
(* out.                                                                    *)
(*                                                                         *)
(* The model is cut where the code cuts its SQLite transactions. A step    *)
(* that is one BEGIN IMMEDIATE transaction in crates/loop-integrations/    *)
(* src/sqlite.rs is one action here. The reads between transactions, and   *)
(* the provider call, are separate actions: those are the interleaving     *)
(* points.                                                                 *)
(***************************************************************************)
EXTENDS Integers, Sequences

CONSTANTS
    Attempts,    \* the overlapping event attempts, e.g. {a1, a2, a3}
    MaxAppends   \* how many context-only appends the operator may make

\* A small workflow, with every shape the race needs: a checked edge, a
\* check-free edge, a self-loop, a cycle and an edge into a final state.
Events == {"e1", "e2", "e3", "e4"}
Src(e) == CASE e = "e1" -> "A" [] e = "e2" -> "B" [] e = "e3" -> "B" [] e = "e4" -> "A"
Tgt(e) == CASE e = "e1" -> "B" [] e = "e2" -> "A" [] e = "e3" -> "F" [] e = "e4" -> "A"
Checked(e) == e \in {"e1", "e3", "e4"}   \* e2 (B to A) is check-free

VARIABLES
    lifecycle,   \* "Active" | "Final" | "Terminated"
    state,       \* the run's current workflow state: "A" | "B" | "F"
    rev,         \* control_revision
    observed,    \* observed_control_revision; -1 is NULL (never observed)
    hist,        \* the semantic history, oldest first
    appends,     \* context appends made so far (a bound only)
    pc,          \* per attempt: "new" | "read" | "snapped" | "evaluated" | "done"
    ev,          \* per attempt: the event it requests (fixed at the start)
    expRev,      \* per attempt: the control revision its write will assert
    res          \* per attempt: the provider result, "none" | "allow" | "deny"

vars == <<lifecycle, state, rev, observed, hist, appends, pc, ev, expRev, res>>

(***************************** history entries *****************************)
\* Every entry names the revision and source state its writer asserted, and
\* whether the run was observed (observed_control_revision = control_revision)
\* when it was written.
Entry(kind, r, from, to) ==
    [kind |-> kind, rev |-> r, from |-> from, to |-> to, obs |-> (observed = rev)]
IsAdv(e) == e.kind \in {"transition", "terminated"}   \* advances control_revision

RECURSIVE Replay(_)
Replay(h) ==   \* the workflow state that a history implies
    IF h = <<>> THEN "A"
    ELSE IF h[Len(h)].kind = "transition" THEN h[Len(h)].to
    ELSE Replay(SubSeq(h, 1, Len(h) - 1))

(********************************* guards **********************************)
\* Each guard is one check in a persistence transaction. They are separate
\* operators so that the scratch breaks in formal/README.md can delete one.
ActiveGuard == lifecycle = "Active"                 \* require_active / is_active
ObservedGuard == observed = rev                     \* require_observed
RevisionGuard(a) == rev = expRev[a]                 \* verify_revision_and_source
SourceGuard(a) == state = Src(ev[a])                \* verify_revision_and_source
CommitGuard(a) ==
    /\ ActiveGuard
    /\ RevisionGuard(a)
    /\ SourceGuard(a)
    /\ ObservedGuard

(********************************** init ***********************************)
Init ==
    /\ lifecycle = "Active"
    /\ state = "A"
    /\ rev = 0
    /\ observed = -1
    /\ hist = <<>>
    /\ appends = 0
    /\ pc = [a \in Attempts |-> "new"]
    /\ ev \in [Attempts -> Events]
    /\ expRev = [a \in Attempts |-> 0]
    /\ res = [a \in Attempts |-> "none"]

(************************ operator and observer actions ********************)
\* `show`: sets observed_control_revision = control_revision (sqlite.rs:953-958).
\* It works on a run in any lifecycle.
Observe ==
    /\ observed /= rev
    /\ observed' = rev
    /\ UNCHANGED <<lifecycle, state, rev, hist, appends, pc, ev, expRev, res>>

\* `append`: a context-only write. It needs an active, observed run and does
\* not advance control_revision (sqlite.rs:438-502).
AppendContext ==
    /\ ActiveGuard
    /\ ObservedGuard
    /\ appends < MaxAppends
    /\ appends' = appends + 1
    /\ hist' = Append(hist, Entry("context", rev, state, state))
    /\ UNCHANGED <<lifecycle, state, rev, observed, pc, ev, expRev, res>>

\* `terminate`: one transaction (sqlite.rs:728-761).
Terminate ==
    /\ ActiveGuard
    /\ ObservedGuard
    /\ lifecycle' = "Terminated"
    /\ rev' = rev + 1
    /\ hist' = Append(hist, Entry("terminated", rev, state, state))
    /\ UNCHANGED <<state, observed, appends, pc, ev, expRev, res>>

(*********************** one event attempt, step by step *******************)
\* Read the run, resolve the event, check the observation (event.rs:80-133).
\* The attempt records the revision it saw. A request that cannot start ends
\* with no write.
LoadOk(a) ==
    /\ pc[a] = "new"
    /\ ActiveGuard /\ SourceGuard(a) /\ ObservedGuard
    /\ pc' = [pc EXCEPT ![a] = "read"]
    /\ expRev' = [expRev EXCEPT ![a] = rev]
    /\ UNCHANGED <<lifecycle, state, rev, observed, hist, appends, ev, res>>

LoadRejected(a) ==
    /\ pc[a] = "new"
    /\ ~(ActiveGuard /\ SourceGuard(a) /\ ObservedGuard)
    /\ pc' = [pc EXCEPT ![a] = "done"]
    /\ UNCHANGED <<lifecycle, state, rev, observed, hist, appends, ev, expRev, res>>

\* Check-free edge: one commit transaction (event.rs:248-277, sqlite.rs:528-674).
\* Checked edge: the same transaction, after the provider call.
CommitWrite(a) ==
    /\ lifecycle' = IF Tgt(ev[a]) = "F" THEN "Final" ELSE "Active"
    /\ state' = Tgt(ev[a])
    /\ rev' = rev + 1
    /\ hist' = Append(hist, Entry("transition", expRev[a], Src(ev[a]), Tgt(ev[a])))
    /\ pc' = [pc EXCEPT ![a] = "done"]
    /\ UNCHANGED <<observed, appends, ev, expRev, res>>

CommitFree(a) ==
    /\ pc[a] = "read" /\ ~Checked(ev[a])
    /\ CommitGuard(a)
    /\ CommitWrite(a)

\* Checked edge: load the evaluation snapshot (event.rs:345-361,
\* sqlite.rs:891-929). It is a read; it takes the run's revision at that moment.
Snapshot(a) ==
    /\ pc[a] = "read" /\ Checked(ev[a])
    /\ ActiveGuard /\ SourceGuard(a)
    /\ pc' = [pc EXCEPT ![a] = "snapped"]
    /\ expRev' = [expRev EXCEPT ![a] = rev]
    /\ UNCHANGED <<lifecycle, state, rev, observed, hist, appends, ev, res>>

SnapshotRejected(a) ==
    /\ pc[a] = "read" /\ Checked(ev[a])
    /\ ~(ActiveGuard /\ SourceGuard(a))
    /\ pc' = [pc EXCEPT ![a] = "done"]
    /\ UNCHANGED <<lifecycle, state, rev, observed, hist, appends, ev, expRev, res>>

\* The provider call runs outside every persistence transaction
\* (event.rs:365-369). Either answer can come back after the run has moved.
ProviderAllows(a) ==
    /\ pc[a] = "snapped"
    /\ pc' = [pc EXCEPT ![a] = "evaluated"]
    /\ res' = [res EXCEPT ![a] = "allow"]
    /\ UNCHANGED <<lifecycle, state, rev, observed, hist, appends, ev, expRev>>

ProviderDenies(a) ==
    /\ pc[a] = "snapped"
    /\ pc' = [pc EXCEPT ![a] = "evaluated"]
    /\ res' = [res EXCEPT ![a] = "deny"]
    /\ UNCHANGED <<lifecycle, state, rev, observed, hist, appends, ev, expRev>>

CommitAllowed(a) ==
    /\ pc[a] = "evaluated" /\ res[a] = "allow"
    /\ CommitGuard(a)
    /\ CommitWrite(a)

\* A denial is recorded in one transaction with the same guards
\* (sqlite.rs:676-726). It writes history but leaves state and revision alone.
RecordDenial(a) ==
    /\ pc[a] = "evaluated" /\ res[a] = "deny"
    /\ CommitGuard(a)
    /\ hist' = Append(hist, Entry("denied", expRev[a], Src(ev[a]), Src(ev[a])))
    /\ pc' = [pc EXCEPT ![a] = "done"]
    /\ UNCHANGED <<lifecycle, state, rev, observed, appends, ev, expRev, res>>

\* A stale commit or denial is refused and writes nothing (PRD.md:730).
WriteRefused(a) ==
    /\ \/ (pc[a] = "read" /\ ~Checked(ev[a]))
       \/ (pc[a] = "evaluated")
    /\ ~CommitGuard(a)
    /\ pc' = [pc EXCEPT ![a] = "done"]
    /\ UNCHANGED <<lifecycle, state, rev, observed, hist, appends, ev, expRev, res>>

\* The end state: every attempt is over. Nothing changes.
Done ==
    /\ \A a \in Attempts : pc[a] = "done"
    /\ UNCHANGED vars

Next ==
    \/ Observe
    \/ AppendContext
    \/ Terminate
    \/ \E a \in Attempts :
        \/ LoadOk(a)
        \/ LoadRejected(a)
        \/ CommitFree(a)
        \/ Snapshot(a)
        \/ SnapshotRejected(a)
        \/ ProviderAllows(a)
        \/ ProviderDenies(a)
        \/ CommitAllowed(a)
        \/ RecordDenial(a)
        \/ WriteRefused(a)
    \/ Done

Spec == Init /\ [][Next]_vars

(******************************* the rules *********************************)
TypeOK ==
    /\ lifecycle \in {"Active", "Final", "Terminated"}
    /\ state \in {"A", "B", "F"}
    /\ rev \in Nat
    /\ observed \in Int
    /\ appends \in 0..MaxAppends
    /\ pc \in [Attempts -> {"new", "read", "snapped", "evaluated", "done"}]
    /\ ev \in [Attempts -> Events]
    /\ expRev \in [Attempts -> Nat]
    /\ res \in [Attempts -> {"none", "allow", "deny"}]
    /\ \A i \in 1..Len(hist) :
        /\ hist[i].kind \in {"transition", "denied", "terminated", "context"}
        /\ hist[i].obs \in BOOLEAN

\* PRD.md:726-728. When overlapping attempts compete against the same
\* pre-mutation run state, at most one may commit. A transition and a
\* termination both advance control_revision, so no two advancing entries
\* may assert the same revision.
AtMostOneCommitPerRevision ==
    \A i, j \in 1..Len(hist) :
        (i /= j /\ IsAdv(hist[i]) /\ IsAdv(hist[j])) => hist[i].rev /= hist[j].rev

\* PRD.md:730. A stale checked result, allow or deny, produces no semantic
\* history. Every transition or denial on record asserted exactly the
\* revision and source state that the run had when it was written; a
\* termination asserted the revision the run had.
StaleWritesNothing ==
    \A i \in 1..Len(hist) :
        LET before == SubSeq(hist, 1, i - 1)
            e == hist[i]
        IN  e.kind \in {"transition", "denied", "terminated"} =>
                /\ e.rev = Len(SelectSeq(before, IsAdv))
                /\ e.kind /= "terminated" => e.from = Replay(before)

\* PRD.md:500, 504. Final and terminated runs take no further writes: nothing follows a termination or a transition into a final
\* state, and a run is terminated at most once.
NoWriteAfterTerminal ==
    \A i \in 1..Len(hist) :
        (hist[i].kind = "terminated"
            \/ (hist[i].kind = "transition" /\ hist[i].to = "F")) => i = Len(hist)

\* PRD.md:334, 340-341 (invariants 1, 7, 8). The engine owns current state,
\* and a committed transition and its history entry are one atomic write:
\* the stored state, revision and lifecycle always equal what the history
\* says. A denial, a context append or a refused request moves none of them.
StoredStateMatchesHistory ==
    /\ state = Replay(hist)
    /\ rev = Len(SelectSeq(hist, IsAdv))
    /\ lifecycle = IF \E i \in 1..Len(hist) : hist[i].kind = "terminated" THEN "Terminated"
                   ELSE IF state = "F" THEN "Final" ELSE "Active"

\* PRD.md:421, 457, 498. Every mutation needs a current `show` observation: an
\* unobserved request is rejected and creates no semantic history. Every entry
\* on record was written while the run was observed.
WritesNeedObservation ==
    \A i \in 1..Len(hist) : hist[i].obs

=============================================================================
