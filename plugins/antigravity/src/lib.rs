//! Google's Antigravity agent in merlin's chat, over the Agent Client
//! Protocol: `agy acp`, from agy 1.5 on.

use merlin_acp::{Acp, Agent};

const AGENT: Agent = Agent {
    id: "antigravity",
    program: "agy",
    args: &["acp"],
};

merlin_plugin::export!(Acp => Acp::new(&[AGENT]));
