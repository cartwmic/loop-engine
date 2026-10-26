//! lean-change core: the workflow, the model and the gates. No process, file or git access lives here;
//! the binary supplies repository identity through the TreeIdentity port.
pub mod evaluate;
pub mod model;
pub mod protocol;
pub mod workflow;
