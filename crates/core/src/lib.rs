//! Symbiote core: the `.symbiote/` control plane, Tier 0 engine, and Tier 1 client.
//! No UI code lives here; the GPUI app is a socket client of `daemon`.

pub mod daemon;
pub mod llm;
pub mod model;
pub mod shell;
pub mod srs;
pub mod store;
pub mod track;
pub mod util;
pub mod verify;
