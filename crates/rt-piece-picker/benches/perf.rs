use std::{hint::black_box, time::Instant};

use rt_piece_picker::{Availability, PiecePicker, MAX_BLOCK_SIZE, MAX_IN_PROGRESS_PIECES};

const PIECE_LENGTH: u32 = MAX_BLOCK_SIZE * 4;
const WARMUP_ITERATIONS: usize = 10_000;
const MEASURED_ITERATIONS: usize = 100_000;
const BITFIELD_MEASURED_ITERATIONS: usize = 20;
const PIECE_COUNTS: [usize; 3] = [1_000, 12_000, 100_000];

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

fn main() {
    println!("probe,piece_count,iterations,total_ns,ns_per_operation,picker_bytes");
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
}
