//! Private command adapters for all 21 MVP application operations (WP1 T122/T129–T132).
//!
//! Each submodule exposes syntax-to-core mapping and exactly one thin adapter per
//! core operation ID. Rendering, route registration, traced multi-operation dispatch,
//! and concrete integration construction belong elsewhere.
//!
//! | Operation ID | Module | Adapter |
//! | --- | --- | --- |
//! | `provider.add` | [`provider`] | [`provider::add`] |
//! | `provider.list` | [`provider`] | [`provider::list_registrations`], [`provider::list_active_run_impact`] |
//! | `provider.check` | [`provider`] | [`provider::check`] |
//! | `provider.update` | [`provider`] | [`provider::update`] |
//! | `provider.rename` | [`provider`] | [`provider::rename`] |
//! | `provider.disable` | [`provider`] | [`provider::disable_authorize`] |
//! | `provider.restore` | [`provider`] | [`provider::restore`] |
//! | `run.create` | [`run`] | [`run::create`] |
//! | `run.list` | [`run`] | [`run::list`] |
//! | `run.show` | [`run`] | [`run::show`] |
//! | `run.graph` | [`run`] | [`run::graph`] |
//! | `run.history` | [`run`] | [`run::history`] |
//! | `run.annotate` | [`run`] | [`run::annotate`] |
//! | `run.label` | [`run`] | [`run::label`] |
//! | `run.request` | [`run`] | [`run::request`] |
//! | `run.guidance` | [`run`] | [`run::guidance`] |
//! | `run.compatibility` | [`run`] | [`run::compatibility`] |
//! | `run.terminate` | [`run`] | [`run::terminate`] |
//! | `run.export` | [`export`] | [`export::execute`] |
//! | `run.evidence.add` | [`evidence`] | [`evidence::add`] |
//! | `run.evidence.list` | [`evidence`] | [`evidence::list`] |

pub mod evidence;
pub mod export;
pub mod provider;
pub mod run;
