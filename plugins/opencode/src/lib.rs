//! opencode in merlin's chat, over the Agent Client Protocol.

use merlin_acp::{Acp, Agent};

const AGENT: Agent = Agent {
    id: "opencode",
    program: "opencode",
    args: &["acp"],
};

merlin_plugin::export!(Acp => Acp::new(&[AGENT]));
