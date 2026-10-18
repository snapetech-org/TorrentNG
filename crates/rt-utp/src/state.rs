use crate::{
    congestion::{DelaySample, UtpCongestionController},
    error::UtpError,
    packet::{PacketType, UtpExtension, UtpHeader, UtpPacket},
    selective_ack::SelectiveAck,
    window::{SendWindow, MAX_WINDOW_PACKETS},
};
pub use crate::window::sequence_before;

pub const DEFAULT_INITIAL_WINDOW_BYTES: u32 = 64 * 1024;
pub const DEFAULT_MTU_PAYLOAD_BYTES: usize = 1_352;
pub const DEFAULT_RETRANSMIT_TIMEOUT_US: u64 = 500_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointRole {
    Initiator,
    Acceptor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionIds {
    pub send: u16,
    pub recv: u16,
}

impl ConnectionIds {
    pub fn new_syn(connection_id: u16) -> Self {
        Self {
            send: connection_id,
            recv: connection_id.wrapping_add(1),
        }
    }

    pub fn from_syn_for_acceptor(connection_id: u16) -> Self {
        Self {
            send: connection_id.wrapping_add(1),
            recv: connection_id,
        }
    }

    pub fn for_role(role: EndpointRole, syn_connection_id: u16) -> Self {
        match role {
            EndpointRole::Initiator => Self::new_syn(syn_connection_id),
            EndpointRole::Acceptor => Self::from_syn_for_acceptor(syn_connection_id),
        }
    }

    pub fn expected_for(&self, packet_type: PacketType) -> u16 {
        match packet_type {
            PacketType::Syn => self.recv.wrapping_sub(1),
            PacketType::State | PacketType::Data | PacketType::Fin | PacketType::Reset => self.recv,
        }
    }

    pub fn validate_inbound(&self, header: &UtpHeader) -> Result<(), UtpError> {
        let expected = self.expected_for(header.packet_type);
        if header.connection_id == expected {
            Ok(())
        } else {
            Err(UtpError::ConnectionIdMismatch {
                expected,
                actual: header.connection_id,
            })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    SynSent,
    SynReceived,
    Connected,
    FinSent,
    FinReceived,
    Closing,
    Reset,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundAction {
    None,
    SendState,
    DeliverPayload,
    BufferPayload,
    Close,
    Reset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReceiveSequenceAction {
    InOrder,
    Buffered,
    DuplicateOrOutsideWindow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReceiveSequenceWindow {
    next_sequence: u16,
    received: Vec<Option<u16>>,
}

impl ReceiveSequenceWindow {
    fn new(next_sequence: u16) -> Self {
        Self {
            next_sequence,
            received: vec![None; MAX_WINDOW_PACKETS],
        }
    }

    fn reset(&mut self, next_sequence: u16) {
        self.next_sequence = next_sequence;
        self.received.fill(None);
    }

    fn insert(&mut self, sequence: u16) -> ReceiveSequenceAction {
        let distance = sequence.wrapping_sub(self.next_sequence);
        if distance >= 0x8000 {
            return ReceiveSequenceAction::DuplicateOrOutsideWindow;
        }
        let distance = usize::from(distance);
        if distance >= self.received.len() {
            return ReceiveSequenceAction::DuplicateOrOutsideWindow;
        }

        let slot = usize::from(sequence) % self.received.len();
        if self.received[slot] == Some(sequence) {
            return ReceiveSequenceAction::DuplicateOrOutsideWindow;
        }
        if let Some(previous) = self.received[slot] {
            if !sequence_before(previous, self.next_sequence) {
                return ReceiveSequenceAction::DuplicateOrOutsideWindow;
            }
        }
        self.received[slot] = Some(sequence);

        if distance != 0 {
            return ReceiveSequenceAction::Buffered;
        }

        while self.received[usize::from(self.next_sequence) % self.received.len()]
            == Some(self.next_sequence)
        {
            self.next_sequence = self.next_sequence.wrapping_add(1);
        }
        ReceiveSequenceAction::InOrder
    }

    fn rollback(&mut self, sequence: u16, was_in_order: bool) {
        let slot = usize::from(sequence) % self.received.len();
        if self.received[slot] == Some(sequence) {
            self.received[slot] = None;
            if was_in_order {
                self.next_sequence = sequence;
            }
        }
    }

    fn ack_nr(&self) -> u16 {
        self.next_sequence.wrapping_sub(1)
    }

    fn selective_ack(&self) -> SelectiveAck {
        let ack_nr = self.ack_nr();
        let offsets = self
            .received
            .iter()
            .flatten()
            .filter_map(|sequence| {
                let offset = sequence.wrapping_sub(ack_nr);
                (offset >= 2 && offset <= MAX_WINDOW_PACKETS as u16 + 1).then_some(offset)
            })
            .collect();
        SelectiveAck::from_received_offsets(offsets)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UtpConnection {
    state: ConnectionState,
    ids: ConnectionIds,
    seq_nr: u16,
    ack_nr: u16,
    receive_sequences: ReceiveSequenceWindow,
    oldest_unacked: u16,
    newest_sent: u16,
    remote_window_bytes: u32,
    local_window_bytes: u32,
    send_window: SendWindow,
    rtt_us: Option<u32>,
    rtt_var_us: Option<u32>,
    retransmit_timeout_us: u64,
    congestion: UtpCongestionController,
}

impl UtpConnection {
    pub fn connect(connection_id: u16, initial_seq_nr: u16) -> Self {
        Self {
            state: ConnectionState::SynSent,
            ids: ConnectionIds::new_syn(connection_id),
            seq_nr: initial_seq_nr,
            ack_nr: 0,
            receive_sequences: ReceiveSequenceWindow::new(1),
            oldest_unacked: initial_seq_nr,
            newest_sent: initial_seq_nr,
            remote_window_bytes: DEFAULT_INITIAL_WINDOW_BYTES,
            local_window_bytes: DEFAULT_INITIAL_WINDOW_BYTES,
            send_window: SendWindow::default(),
            rtt_us: None,
            rtt_var_us: None,
            retransmit_timeout_us: DEFAULT_RETRANSMIT_TIMEOUT_US,
            congestion: UtpCongestionController::default(),
        }
    }

    pub fn accept(syn: &UtpHeader, initial_seq_nr: u16) -> Result<Self, UtpError> {
        if syn.packet_type != PacketType::Syn {
            return Err(UtpError::InvalidStatePacket {
                state: ConnectionState::Closed,
                packet_type: syn.packet_type,
            });
        }
        Ok(Self {
            state: ConnectionState::SynReceived,
            ids: ConnectionIds::from_syn_for_acceptor(syn.connection_id),
            seq_nr: initial_seq_nr,
            ack_nr: syn.seq_nr,
            receive_sequences: ReceiveSequenceWindow::new(syn.seq_nr.wrapping_add(1)),
            oldest_unacked: initial_seq_nr,
            newest_sent: initial_seq_nr,
            remote_window_bytes: syn.wnd_size,
            local_window_bytes: DEFAULT_INITIAL_WINDOW_BYTES,
            send_window: SendWindow::default(),
            rtt_us: None,
            rtt_var_us: None,
            retransmit_timeout_us: DEFAULT_RETRANSMIT_TIMEOUT_US,
            congestion: UtpCongestionController::default(),
        })
    }

    pub fn state(&self) -> ConnectionState {
        self.state
    }

    pub fn ids(&self) -> ConnectionIds {
        self.ids
    }

    pub fn next_seq_nr(&self) -> u16 {
        self.seq_nr
    }

    pub fn ack_nr(&self) -> u16 {
        self.ack_nr
    }

    pub fn bytes_in_flight(&self) -> u32 {
        self.send_window.bytes_in_flight()
    }

    pub fn retransmit_timeout_us(&self) -> u64 {
        self.retransmit_timeout_us
    }

    pub fn rtt_us(&self) -> Option<u32> {
        self.rtt_us
    }

    pub fn rtt_var_us(&self) -> Option<u32> {
        self.rtt_var_us
    }

    pub fn available_send_window(&self) -> u32 {
        if !self.send_window.can_record_sequence(self.seq_nr, 1) {
            return 0;
        }
        self.remote_window_bytes
            .min(self.congestion.cwnd_bytes())
            .min(crate::window::MAX_IN_FLIGHT_BYTES)
            .saturating_sub(self.send_window.bytes_in_flight())
    }

    pub(crate) fn set_local_window_bytes(&mut self, bytes: u32) {
        self.local_window_bytes = bytes;
    }

    pub fn congestion_window_bytes(&self) -> u32 {
        self.congestion.cwnd_bytes()
    }

    pub fn congestion_base_delay_us(&self) -> Option<u32> {
        self.congestion.base_delay_us()
    }

    pub fn congestion_current_delay_us(&self) -> Option<u32> {
        self.congestion.current_delay_us()
    }

    pub fn is_established(&self) -> bool {
        matches!(
            self.state,
            ConnectionState::Connected | ConnectionState::FinSent | ConnectionState::FinReceived
        )
    }

    pub fn build_syn(&mut self, now_us: u32) -> UtpPacket {
        let packet = UtpPacket {
            header: self.header(PacketType::Syn, self.ids.send, now_us, 0),
            extensions: Vec::new(),
            payload: Vec::new(),
        };
        // SYN consumes one sequence number, just like the BEP 29 connection
        // setup sequence. The first DATA packet therefore uses ISN + 1.
        self.mark_sent(0)
            .expect("an initial SYN always fits in an empty send window");
        packet
    }

    pub fn build_state(&mut self, now_us: u32, timestamp_diff: u32) -> UtpPacket {
        let packet = self.build_state_with_selective_ack(
            now_us,
            timestamp_diff,
            self.receive_sequences.selective_ack(),
        );
        // The handshake STATE carries the acceptor's initial sequence
        // number. Once the connection is established, ACK-only STATE packets
        // do not consume sequence numbers.
        if self.state == ConnectionState::SynReceived {
            self.mark_sent(0)
                .expect("the handshake STATE fits in an empty send window");
        }
        packet
    }

    pub fn build_data(
        &mut self,
        now_us: u32,
        timestamp_diff: u32,
        payload: Vec<u8>,
    ) -> Result<UtpPacket, UtpError> {
        let payload_len = u32::try_from(payload.len()).unwrap_or(u32::MAX);
        if !self
            .send_window
            .can_record_sequence(self.seq_nr, payload_len)
        {
            return Err(UtpError::SendWindowFull {
                packets: self.send_window.packet_count(),
                bytes: self.send_window.bytes_in_flight(),
            });
        }
        let packet = UtpPacket {
            header: self.header(PacketType::Data, self.ids.send, now_us, timestamp_diff),
            extensions: Vec::new(),
            payload,
        };
        self.mark_sent(payload_len)?;
        Ok(packet)
    }

    pub fn build_fin(&mut self, now_us: u32, timestamp_diff: u32) -> UtpPacket {
        let packet = UtpPacket {
            header: self.header(PacketType::Fin, self.ids.send, now_us, timestamp_diff),
            extensions: Vec::new(),
            payload: Vec::new(),
        };
        self.mark_sent(0)
            .expect("FIN fits after the stream has drained its data window");
        self.state = ConnectionState::FinSent;
        packet
    }

    pub fn build_state_with_selective_ack(
        &self,
        now_us: u32,
        timestamp_diff: u32,
        selective_ack: SelectiveAck,
    ) -> UtpPacket {
        let extensions = if selective_ack.as_bytes().is_empty() {
            Vec::new()
        } else {
            vec![UtpExtension {
                kind: SelectiveAck::EXTENSION_KIND,
                data: selective_ack.into_bytes(),
            }]
        };
        UtpPacket {
            header: self.header(PacketType::State, self.ids.send, now_us, timestamp_diff),
            extensions,
            payload: Vec::new(),
        }
    }

    pub fn on_inbound(
        &mut self,
        packet: &UtpPacket,
        now_us: u32,
    ) -> Result<InboundAction, UtpError> {
        self.ids.validate_inbound(&packet.header)?;
        // RESET is a terminal control packet. Its ACK number is not part of
        // the data acknowledgement stream, so an old or unrelated ACK must
        // not prevent the connection from entering Reset.
        if packet.header.packet_type == PacketType::Reset {
            self.state = ConnectionState::Reset;
            return Ok(InboundAction::Reset);
        }
        self.remote_window_bytes = packet.header.wnd_size;
        let bytes_acked = self.apply_ack(packet.header.ack_nr, &packet.extensions)?;
        if packet.header.timestamp_diff > 0 {
            self.update_rtt(packet.header.timestamp_diff);
            self.congestion.on_ack(
                now_us,
                DelaySample {
                    timestamp_diff_us: packet.header.timestamp_diff,
                    // The payload belongs to the inbound packet and says
                    // nothing about how many of our bytes its ACK covered.
                    // ACK-only STATE packets are the normal case, so use the
                    // send-side flight that the ACK actually advanced instead.
                    bytes_acked,
                },
            );
        }

        match packet.header.packet_type {
            PacketType::Syn => Err(UtpError::InvalidStatePacket {
                state: self.state,
                packet_type: packet.header.packet_type,
            }),
            PacketType::State => {
                if self.state == ConnectionState::SynSent {
                    self.ack_nr = packet.header.seq_nr;
                    self.receive_sequences
                        .reset(packet.header.seq_nr.wrapping_add(1));
                    self.state = ConnectionState::Connected;
                } else if self.state == ConnectionState::FinSent {
                    self.state = ConnectionState::Closing;
                }
                Ok(InboundAction::None)
            }
            PacketType::Data => {
                let action = match self.receive_sequences.insert(packet.header.seq_nr) {
                    ReceiveSequenceAction::InOrder => {
                        self.ack_nr = self.receive_sequences.ack_nr();
                        InboundAction::DeliverPayload
                    }
                    ReceiveSequenceAction::Buffered => InboundAction::BufferPayload,
                    ReceiveSequenceAction::DuplicateOrOutsideWindow => InboundAction::SendState,
                };
                if matches!(self.state, ConnectionState::SynReceived) {
                    self.state = ConnectionState::Connected;
                }
                Ok(action)
            }
            PacketType::Fin => {
                if self.receive_sequences.insert(packet.header.seq_nr)
                    != ReceiveSequenceAction::InOrder
                {
                    return Ok(InboundAction::SendState);
                }
                self.ack_nr = self.receive_sequences.ack_nr();
                self.state = if self.state == ConnectionState::FinSent {
                    ConnectionState::Closing
                } else {
                    ConnectionState::FinReceived
                };
                Ok(InboundAction::Close)
            }
            PacketType::Reset => unreachable!("RESET is handled before ACK validation"),
        }
    }

    pub fn mark_closed(&mut self) {
        self.state = ConnectionState::Closed;
    }

    pub fn on_timeout(&mut self) {
        self.congestion.on_timeout();
    }

    pub(crate) fn all_sent_packets_acked(&self) -> bool {
        self.send_window.is_empty()
    }

    pub(crate) fn packet_is_unacknowledged(&self, sequence: u16) -> bool {
        self.send_window.contains(sequence)
    }

    pub(crate) fn reject_inbound_sequence(&mut self, sequence: u16, was_in_order: bool) {
        self.receive_sequences.rollback(sequence, was_in_order);
        self.ack_nr = self.receive_sequences.ack_nr();
    }

    fn header(
        &self,
        packet_type: PacketType,
        connection_id: u16,
        now_us: u32,
        timestamp_diff: u32,
    ) -> UtpHeader {
        UtpHeader {
            packet_type,
            version: 1,
            extension: 0,
            connection_id,
            timestamp_us: now_us,
            timestamp_diff,
            wnd_size: self.local_window_bytes,
            seq_nr: self.seq_nr,
            ack_nr: self.ack_nr,
        }
    }

    fn mark_sent(&mut self, payload_len: u32) -> Result<(), UtpError> {
        self.send_window.record(self.seq_nr, payload_len)?;
        self.newest_sent = self.seq_nr;
        self.seq_nr = self.seq_nr.wrapping_add(1);
        self.oldest_unacked = self
            .send_window
            .oldest_sequence()
            .unwrap_or(self.seq_nr);
        Ok(())
    }

    fn apply_ack(&mut self, ack_nr: u16, extensions: &[UtpExtension]) -> Result<u32, UtpError> {
        if sequence_before(self.newest_sent, ack_nr) {
            return Err(UtpError::AckOutOfWindow {
                ack_nr,
                oldest_unacked: self.oldest_unacked,
                newest_sent: self.newest_sent,
            });
        }
        let selective_ack = extensions
            .iter()
            .find(|extension| extension.kind == SelectiveAck::EXTENSION_KIND)
            .map(|extension| extension.data.as_slice());
        let bytes_acked = self.send_window.acknowledge(ack_nr, selective_ack);
        self.oldest_unacked = self
            .send_window
            .oldest_sequence()
            .unwrap_or(self.seq_nr);
        Ok(bytes_acked)
    }

    fn update_rtt(&mut self, measured_us: u32) {
        match (self.rtt_us, self.rtt_var_us) {
            (Some(rtt), Some(var)) => {
                let delta = rtt.abs_diff(measured_us);
                let next_var = ((u64::from(var) * 3) + u64::from(delta)) / 4;
                let next_rtt = ((u64::from(rtt) * 7) + u64::from(measured_us)) / 8;
                self.rtt_us = Some(next_rtt.min(u64::from(u32::MAX)) as u32);
                self.rtt_var_us = Some(next_var.min(u64::from(u32::MAX)) as u32);
            }
            _ => {
                self.rtt_us = Some(measured_us);
                self.rtt_var_us = Some(measured_us / 2);
            }
        }
        let rtt = u64::from(self.rtt_us.unwrap_or(DEFAULT_RETRANSMIT_TIMEOUT_US as u32));
        let var = u64::from(self.rtt_var_us.unwrap_or(0));
        self.retransmit_timeout_us = (rtt + 4 * var).max(100_000);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inbound_state(ids: ConnectionIds, seq_nr: u16, ack_nr: u16) -> UtpPacket {
        UtpPacket {
            header: UtpHeader {
                packet_type: PacketType::State,
                version: 1,
                extension: 0,
                connection_id: ids.recv,
                timestamp_us: 20,
                timestamp_diff: 10_000,
                wnd_size: 32_000,
                seq_nr,
                ack_nr,
            },
            extensions: Vec::new(),
            payload: Vec::new(),
        }
    }

    #[test]
    fn connection_ids_follow_bep29_syn_rule() {
        let initiator = ConnectionIds::for_role(EndpointRole::Initiator, 99);
        let acceptor = ConnectionIds::for_role(EndpointRole::Acceptor, 99);
        assert_eq!(
            initiator,
            ConnectionIds {
                send: 99,
                recv: 100
            }
        );
        assert_eq!(
            acceptor,
            ConnectionIds {
                send: 100,
                recv: 99
            }
        );
        assert_eq!(initiator.expected_for(PacketType::State), 100);
        assert_eq!(acceptor.expected_for(PacketType::Syn), 98);
    }

    #[test]
    fn initiator_syn_to_state_establishes_connection() {
        let mut conn = UtpConnection::connect(50, 7);
        let syn = conn.build_syn(1);
        assert_eq!(syn.header.packet_type, PacketType::Syn);
        assert_eq!(syn.header.connection_id, 50);

        let state = inbound_state(conn.ids(), 90, 7);
        assert_eq!(conn.on_inbound(&state, 0).unwrap(), InboundAction::None);
        assert_eq!(conn.state(), ConnectionState::Connected);
        assert_eq!(conn.available_send_window(), 32_000);
        assert!(conn.retransmit_timeout_us() >= 100_000);
    }

    #[test]
    fn acceptor_uses_syn_to_seed_ack_and_connection_ids() {
        let syn = UtpHeader {
            packet_type: PacketType::Syn,
            version: 1,
            extension: 0,
            connection_id: 700,
            timestamp_us: 1,
            timestamp_diff: 0,
            wnd_size: 10_000,
            seq_nr: 44,
            ack_nr: 0,
        };
        let mut conn = UtpConnection::accept(&syn, 9).unwrap();
        let state = conn.build_state(2, 1);
        assert_eq!(
            conn.ids(),
            ConnectionIds {
                send: 701,
                recv: 700
            }
        );
        assert_eq!(state.header.connection_id, 701);
        assert_eq!(state.header.ack_nr, 44);
    }

    #[test]
    fn data_send_advances_sequence_and_tracks_flight() {
        let mut conn = UtpConnection::connect(1, 10);
        let data = conn.build_data(1, 0, vec![1, 2, 3, 4]).unwrap();
        assert_eq!(data.header.seq_nr, 10);
        assert_eq!(conn.next_seq_nr(), 11);
        assert_eq!(conn.bytes_in_flight(), 4);
    }

    #[test]
    fn congestion_ack_uses_outgoing_flight_not_inbound_payload() {
        let mut conn = UtpConnection::connect(1, 10);
        conn.build_syn(1);
        let mut handshake = inbound_state(conn.ids(), 90, 10);
        handshake.header.timestamp_diff = 0;
        conn.on_inbound(&handshake, 0).unwrap();

        let before = conn.congestion_window_bytes();
        let data = conn.build_data(2, 0, vec![0; 4_096]).unwrap();
        let mut ack = inbound_state(conn.ids(), 91, data.header.seq_nr);
        ack.header.timestamp_diff = 20_000;
        // This is an ACK-only STATE packet; its empty payload must not make
        // the congestion controller believe that zero bytes were delivered.
        assert!(ack.payload.is_empty());
        conn.on_inbound(&ack, 0).unwrap();

        assert_eq!(conn.congestion_window_bytes(), before.saturating_add(4_096));
    }

    #[test]
    fn inbound_data_updates_ack_and_delivers_payload() {
        let syn = UtpHeader {
            packet_type: PacketType::Syn,
            version: 1,
            extension: 0,
            connection_id: 3,
            timestamp_us: 0,
            timestamp_diff: 0,
            wnd_size: 1,
            seq_nr: 12,
            ack_nr: 0,
        };
        let mut conn = UtpConnection::accept(&syn, 99).unwrap();
        let packet = UtpPacket {
            header: UtpHeader {
                packet_type: PacketType::Data,
                version: 1,
                extension: 0,
                connection_id: conn.ids().recv,
                timestamp_us: 1,
                timestamp_diff: 0,
                wnd_size: 20,
                seq_nr: 13,
                ack_nr: 0,
            },
            extensions: Vec::new(),
            payload: b"abc".to_vec(),
        };
        assert_eq!(
            conn.on_inbound(&packet, 0).unwrap(),
            InboundAction::DeliverPayload
        );
        assert_eq!(conn.ack_nr(), 13);
        assert_eq!(conn.state(), ConnectionState::Connected);
    }

    #[test]
    fn duplicate_and_out_of_order_data_are_acknowledged_without_delivery() {
        let mut initiator = UtpConnection::connect(50, 10);
        let syn = initiator.build_syn(1);
        let mut acceptor = UtpConnection::accept(&syn.header, 20).unwrap();
        let state = acceptor.build_state(2, 0);
        initiator.on_inbound(&state, 0).unwrap();

        let data = initiator.build_data(3, 0, b"first".to_vec()).unwrap();
        assert_eq!(
            acceptor.on_inbound(&data, 0).unwrap(),
            InboundAction::DeliverPayload
        );
        assert_eq!(
            acceptor.on_inbound(&data, 0).unwrap(),
            InboundAction::SendState
        );

        let mut future = data.clone();
        future.header.seq_nr = future.header.seq_nr.wrapping_add(2);
        assert_eq!(
            acceptor.on_inbound(&future, 0).unwrap(),
            InboundAction::BufferPayload
        );
        assert_eq!(acceptor.ack_nr(), data.header.seq_nr);

        let state = acceptor.build_state(4, 0);
        assert_eq!(state.extensions.len(), 1);
        assert_eq!(state.extensions[0].kind, SelectiveAck::EXTENSION_KIND);
        assert_eq!(state.extensions[0].data, vec![1]);

        let mut missing = data.clone();
        missing.header.seq_nr = missing.header.seq_nr.wrapping_add(1);
        assert_eq!(
            acceptor.on_inbound(&missing, 0).unwrap(),
            InboundAction::DeliverPayload
        );
        assert_eq!(acceptor.ack_nr(), future.header.seq_nr);
    }

    #[test]
    fn cumulative_and_selective_acks_account_only_newly_acked_bytes() {
        let mut conn = UtpConnection::connect(50, 10);
        conn.build_syn(1);
        let mut handshake = inbound_state(conn.ids(), 90, 10);
        handshake.header.timestamp_diff = 0;
        conn.on_inbound(&handshake, 0).unwrap();

        let first = conn.build_data(2, 0, vec![0; 100]).unwrap();
        let second = conn.build_data(2, 0, vec![0; 200]).unwrap();
        let third = conn.build_data(2, 0, vec![0; 300]).unwrap();
        let mut ack = inbound_state(conn.ids(), 91, first.header.seq_nr);
        ack.extensions.push(UtpExtension {
            kind: SelectiveAck::EXTENSION_KIND,
            data: vec![1],
        });

        conn.on_inbound(&ack, 0).unwrap();
        assert_eq!(conn.bytes_in_flight(), 200);
        assert!(!conn.packet_is_unacknowledged(first.header.seq_nr));
        assert!(conn.packet_is_unacknowledged(second.header.seq_nr));
        assert!(!conn.packet_is_unacknowledged(third.header.seq_nr));
    }

    #[test]
    fn receive_sequence_window_wraps_and_advances_through_buffered_data() {
        let syn = UtpHeader {
            packet_type: PacketType::Syn,
            version: 1,
            extension: 0,
            connection_id: 7,
            timestamp_us: 0,
            timestamp_diff: 0,
            wnd_size: DEFAULT_INITIAL_WINDOW_BYTES,
            seq_nr: u16::MAX - 1,
            ack_nr: 0,
        };
        let mut conn = UtpConnection::accept(&syn, 100).unwrap();
        let mut after_wrap = UtpPacket {
            header: UtpHeader {
                packet_type: PacketType::Data,
                version: 1,
                extension: 0,
                connection_id: conn.ids().recv,
                timestamp_us: 1,
                timestamp_diff: 0,
                wnd_size: DEFAULT_INITIAL_WINDOW_BYTES,
                seq_nr: 0,
                ack_nr: 0,
            },
            extensions: Vec::new(),
            payload: vec![2],
        };
        assert_eq!(
            conn.on_inbound(&after_wrap, 0).unwrap(),
            InboundAction::BufferPayload
        );
        after_wrap.header.seq_nr = u16::MAX;
        assert_eq!(
            conn.on_inbound(&after_wrap, 0).unwrap(),
            InboundAction::DeliverPayload
        );
        assert_eq!(conn.ack_nr(), 0);
    }

    #[test]
    fn rejecting_an_over_budget_packet_restores_the_gap_and_keeps_later_sack() {
        let mut window = ReceiveSequenceWindow::new(10);
        assert_eq!(window.insert(10), ReceiveSequenceAction::InOrder);
        assert_eq!(window.insert(12), ReceiveSequenceAction::Buffered);
        assert_eq!(window.insert(11), ReceiveSequenceAction::InOrder);
        assert_eq!(window.ack_nr(), 12);

        window.rollback(11, true);
        assert_eq!(window.ack_nr(), 10);
        assert_eq!(window.selective_ack().as_bytes(), &[1]);
    }

    #[test]
    fn wrong_connection_id_is_rejected() {
        let mut conn = UtpConnection::connect(10, 1);
        let mut state = inbound_state(conn.ids(), 2, 1);
        state.header.connection_id = 999;
        assert!(matches!(
            conn.on_inbound(&state, 0),
            Err(UtpError::ConnectionIdMismatch {
                expected: 11,
                actual: 999
            })
        ));
    }

    #[test]
    fn ack_after_newest_sent_is_rejected() {
        let mut conn = UtpConnection::connect(1, 10);
        let state = inbound_state(conn.ids(), 2, 99);
        assert!(matches!(
            conn.on_inbound(&state, 0),
            Err(UtpError::AckOutOfWindow {
                ack_nr: 99,
                oldest_unacked: 10,
                newest_sent: 10
            })
        ));
    }

    #[test]
    fn reset_terminates_before_ack_window_validation() {
        let mut conn = UtpConnection::connect(1, 10);
        let packet = UtpPacket {
            header: UtpHeader {
                packet_type: PacketType::Reset,
                version: 1,
                extension: 0,
                connection_id: conn.ids().recv,
                timestamp_us: 20,
                timestamp_diff: 10_000,
                wnd_size: 32_000,
                seq_nr: 2,
                ack_nr: 99,
            },
            extensions: Vec::new(),
            payload: Vec::new(),
        };

        assert_eq!(conn.on_inbound(&packet, 0).unwrap(), InboundAction::Reset);
        assert_eq!(conn.state(), ConnectionState::Reset);
    }

    #[test]
    fn zero_ack_is_treated_as_no_ack_before_peer_has_seen_sequence() {
        let mut conn = UtpConnection::connect(1, 10);
        let state = inbound_state(conn.ids(), 2, 0);
        assert_eq!(conn.on_inbound(&state, 0).unwrap(), InboundAction::None);
    }

    #[test]
    fn selective_ack_extension_can_be_attached_to_state_packet() {
        let conn = UtpConnection::connect(20, 5);
        let packet =
            conn.build_state_with_selective_ack(1, 0, SelectiveAck::from_received_offsets(vec![2]));
        assert_eq!(packet.extensions.len(), 1);
        assert_eq!(packet.extensions[0].kind, SelectiveAck::EXTENSION_KIND);
        assert_eq!(packet.extensions[0].data, vec![1]);
    }

    #[test]
    fn sequence_before_handles_wraparound() {
        assert!(sequence_before(65_535, 0));
        assert!(sequence_before(0, 1));
        assert!(!sequence_before(1, 0));
        assert!(!sequence_before(42, 42));
    }
}
