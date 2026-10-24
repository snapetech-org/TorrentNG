# rt-engine

TorrentNG's session and transfer engine. See [ENGINE.md](../../docs/ENGINE.md)
for the runtime architecture.

`EngineHandle::{swarm_status, preview_swarm, switch_to_public, restore_private_swarm}`
provide journaled, exclusive Swarm Switch sessions. They retain the original
private identity, validate shared payload proofs, fence competing actors and
file mutations, and verify again on restore. Comparison performs bounded DHT,
peer availability and tracker scrape observations without private DHT announces.
See [Swarm Switch](../../docs/SWARM_SWITCH.md) for API and recovery contracts.
