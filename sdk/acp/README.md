# merlin-acp

[Agent Client Protocol](https://agentclientprotocol.com) (ACP) support for
[merlin](https://github.com/merlin-editor/merlin) agent plugins.

An agent plugin names the program and how to start it; this crate speaks
ACP to it and turns what it says into merlin's chat: messages, thoughts,
its plan, its steps, permission cards, file reads and edits through the
editor, and commands in terminal tabs.

```toml
[dependencies]
merlin-plugin = "0.15.0"
merlin-acp = "0.15.0"
```

```rust
use merlin_acp::{Acp, Agent};

const OPENCODE: Agent = Agent { id: "opencode", program: "opencode", args: &["acp"] };

merlin_plugin::export!(Acp => Acp::new(&[OPENCODE]));
```

The manifest declares the agent in `[[agents]]`, the program in
`permissions.processes` and `[programs]`, and `permissions.agent = true`.
