//! Production driver operation registry.
//!
//! Driver metadata (`--help`, `--version`, `--list-operations`) is not an
//! application operation. This registry is the production source for both
//! dispatch exposure and operation-list rendering.

/// Production operation exposed by the CLI driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriverOperation {
    pub id: &'static str,
    pub argv: &'static str,
}

/// Operations the production driver can dispatch, in stable catalog order.
pub const DRIVER_OPERATIONS: &[DriverOperation] = &[
    DriverOperation {
        id: "provider.add",
        argv: "provider add <HANDLE> --exec <PATH> --working-directory <PATH> [--arg <VALUE> ...] [--timeout <SECONDS>]",
    },
    DriverOperation {
        id: "provider.list",
        argv: "provider list [--enabled] [--tombstoned] [--active-runs-for <REGISTRATION-ID>] [--cursor <CURSOR>] [--limit <COUNT>]",
    },
    DriverOperation {
        id: "provider.check",
        argv: "provider check <TARGET> [--active-runs] [--cursor <CURSOR>] [--limit <COUNT>]",
    },
    DriverOperation {
        id: "provider.update",
        argv: "provider update <TARGET> --exec <PATH> [--arg <VALUE> ...] [--working-directory <PATH>] [--timeout <SECONDS>]",
    },
    DriverOperation {
        id: "provider.rename",
        argv: "provider rename <TARGET> <NEW-HANDLE>",
    },
    DriverOperation {
        id: "provider.disable",
        argv: "provider disable <TARGET> [--warning-cursor <CURSOR>] [--limit <COUNT>] [--allow-active-runs <ACK-TOKEN>]",
    },
    DriverOperation {
        id: "provider.restore",
        argv: "provider restore <REGISTRATION-ID> --handle <HANDLE> --exec <PATH> --working-directory <PATH> [--arg <VALUE> ...] [--timeout <SECONDS>]",
    },
    DriverOperation {
        id: "run.create",
        argv: "run create <TARGET> [--label <LABEL>] [--inputs <PATH>]",
    },
    DriverOperation {
        id: "run.list",
        argv: "run list [--terminal] [--all] [--cursor <CURSOR>] [--limit <COUNT>]",
    },
    DriverOperation {
        id: "run.show",
        argv: "run show <RUN-ID>",
    },
    DriverOperation {
        id: "run.graph",
        argv: "run graph <RUN-ID>",
    },
    DriverOperation {
        id: "run.history",
        argv: "run history <RUN-ID> [--cursor <CURSOR>] [--limit <COUNT>]",
    },
    DriverOperation {
        id: "run.evidence.add",
        argv: "run evidence add <RUN-ID> --kind <KIND> --ref <LOCATOR> [--digest <DIGEST>] [--media-type <TYPE>] [--metadata <PATH>]",
    },
    DriverOperation {
        id: "run.evidence.list",
        argv: "run evidence list <RUN-ID> [--cursor <CURSOR>] [--limit <COUNT>]",
    },
    DriverOperation {
        id: "run.annotate",
        argv: "run annotate <RUN-ID> [--note <TEXT>] [--actor <PATH>] [--corrects <SEQUENCE>]",
    },
    DriverOperation {
        id: "run.label",
        argv: "run label <RUN-ID> [--set <LABEL> | --clear]",
    },
    DriverOperation {
        id: "run.request",
        argv: "run request <RUN-ID> <EVENT> [--evidence-id <ID> ...] [--evidence <PATH>] [--note <TEXT>]",
    },
    DriverOperation {
        id: "run.guidance",
        argv: "run guidance <RUN-ID> [--evidence-id <ID> ...]",
    },
    DriverOperation {
        id: "run.compatibility",
        argv: "run compatibility <RUN-ID>",
    },
    DriverOperation {
        id: "run.terminate",
        argv: "run terminate <RUN-ID> [--note <TEXT>]",
    },
    DriverOperation {
        id: "run.export",
        argv: "run export <RUN-ID> --output <DIR>",
    },
];
