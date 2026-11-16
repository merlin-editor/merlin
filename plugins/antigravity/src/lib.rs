//! Google's Antigravity agent in merlin's chat, over the Agent Client
//! Protocol: `agy acp`, from agy 1.5 on.

use merlin_acp::{Acp, Agent, ModeLook};
use merlin_plugin::AgentAsks;

const MODES: &[ModeLook] = &[
    ModeLook {
        id: "default",
        colour: "blue",
        asks: AgentAsks::AsksFirst,
    },
    ModeLook {
        id: "accept-edits",
        colour: "green",
        asks: AgentAsks::Edits,
    },
    ModeLook {
        id: "auto",
        colour: "orange",
        asks: AgentAsks::EditsAndSafeCommands,
    },
    ModeLook {
        id: "plan",
        colour: "purple",
        asks: AgentAsks::ReadsOnly,
    },
];

const AGENT: Agent = Agent {
    id: "antigravity",
    program: "agy",
    args: &["acp"],
    modes: MODES,
};

merlin_plugin::export!(Acp => Acp::new(&[AGENT]));
