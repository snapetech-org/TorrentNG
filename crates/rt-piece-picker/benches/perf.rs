use std::{hint::black_box, time::Instant};

use rt_piece_picker::{
    Availability, BlockRequest, PiecePicker, MAX_BLOCK_SIZE, MAX_IN_PROGRESS_PIECES,
};

const PIECE_LENGTH: u32 = MAX_BLOCK_SIZE * 4;
const WARMUP_ITERATIONS: usize = 10_000;
const MEASURED_ITERATIONS: usize = 100_000;
const BITFIELD_MEASURED_ITERATIONS: usize = 20;
const PIECE_COUNTS: [usize; 3] = [1_000, 12_000, 100_000];
const ENDGAME_PEER_COUNTS: [usize; 4] = [1, 4, 16, 64];
const ENDGAME_BLOCKS: usize = 128;
const ENDGAME_DUPLICATES_PER_PEER: usize = 32;
const ENDGAME_REFILL_ITERATIONS: usize = 5_000;

fn all_pieces_bitfield(piece_count: usize) -> Vec<u8> {
    let mut bitfield = vec![0xff; piece_count.div_ceil(8)];
    let tail_bits = piece_count % 8;
    if tail_bits != 0 {
        if let Some(last) = bitfield.last_mut() {
            *last = 0xff << (8 - tail_bits);
        }
    }
    bitfield
}

fn sparse_pieces_bitfield(piece_count: usize, stride: usize) -> Vec<u8> {
    let mut bitfield = vec![0_u8; piece_count.div_ceil(8)];
    for piece in (0..piece_count).step_by(stride) {
        bitfield[piece / 8] |= 0x80 >> (piece % 8);
    }
    bitfield
}

fn run_picker_probe(piece_count: usize) {
    let mut picker = PiecePicker::new(piece_count, PIECE_LENGTH, PIECE_LENGTH);
    picker
        .availability
        .add_bitfield(&all_pieces_bitfield(piece_count));

    // Prebuild partial states up to the picker's admission cap so the timed
    // loop measures selection/cancellation, not first-use state allocation.
    let admitted_piece_count = piece_count.min(MAX_IN_PROGRESS_PIECES);
    for piece in 0..admitted_piece_count {
        picker.restore_partial_piece(piece, &[0]);
    }
    let mut peer_has = vec![false; piece_count];
    peer_has[..admitted_piece_count].fill(true);

    for _ in 0..WARMUP_ITERATIONS {
        let request = picker
            .pick(&peer_has)
            .expect("fresh pieces remain selectable");
        picker.cancel_request(request.piece as usize, request.begin);
    }

    let picker_bytes = picker.memory_bytes();
    let started = Instant::now();
    for _ in 0..MEASURED_ITERATIONS {
        let request = black_box(
            picker
                .pick(&peer_has)
                .expect("fresh pieces remain selectable"),
        );
        picker.cancel_request(request.piece as usize, request.begin);
    }
    let elapsed_ns = started.elapsed().as_nanos();
    let ns_per_cycle = elapsed_ns / MEASURED_ITERATIONS as u128;
    println!(
        "picker_pick_cancel,{piece_count},{MEASURED_ITERATIONS},{elapsed_ns},{ns_per_cycle},{picker_bytes}"
    );
}

fn run_bitfield_update_probe(piece_count: usize, distribution: &str, bitfield: &[u8]) {
    let mut availability = Availability::new(piece_count);
    let active_piece_count = piece_count - 2;
    availability.add_have(active_piece_count);
    availability.add_have(active_piece_count + 1);
    availability.add_have(active_piece_count + 1);
    availability.add_bitfield(bitfield);
    for _ in 0..3 {
        availability.add_bitfield(bitfield);
        availability.remove_bitfield(bitfield);
    }

    let availability_bytes = availability.memory_bytes();
    let started = Instant::now();
    for _ in 0..BITFIELD_MEASURED_ITERATIONS {
        availability.add_bitfield(black_box(bitfield));
        availability.remove_bitfield(black_box(bitfield));
    }
    let elapsed_ns = started.elapsed().as_nanos();
    let ns_per_cycle = elapsed_ns / BITFIELD_MEASURED_ITERATIONS as u128;
    println!(
        "availability_add_remove_{distribution},{piece_count},{BITFIELD_MEASURED_ITERATIONS},{elapsed_ns},{ns_per_cycle},{availability_bytes}"
    );
}

fn endgame_peer_requests(peer_count: usize) -> (PiecePicker, Vec<bool>, Vec<Vec<BlockRequest>>) {
    let piece_length = MAX_BLOCK_SIZE * ENDGAME_BLOCKS as u32;
    let mut picker = PiecePicker::new(1, piece_length, piece_length);
    picker.availability.add_have(0);
    let peer_has = vec![true];

    let mut peers = (0..peer_count)
        .map(|_| Vec::with_capacity(ENDGAME_BLOCKS / peer_count + ENDGAME_DUPLICATES_PER_PEER))
        .collect::<Vec<_>>();
    for _ in 0..ENDGAME_BLOCKS {
        let request = picker
            .pick(&peer_has)
            .expect("initial peers fill the endgame piece");
        peers[request.begin as usize / MAX_BLOCK_SIZE as usize % peer_count].push(request);
    }

    if peer_count > 1 {
        for requests in &mut peers {
            for _ in 0..ENDGAME_DUPLICATES_PER_PEER {
                let request = picker
                    .pick_endgame(&peer_has, requests)
                    .expect("every peer has bounded duplicate work");
                requests.push(request);
            }
        }
    }
    assert!(picker.endgame_active());
    (picker, peer_has, peers)
}

fn run_endgame_refill_probe(peer_count: usize) {
    let (picker, peer_has, peer_requests) = endgame_peer_requests(peer_count);
    let own_requests = &peer_requests[0];
    let legacy = || {
        let mut candidates = peer_requests
            .iter()
            .skip(1)
            .flat_map(|requests| requests.iter().copied())
            .collect::<Vec<_>>();
        candidates.sort_unstable_by_key(|request| (request.piece, request.begin));
        candidates.dedup_by_key(|request| (request.piece, request.begin));
        let candidate_bytes = candidates.capacity() * std::mem::size_of::<BlockRequest>();
        let useful = picker.endgame_duplicate_work_for_peer(
            &peer_has,
            own_requests,
            &candidates,
            ENDGAME_BLOCKS,
        );
        (useful, candidate_bytes)
    };
    let indexed = || {
        if peer_count == 1 {
            0
        } else {
            picker.useful_endgame_duplicate_work_for_peer(&peer_has, own_requests, ENDGAME_BLOCKS)
        }
    };

    assert_eq!(legacy().0, indexed());
    for _ in 0..WARMUP_ITERATIONS.min(1_000) {
        black_box(legacy());
        black_box(indexed());
    }

    for (name, use_index) in [
        ("endgame_refill_collect_sort", false),
        ("endgame_refill_picker_index", true),
    ] {
        let started = Instant::now();
        let mut reported_bytes = 0usize;
        for _ in 0..ENDGAME_REFILL_ITERATIONS {
            if use_index {
                black_box(indexed());
            } else {
                let (useful, bytes) = legacy();
                reported_bytes = reported_bytes.max(bytes);
                black_box(useful);
            }
        }
        let elapsed_ns = started.elapsed().as_nanos();
        let ns_per_cycle = elapsed_ns / ENDGAME_REFILL_ITERATIONS as u128;
        println!(
            "{name},{peer_count},{ENDGAME_REFILL_ITERATIONS},{elapsed_ns},{ns_per_cycle},{reported_bytes}"
        );
    }
}

fn main() {
    println!("probe,scale,iterations,total_ns,ns_per_operation,reported_memory_bytes");
    for piece_count in PIECE_COUNTS {
        run_picker_probe(piece_count);
        let empty = vec![0_u8; piece_count.div_ceil(8)];
        let active_piece_count = piece_count - 2;
        let sparse = sparse_pieces_bitfield(active_piece_count, 64);
        let dense = all_pieces_bitfield(active_piece_count);
        run_bitfield_update_probe(piece_count, "empty", &empty);
        run_bitfield_update_probe(piece_count, "sparse_1_in_64", &sparse);
        run_bitfield_update_probe(piece_count, "dense", &dense);
    }
    for peer_count in ENDGAME_PEER_COUNTS {
        run_endgame_refill_probe(peer_count);
    }
}
