use std::collections::VecDeque;

use crate::error::UtpError;

pub const MAX_WINDOW_PACKETS: usize = 128;
pub const MAX_IN_FLIGHT_BYTES: u32 = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SentPacket {
    sequence: u16,
    payload_bytes: u32,
}

/// Bounded metadata for sent packets that have not been acknowledged yet.
/// Payload ownership stays with the stream's retransmit queue; this window
/// owns only sequence and byte accounting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendWindow {
    packets: VecDeque<u16>,
    slots: Vec<Option<SentPacket>>,
    bytes_in_flight: u32,
}

impl Default for SendWindow {
    fn default() -> Self {
        Self {
            packets: VecDeque::new(),
            slots: vec![None; MAX_WINDOW_PACKETS],
            bytes_in_flight: 0,
        }
    }
}

impl SendWindow {
    pub fn packet_count(&self) -> usize {
        self.packets.len()
    }

    pub fn oldest_sequence(&self) -> Option<u16> {
        self.packets.front().copied()
    }

    pub fn bytes_in_flight(&self) -> u32 {
        self.bytes_in_flight
    }

    pub fn is_empty(&self) -> bool {
        self.packets.is_empty()
    }

    pub fn contains(&self, sequence: u16) -> bool {
        self.slots[usize::from(sequence) % self.slots.len()]
            .is_some_and(|packet| packet.sequence == sequence)
    }

    pub fn can_record(&self, payload_bytes: u32) -> bool {
        self.packets.len() < MAX_WINDOW_PACKETS
            && self.bytes_in_flight.saturating_add(payload_bytes) <= MAX_IN_FLIGHT_BYTES
    }

    pub fn can_record_sequence(&self, sequence: u16, payload_bytes: u32) -> bool {
        self.can_record(payload_bytes)
            && self.oldest_sequence().is_none_or(|oldest| {
                usize::from(sequence.wrapping_sub(oldest)) < MAX_WINDOW_PACKETS
            })
            && !self.contains(sequence)
    }

    pub fn record(&mut self, sequence: u16, payload_bytes: u32) -> Result<(), UtpError> {
        if !self.can_record_sequence(sequence, payload_bytes) {
            return Err(UtpError::SendWindowFull {
                packets: self.packets.len(),
                bytes: self.bytes_in_flight,
            });
        }
        let index = usize::from(sequence) % self.slots.len();
        self.packets.push_back(sequence);
        self.slots[index] = Some(SentPacket {
            sequence,
            payload_bytes,
        });
        self.bytes_in_flight += payload_bytes;
        Ok(())
    }

    /// Remove packets covered by a cumulative ACK or SACK and return the
    /// payload bytes newly acknowledged by this packet.
    pub fn acknowledge(&mut self, ack_nr: u16, selective_ack: Option<&[u8]>) -> u32 {
        let oldest = self.packets.front().copied();
        let cumulative_is_fresh = oldest.is_none_or(|oldest| !sequence_before(ack_nr, oldest));
        let mut newly_acked_bytes = 0u32;

        let packet_count = self.packets.len();
        for _ in 0..packet_count {
            let Some(sequence) = self.packets.pop_front() else {
                break;
            };
            let index = usize::from(sequence) % self.slots.len();
            let Some(packet) = self.slots[index] else {
                continue;
            };
            if packet.sequence != sequence {
                continue;
            }
            let cumulatively_acked =
                cumulative_is_fresh && (sequence == ack_nr || sequence_before(sequence, ack_nr));
            let offset = sequence.wrapping_sub(ack_nr);
            let selectively_acked = offset >= 2
                && selective_ack.is_some_and(|bits| {
                    let bit_index = usize::from(offset - 2);
                    bits.get(bit_index / 8)
                        .is_some_and(|byte| byte & (1 << (bit_index % 8)) != 0)
                });
            if cumulatively_acked || selectively_acked {
                newly_acked_bytes = newly_acked_bytes.saturating_add(packet.payload_bytes);
                self.bytes_in_flight = self.bytes_in_flight.saturating_sub(packet.payload_bytes);
                self.slots[index] = None;
            } else {
                self.packets.push_back(sequence);
            }
        }
        newly_acked_bytes
    }
}

/// Sequence comparison valid while the active window stays below half the
/// u16 sequence space, as enforced by `MAX_WINDOW_PACKETS`.
pub fn sequence_before(a: u16, b: u16) -> bool {
    a != b && a.wrapping_sub(b) > 0x8000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cumulative_ack_releases_only_the_covered_packet_bytes() {
        let mut window = SendWindow::default();
        window.record(10, 100).unwrap();
        window.record(11, 200).unwrap();
        window.record(12, 300).unwrap();

        assert_eq!(window.acknowledge(10, None), 100);
        assert_eq!(window.bytes_in_flight(), 500);
        assert!(window.contains(11));
        assert!(!window.contains(10));
    }

    #[test]
    fn selective_ack_releases_a_hole_without_releasing_other_packets() {
        let mut window = SendWindow::default();
        window.record(20, 100).unwrap();
        window.record(21, 200).unwrap();
        window.record(22, 300).unwrap();
        window.record(23, 400).unwrap();

        // ack_nr 20 plus bit 0 acknowledges sequence 22; 21 remains a hole.
        assert_eq!(window.acknowledge(20, Some(&[0b0000_0001])), 300);
        assert_eq!(window.bytes_in_flight(), 700);
        assert!(window.contains(21));
        assert!(!window.contains(22));
        assert!(window.contains(23));
    }

    #[test]
    fn stale_cumulative_ack_can_still_carry_a_useful_sack() {
        let mut window = SendWindow::default();
        window.record(30, 100).unwrap();
        window.record(31, 200).unwrap();
        window.record(32, 300).unwrap();

        assert_eq!(window.acknowledge(29, Some(&[0b0000_0010])), 300);
        assert_eq!(window.bytes_in_flight(), 300);
        assert!(window.contains(30));
        assert!(window.contains(31));
    }

    #[test]
    fn window_enforces_packet_and_byte_budgets() {
        let mut window = SendWindow::default();
        assert!(window.record(1, MAX_IN_FLIGHT_BYTES).is_ok());
        assert!(!window.can_record(1));
        assert!(matches!(
            window.record(2, 1),
            Err(UtpError::SendWindowFull { .. })
        ));

        let mut packet_limited = SendWindow::default();
        for sequence in 0..MAX_WINDOW_PACKETS as u16 {
            packet_limited.record(sequence, 0).unwrap();
        }
        assert!(!packet_limited.can_record(0));
    }

    #[test]
    fn send_window_supports_sequence_wraparound() {
        let mut window = SendWindow::default();
        window.record(u16::MAX, 100).unwrap();
        window.record(0, 200).unwrap();
        window.record(1, 300).unwrap();

        assert_eq!(window.acknowledge(u16::MAX, None), 100);
        assert!(window.contains(0));
        assert_eq!(window.acknowledge(0, None), 200);
        assert!(window.contains(1));
    }
}
