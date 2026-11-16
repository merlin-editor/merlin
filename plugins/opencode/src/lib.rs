//! opencode in merlin's chat, over the Agent Client Protocol.

use merlin_acp::{Acp, Agent, ModeLook};
use merlin_plugin::AgentAsks;

/// opencode's own colours: build blue, plan purple. Your own agents get
/// merlin's guess from their names.
const MODES: &[ModeLook] = &[
    ModeLook {
        id: "build",
        colour: "blue",
        asks: AgentAsks::Unsaid,
    },
    ModeLook {
        id: "plan",
        colour: "purple",
        asks: AgentAsks::ReadsOnly,
    },
];

const AGENT: Agent = Agent {
    id: "opencode",
    program: "opencode",
    args: &["acp"],
    modes: MODES,
};

merlin_plugin::export!(Acp => Acp::new(&[AGENT]));
