use std::{hint::black_box, time::Instant};

use rt_piece_picker::{PiecePicker, MAX_BLOCK_SIZE, MAX_IN_PROGRESS_PIECES};

const PIECE_LENGTH: u32 = MAX_BLOCK_SIZE * 4;
const WARMUP_ITERATIONS: usize = 10_000;
const MEASURED_ITERATIONS: usize = 100_000;
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

fn main() {
    println!("probe,piece_count,iterations,total_ns,ns_per_operation,picker_bytes");
    for piece_count in PIECE_COUNTS {
        run_picker_probe(piece_count);
    }
}
