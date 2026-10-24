//! Bounded upload preparation and socket progress, separate from peer lifecycle
//! and download/control operations. A batch retains its backing through settlement.

use super::*;

pub(super) async fn send_upload_batch(
    io: &mut PeerIo,
    batch: &UploadBatchCompletion,
    ready: &[usize],
    cursor: &mut UploadWriteCursor,
) -> Result<usize, (usize, anyhow::Error)> {
    #[cfg(all(target_os = "linux", feature = "seed-mmap"))]
    if let Ok(UploadPayloadBatch::Mapped { data, .. }) = &batch.result {
        let PeerIo::Tcp(framed) = io else {
            return Err((0, anyhow::anyhow!("mapped upload requires plaintext TCP")));
        };
        match timeout(PEER_SOCKET_WRITE_TIMEOUT, framed.flush()).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Err((0, error.into())),
            Err(_) => return Err((0, anyhow::anyhow!("peer socket write timed out"))),
        }
        return write_mapped_upload(framed.get_mut(), data, &batch.requests, ready, cursor).await;
    }
    let frames = ready
        .iter()
        .map(|&index| {
            let request = batch.requests[index];
            (request.wire_piece, request.wire_begin, batch.data(index))
        })
        .collect::<UploadBatch<_>>();
    io.send_pieces(&frames, cursor).await
}

#[cfg(all(target_os = "linux", feature = "seed-mmap"))]
async fn write_mapped_upload(
    socket: &TcpStream,
    data: &rt_storage::mapped_upload::MappedUploadRead,
    requests: &[UploadRequest],
    ready: &[usize],
    cursor: &mut UploadWriteCursor,
) -> Result<usize, (usize, anyhow::Error)> {
    use std::os::fd::AsFd;
    if ready.len() > MAX_UPLOAD_WRITE_FRAMES {
        return Err((0, anyhow::anyhow!("too many mapped upload frames")));
    }
    let mut headers = [[0u8; 13]; MAX_UPLOAD_WRITE_FRAMES];
    for (slot, &index) in ready.iter().enumerate() {
        let request = requests[index];
        headers[slot][..4].copy_from_slice(&(9 + request.length).to_be_bytes());
        headers[slot][4] = 7;
        headers[slot][5..9].copy_from_slice(&request.wire_piece.to_be_bytes());
        headers[slot][9..13].copy_from_slice(&request.wire_begin.to_be_bytes());
    }
    let deadline = TokioInstant::now() + PEER_SOCKET_WRITE_TIMEOUT;
    let mut completed = 0;
    let mut header_offset = 0;
    let mut payload_offset = 0;
    while completed < ready.len() {
        let result = tokio::time::timeout_at(deadline, async {
            loop {
                socket.writable().await?;
                let frames = ready
                    .iter()
                    .enumerate()
                    .skip(completed)
                    .map(|(slot, &index)| {
                        let request = requests[index];
                        let consumed = if slot == completed { payload_offset } else { 0 };
                        let head = if slot == completed { header_offset } else { 0 };
                        (
                            &headers[slot][head..],
                            (request.begin - requests[0].begin) as usize + consumed,
                            request.length as usize - consumed,
                        )
                    })
                    .collect::<UploadBatch<_>>();
                match socket.try_io(tokio::io::Interest::WRITABLE, || {
                    data.try_send_frames(socket.as_fd(), &frames)
                }) {
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => continue,
                    result => return result,
                }
            }
        })
        .await;
        let mut written = match result {
            Ok(Ok(0)) => {
                return Err((
                    completed,
                    std::io::Error::from(std::io::ErrorKind::WriteZero).into(),
                ))
            }
            Ok(Ok(written)) => written,
            Ok(Err(error)) => return Err((completed, error.into())),
            Err(_) => return Err((completed, anyhow::anyhow!("peer socket write timed out"))),
        };
        while written > 0 && completed < ready.len() {
            let head = written.min(13 - header_offset);
            header_offset += head;
            written -= head;
            let payload = written.min(requests[ready[completed]].length as usize - payload_offset);
            payload_offset += payload;
            cursor.record(payload);
            written -= payload;
            if header_offset == 13 && payload_offset == requests[ready[completed]].length as usize {
                completed += 1;
                header_offset = 0;
                payload_offset = 0;
            }
        }
        debug_assert_eq!(written, 0);
    }
    Ok(completed)
}

#[cfg(test)]
pub(super) async fn write_piece_vectored<W>(
    stream: &mut W,
    header: &[u8; 13],
    payload: &[u8],
) -> std::io::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::AsyncWriteExt;

    let mut header_offset = 0;
    let mut payload_offset = 0;
    while header_offset < header.len() || payload_offset < payload.len() {
        let header_slice = &header[header_offset..];
        let payload_slice = &payload[payload_offset..];
        let buffers = [IoSlice::new(header_slice), IoSlice::new(payload_slice)];
        debug!(
            target: "torrentng::upload_batches",
            operation = "socket_write",
            frames = 1,
            payload_remaining = payload_slice.len(),
            vectored_slices = buffers.len(),
            "writing peer Piece frame"
        );
        let written = stream.write_vectored(&buffers).await?;
        if written == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "failed to write peer piece frame",
            ));
        }
        let header_written = written.min(header_slice.len());
        header_offset += header_written;
        payload_offset += written - header_written;
    }
    Ok(())
}

pub(super) async fn write_piece_frames_vectored<W>(
    stream: &mut W,
    pieces: &[(u32, u32, &[u8])],
    cursor: &mut UploadWriteCursor,
) -> Result<usize, (usize, std::io::Error)>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::AsyncWriteExt;

    if pieces.len() > MAX_UPLOAD_WRITE_FRAMES {
        return Err((
            0,
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "too many Piece frames in one upload batch",
            ),
        ));
    }
    let mut headers = [[0u8; 13]; MAX_UPLOAD_WRITE_FRAMES];
    for (index, (piece, begin, data)) in pieces.iter().enumerate() {
        let message_len = 9usize
            .checked_add(data.len())
            .and_then(|len| u32::try_from(len).ok())
            .ok_or_else(|| {
                (
                    0,
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "piece frame length exceeds u32",
                    ),
                )
            })?;
        let header = &mut headers[index];
        header[..4].copy_from_slice(&message_len.to_be_bytes());
        header[4] = 7;
        header[5..9].copy_from_slice(&piece.to_be_bytes());
        header[9..13].copy_from_slice(&begin.to_be_bytes());
    }

    let deadline = TokioInstant::now() + PEER_SOCKET_WRITE_TIMEOUT;
    let mut cursors = [(0usize, 0usize); MAX_UPLOAD_WRITE_FRAMES];
    loop {
        let completed = cursors
            .iter()
            .take(pieces.len())
            .zip(pieces)
            .take_while(|((header, payload), (_, _, data))| *header == 13 && *payload == data.len())
            .count();
        if completed == pieces.len() {
            return Ok(completed);
        }

        let write_result = {
            let mut buffers: [IoSlice<'_>; MAX_UPLOAD_WRITE_FRAMES * 2] =
                std::array::from_fn(|_| IoSlice::new(&[]));
            let mut buffer_count = 0;
            for (index, (_, _, data)) in pieces.iter().enumerate().skip(completed) {
                let (header_offset, payload_offset) = cursors[index];
                if header_offset < headers[index].len() {
                    buffers[buffer_count] = IoSlice::new(&headers[index][header_offset..]);
                    buffer_count += 1;
                }
                if payload_offset < data.len() {
                    buffers[buffer_count] = IoSlice::new(&data[payload_offset..]);
                    buffer_count += 1;
                }
            }
            let payload_remaining = pieces
                .iter()
                .enumerate()
                .skip(completed)
                .map(|(index, (_, _, data))| data.len().saturating_sub(cursors[index].1))
                .sum::<usize>();
            debug!(
                target: "torrentng::upload_batches",
                operation = "socket_write",
                frames = pieces.len() - completed,
                payload_remaining,
                vectored_slices = buffer_count,
                "writing ready peer Piece frames"
            );
            tokio::time::timeout_at(deadline, stream.write_vectored(&buffers[..buffer_count])).await
        };
        let written = match write_result {
            Ok(Ok(written)) => written,
            Ok(Err(error)) => {
                let completed = cursors
                    .iter()
                    .take(pieces.len())
                    .zip(pieces)
                    .take_while(|((header, payload), (_, _, data))| {
                        *header == 13 && *payload == data.len()
                    })
                    .count();
                return Err((completed, error));
            }
            Err(_) => {
                let completed = cursors
                    .iter()
                    .take(pieces.len())
                    .zip(pieces)
                    .take_while(|((header, payload), (_, _, data))| {
                        *header == 13 && *payload == data.len()
                    })
                    .count();
                return Err((
                    completed,
                    std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "peer socket write timed out",
                    ),
                ));
            }
        };
        if written == 0 {
            let completed = cursors
                .iter()
                .take(pieces.len())
                .zip(pieces)
                .take_while(|((header, payload), (_, _, data))| {
                    *header == 13 && *payload == data.len()
                })
                .count();
            return Err((
                completed,
                std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "failed to write peer Piece frames",
                ),
            ));
        }

        let mut remaining = written;
        for (index, (_, _, data)) in pieces.iter().enumerate() {
            if remaining == 0 {
                break;
            }
            let (header_offset, payload_offset) = &mut cursors[index];
            let header_written = remaining.min(headers[index].len() - *header_offset);
            *header_offset += header_written;
            remaining -= header_written;
            let payload_written = remaining.min(data.len() - *payload_offset);
            *payload_offset += payload_written;
            cursor.record(payload_written);
            remaining -= payload_written;
        }
        debug_assert_eq!(remaining, 0);
    }
}

pub(super) async fn read_upload_batch_blocks(
    upload: &UploadReadContext,
    specs: &[UploadReadSpec],
) -> anyhow::Result<UploadPayloadBatch> {
    anyhow::ensure!(
        !specs.is_empty() && specs.len() <= MAX_UPLOAD_READ_BATCH_BLOCKS,
        "upload batch has an invalid request count"
    );
    if specs.iter().any(|spec| spec.span.is_none()) {
        let mut blocks = UploadBatch::new();
        for spec in specs {
            blocks.push(
                read_upload_block_with_span(
                    upload,
                    spec.request,
                    spec.choke_critical,
                    spec.span.as_ref(),
                )
                .await?,
            );
        }
        return Ok(UploadPayloadBatch::Individual(Box::new(blocks)));
    }

    anyhow::ensure!(
        upload.plane == Plane::V1,
        "only v1 upload reads can be batched"
    );
    let first = &specs[0];
    let first_span = first.span.as_ref().expect("batch span checked above");
    let mut total_len = 0usize;
    for (index, spec) in specs.iter().enumerate() {
        let span = spec.span.as_ref().expect("batch span checked above");
        let block = spec.request.canonical_block();
        anyhow::ensure!(
            spec.request.wire_block() == block
                && block.length > 0
                && block.length <= MAX_BLOCK_SIZE
                && span.length == block.length as usize
                && spec.choke_critical == first.choke_critical,
            "upload batch contains an invalid or mismatched request"
        );
        if index > 0 {
            let previous = &specs[index - 1];
            let previous_span = previous.span.as_ref().expect("batch span checked above");
            anyhow::ensure!(
                block.piece == previous.request.piece
                    && span.file_ordinal == first_span.file_ordinal
                    && previous_span
                        .offset
                        .checked_add(previous_span.length as u64)
                        == Some(span.offset),
                "upload batch crossed a piece, file, or non-contiguous extent"
            );
        }
        total_len = total_len
            .checked_add(span.length)
            .ok_or_else(|| anyhow::anyhow!("upload batch length overflow"))?;
    }
    anyhow::ensure!(
        total_len <= MAX_UPLOAD_READ_BATCH_BYTES,
        "upload batch exceeds its bounded read size"
    );

    let mut blocks = [BlockExtent::new(0, 0, 1); MAX_UPLOAD_READ_BATCH_BLOCKS];
    for (block, spec) in blocks.iter_mut().zip(specs) {
        *block = spec.request.canonical_block();
    }
    let read_permits = upload
        .read_admission
        .acquire_read_batch_owned(&blocks[..specs.len()])
        .await
        .map_err(|error| anyhow::anyhow!("global upload read admission failed: {error:?}"))?;
    let lease = reserve_peer_upload_bytes(
        &upload.resources,
        u32::try_from(total_len).map_err(|_| anyhow::anyhow!("upload batch exceeds u32"))?,
    )?;

    let deadline = Instant::now() + PEER_SOCKET_WRITE_TIMEOUT;
    let priority = if first.choke_critical {
        IoRequestPriority::choke_critical(deadline)
    } else {
        IoRequestPriority::deadline(deadline)
    };
    let path = upload
        .piece_map
        .file_by_ordinal(first_span.file_ordinal)
        .ok_or_else(|| anyhow::anyhow!("upload batch file is outside its piece map"))?
        .path
        .resolve(&upload.save_root);
    #[cfg(all(target_os = "linux", feature = "seed-mmap"))]
    if upload.mapped_upload {
        if let Some(data) = upload
            .storage
            .read_upload_mapped(&path, first_span.offset, total_len, priority)
            .await?
        {
            return Ok(UploadPayloadBatch::Mapped {
                data,
                _lease: lease,
                _read_permit: read_permits,
            });
        }
    }
    let read = upload
        .storage
        .read_owned_at_with_priority(
            IoClass::PeerRead,
            &path,
            first_span.offset,
            total_len,
            priority,
        )
        .await?;
    anyhow::ensure!(
        read.len() == total_len,
        "batched upload read returned {} bytes, expected {}",
        read.len(),
        total_len
    );
    Ok(UploadPayloadBatch::Contiguous {
        data: read.into_bytes(),
        _lease: lease,
        _read_permit: read_permits,
    })
}

// Capture the owned inputs directly instead of retaining an async-fn
// argument copy alongside its moved locals in every reusable read slot.
// Rust 1.97 x86_64 layout: direct capture plus the delayed completion
// projection reduces each reusable future from 5,200 to 4,456 bytes.
#[allow(clippy::manual_async_fn)]
pub(super) fn upload_read_batch_future(
    specs: UploadBatch<UploadReadSpec>,
    read_context: Arc<UploadReadContext>,
) -> impl std::future::Future<Output = UploadBatchCompletion> {
    async move {
        let read = async {
            let generation = read_context.upload_control.generation();
            let total_bytes = specs
                .iter()
                .map(|spec| u64::from(spec.request.wire_length))
                .sum();
            let Some(first) = specs.first() else {
                anyhow::bail!("empty upload batch");
            };
            let credit = read_context
                .upload_rate
                .network_budget
                .acquire_upload_batch(
                    read_context.upload_rate.peer,
                    total_bytes,
                    u64::from(first.request.wire_length),
                    &read_context.upload_control,
                    generation,
                )
                .await?;
            let Some(credit) = credit else {
                return Ok((None, 0, None));
            };
            let admitted = if credit.bytes() == total_bytes {
                specs.len()
            } else {
                1
            };
            // Rate-limited demand holds no payload while waiting. Credit remains
            // bounded to one block; read cancellation/timeout refunds it by RAII.
            let payload = timeout(
                PEER_UPLOAD_READ_TIMEOUT,
                read_upload_batch_blocks(&read_context, &specs[..admitted]),
            )
            .await
            .map_err(|_| anyhow::anyhow!("upload read exceeded its peer deadline"))??;
            if read_context.upload_control.generation() != generation {
                return Ok((None, 0, None));
            }
            Ok::<_, anyhow::Error>((Some(payload), admitted, Some(credit)))
        };
        let outcome = std::panic::AssertUnwindSafe(read).catch_unwind().await;
        // Specs already retain the coordinates while rate/read work is pending.
        // Build the completion projection only after that work (including caught
        // errors/panics), preserving every original request for settlement.
        let requests: UploadBatch<_> = specs.iter().map(|spec| spec.request).collect();
        match outcome {
            Ok(Ok((payload, admitted, credit))) => UploadBatchCompletion {
                requests,
                admitted,
                credit,
                interrupted: payload.is_none(),
                panicked: false,
                result: payload.ok_or_else(|| anyhow::anyhow!("upload admission interrupted")),
            },
            Ok(Err(error)) => UploadBatchCompletion {
                admitted: requests.len(),
                requests,
                credit: None,
                result: Err(error),
                interrupted: false,
                panicked: false,
            },
            Err(_) => UploadBatchCompletion {
                admitted: requests.len(),
                requests,
                credit: None,
                result: Err(anyhow::anyhow!("upload read panicked")),
                interrupted: false,
                panicked: true,
            },
        }
    }
}
