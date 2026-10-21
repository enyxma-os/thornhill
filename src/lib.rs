pub mod actuator;
pub mod brain;
pub mod bus;
pub mod graph;
pub mod protocol;
pub mod reflex;
pub mod sensory;

pub use actuator::{ladder_check, ChipRegistry};
pub use brain::Brain;
pub use bus::{BusClient, BusServer};
pub use graph::GraphStore;
pub use protocol::{CanonicalEvent, SubFilter, WireRequest, WireResponse};
pub use reflex::{is_escalation_candidate, ReflexEngine};
pub use sensory::hyprland::{discover_socket_path, HyprlandSensory};
