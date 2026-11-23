// Words for the statechart report and chart. The chart is drawn only when the
// model has at most 500 states; on a break the report still prints the path.
// Pure functions of their arguments, as statechart requires.

const attempts = (v) =>
  Object.keys(v.pc)
    .sort()
    .map((a) => `${a} ${v.pc[a]}`)
    .join(", ");

export const title = "Loop Engine run: overlapping event attempts";

export const state = (v) =>
  `${v.lifecycle} at ${v.state}, revision ${v.rev}, ` +
  `${v.observed === v.rev ? "observed" : "not observed"}; ${attempts(v)}`;

const steps = {
  Observe: "the driver runs show",
  AppendContext: "the driver appends context",
  Terminate: "the operator terminates the run",
  LoadOk: "an attempt reads the run and starts",
  LoadRejected: "an attempt is refused before it starts",
  CommitFree: "a check-free event commits",
  Snapshot: "a checked attempt takes its snapshot",
  SnapshotRejected: "a checked attempt is refused at the snapshot",
  ProviderAllows: "the provider allows",
  ProviderDenies: "the provider denies",
  CommitAllowed: "an allowed event commits",
  RecordDenial: "a denial is recorded",
  WriteRefused: "a stale write is refused",
  Done: "every attempt is over",
};

export const step = (action) => steps[action] ?? action;

export const group = (v) => v.lifecycle;

export const colors = { Active: "blue", Final: "teal", Terminated: "orange" };
