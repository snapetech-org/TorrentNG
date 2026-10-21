use bytes::BytesMut;
use rt_peer_wire::{PeerCodec,Message};
use rt_metrics::{ResourceGovernor,ResourceGovernorConfig};
use tokio_util::codec::{Decoder,Encoder};
fn main(){
 let g=ResourceGovernor::new(ResourceGovernorConfig::default());
 let mut codec=PeerCodec::with_resources(g.clone());
 let mut wire=BytesMut::new();
 PeerCodec::default().encode(Message::Extended{ext_id:7,payload:vec![0x5a;512*1024]},&mut wire).unwrap();
 let mut input=BytesMut::from(&wire[..4]);
 assert!(codec.decode(&mut input).unwrap().is_none());
 input.extend_from_slice(&wire[4..]);
 let message=codec.decode(&mut input).unwrap().unwrap();
 drop(message); drop(wire);
 assert!(codec.decode(&mut input).unwrap().is_none());
 println!("after_message_drop_and_trim: read_capacity={} governor_used={}",input.capacity(),g.snapshot().total_used_bytes);
 for _ in 0..100 { assert!(codec.decode(&mut input).unwrap().is_none()); }
 println!("after_100_empty_decodes: read_capacity={} governor_used={}",input.capacity(),g.snapshot().total_used_bytes);
 drop(codec);drop(input);
 println!("after_connection_drop: governor_used={}",g.snapshot().total_used_bytes);
}
