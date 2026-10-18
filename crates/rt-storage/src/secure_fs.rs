//! Descriptor-anchored filesystem operations for root-confined plans.
//!
//! Path validation is not enough for a storage sandbox: an ancestor can be
//! replaced between canonicalization and the eventual open/rename/unlink.
//! These helpers walk every ancestor from an already-opened root directory
//! with `O_NOFOLLOW`, then perform the operation relative to the resulting
//! directory descriptors. The descriptors remain anchored if an attacker
//! renames or replaces a path component after the walk.

use std::collections::hash_map::DefaultHasher;
use std::ffi::{CStr, CString, OsString};
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use sha1::{Digest, Sha1};

use crate::plan::ensure_storage_tree_depth;
use crate::{PlannedStorageAction, StorageError, StoragePlan, StoragePlanStep};

// Serialize the check/open/rename sequence among TorrentNG workers that
// target the same destination. Linux's renameat2(RENAME_NOREPLACE) below
// also closes the race against other processes; this lock only prevents two
// in-process workers from needlessly contending on the *same* destination
// before reaching that syscall.
//
// The lock is sharded by a hash of the destination path rather than global:
// a single process-wide mutex would serialize every rename in the daemon --
// across every torrent and every storage device -- for a race that can only
// ever occur between two workers targeting the identical path. With many
// shards, unrelated destinations almost always land in different shards and
// proceed concurrently, while two workers racing the same destination still
// reliably hash to the same shard and serialize as intended.
const RENAME_LOCK_SHARDS: usize = 64;
const DIRECTORY_DELETE_BATCH_ENTRIES: usize = 256;
static RENAME_SERIAL: OnceLock<Vec<Mutex<()>>> = OnceLock::new();

fn rename_lock_shard(destination: &Path) -> &Mutex<()> {
    let shards = RENAME_SERIAL.get_or_init(|| {
        std::iter::repeat_with(|| Mutex::new(()))
            .take(RENAME_LOCK_SHARDS)
            .collect()
    });
    let mut hasher = DefaultHasher::new();
    destination.hash(&mut hasher);
    let index = (hasher.finish() as usize) % shards.len();
    &shards[index]
}

pub(crate) fn execute_step(step: &StoragePlanStep, roots: &[PathBuf]) -> Result<(), StorageError> {
    let no_control = || Ok(());
    execute_step_with_control(step, roots, &no_control)
}

pub(crate) fn execute_step_with_control(
    step: &StoragePlanStep,
    roots: &[PathBuf],
    check_control: &dyn Fn() -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    check_control()?;
    match step.action {
        PlannedStorageAction::ImportExisting => {
            let source = required_path(step.source.as_ref(), "import-source")?;
            let destination = required_path(step.destination.as_ref(), "import-destination")?;
            secure_import(source, destination, step.bytes, roots, check_control)
        }
        PlannedStorageAction::Rename => {
            let source = required_path(step.source.as_ref(), "rename-source")?;
            let destination = required_path(step.destination.as_ref(), "rename-destination")?;
            secure_rename(source, destination, step.bytes, roots, check_control)
        }
        PlannedStorageAction::CopyVerifyRename => {
            let source = required_path(step.source.as_ref(), "copy-source")?;
            let destination = required_path(step.destination.as_ref(), "copy-destination")?;
            secure_copy(source, destination, step.bytes, roots, check_control)
        }
        PlannedStorageAction::SafeDelete => {
            let source = required_path(step.source.as_ref(), "delete-source")?;
            secure_delete(source, roots, false, check_control)
        }
        PlannedStorageAction::SafeDeleteIfPresent => {
            let source = required_path(step.source.as_ref(), "delete-source")?;
            secure_delete(source, roots, true, check_control)
        }
        PlannedStorageAction::PruneEmptyDirs => {
            let source = required_path(step.source.as_ref(), "prune-source")?;
            let root = required_path(step.destination.as_ref(), "prune-root")?;
            secure_prune(source, root, roots, check_control)
        }
    }
}

pub(crate) fn step_is_applied(
    plan: &StoragePlan,
    index: usize,
    roots: &[PathBuf],
    checkpointed: bool,
) -> Result<bool, StorageError> {
    let step = &plan.steps[index];
    match step.action {
        PlannedStorageAction::Rename => {
            let source = required_path(step.source.as_ref(), "reconcile-source")?;
            let destination = required_path(step.destination.as_ref(), "reconcile-destination")?;
            let source_state = entry_state(source, roots)?;
            let destination_state = entry_state(destination, roots)?;
            if source_state.is_some() && destination_state.is_some() {
                return Err(staged_uncertain(
                    "reconcile",
                    format!(
                        "rename has both source and destination present: {} -> {}",
                        source.display(),
                        destination.display()
                    ),
                ));
            }
            if source_state.is_none() && destination_state.is_some() {
                if checkpointed {
                    if destination_state.is_some_and(EntryType::is_symlink) {
                        return Err(staged_uncertain(
                            "reconcile",
                            format!(
                                "checkpointed rename destination is a symlink: {}",
                                destination.display()
                            ),
                        ));
                    }
                    verify_checkpointed_length(
                        destination,
                        step.bytes,
                        roots,
                        "reconcile-destination",
                    )?;
                    // A durable checkpoint is written only after the rename
                    // syscall returns. Once the source is gone, the
                    // checkpoint is the only proof available that this
                    // destructive step was the operation that published the
                    // destination. Do not demand the now-deleted source or
                    // repeat the rename against a newly-created source.
                    return Ok(true);
                }
                let Some(previous) = index
                    .checked_sub(1)
                    .and_then(|previous| plan.steps.get(previous))
                else {
                    return Err(staged_uncertain(
                        "reconcile",
                        format!(
                            "cannot prove rename completion after source disappeared: {} -> {}",
                            source.display(),
                            destination.display()
                        ),
                    ));
                };
                let previous_source =
                    required_path(previous.source.as_ref(), "reconcile-previous-source")?;
                if !matches!(previous.action, PlannedStorageAction::CopyVerifyRename)
                    || previous.destination.as_deref() != Some(source)
                    || entry_state(previous_source, roots)?.is_none()
                {
                    return Err(staged_uncertain(
                        "reconcile",
                        format!(
                            "cannot prove rename completion after source disappeared: {} -> {}",
                            source.display(),
                            destination.display()
                        ),
                    ));
                }
                reconcile_content(previous_source, destination, roots)?;
                return Ok(true);
            }
            if checkpointed {
                return Err(staged_uncertain(
                    "reconcile",
                    format!(
                        "checkpointed rename is not in its committed state: {} -> {}",
                        source.display(),
                        destination.display()
                    ),
                ));
            }
            Ok(false)
        }
        PlannedStorageAction::CopyVerifyRename | PlannedStorageAction::ImportExisting => {
            let source = required_path(step.source.as_ref(), "reconcile-source")?;
            let destination = required_path(step.destination.as_ref(), "reconcile-destination")?;
            if entry_state(destination, roots)?.is_none() {
                return Ok(false);
            }
            if entry_state(source, roots)?.is_none() {
                return Err(staged_uncertain(
                    "reconcile",
                    format!(
                        "cannot prove copy or import completion after source disappeared: {} -> {}",
                        source.display(),
                        destination.display()
                    ),
                ));
            }
            reconcile_content(source, destination, roots)?;
            Ok(true)
        }
        PlannedStorageAction::SafeDelete | PlannedStorageAction::SafeDeleteIfPresent => {
            let source = required_path(step.source.as_ref(), "reconcile-source")?;
            let source_exists = entry_state(source, roots)?.is_some();
            if checkpointed && source_exists {
                return Err(staged_uncertain(
                    "reconcile",
                    format!(
                        "checkpointed delete target reappeared and will not be deleted again: {}",
                        source.display()
                    ),
                ));
            }
            Ok(!source_exists)
        }
        PlannedStorageAction::PruneEmptyDirs => Ok(entry_state(
            required_path(step.source.as_ref(), "reconcile-source")?,
            roots,
        )?
        .is_none()),
    }
}

fn verify_checkpointed_length(
    path: &Path,
    expected_bytes: u64,
    roots: &[PathBuf],
    step: &'static str,
) -> Result<(), StorageError> {
    if expected_bytes == u64::MAX {
        return Ok(());
    }
    let (parent, name) = open_parent(path, roots, false, step)?;
    let no_control = || Ok(());
    let actual = content_len_at(&parent, &name, path, 0, &no_control)?;
    if actual == expected_bytes {
        Ok(())
    } else {
        Err(staged_uncertain(
            step,
            format!(
                "checkpointed destination has {actual} bytes, expected {expected_bytes}: {}",
                path.display()
            ),
        ))
    }
}

fn reconcile_content(
    source: &Path,
    destination: &Path,
    roots: &[PathBuf],
) -> Result<(), StorageError> {
    verify_content(source, destination, roots).map_err(|error| match error {
        StorageError::FilesystemStateUncertain { .. } => error,
        error => staged_uncertain("reconcile", error.to_string()),
    })
}

pub(crate) fn rollback_plan(
    plan: &StoragePlan,
    roots: &[PathBuf],
) -> (Vec<StoragePlanStep>, Vec<(StoragePlanStep, String)>) {
    let mut rolled_back = Vec::new();
    let mut failures = Vec::new();
    for step in &plan.rollback_steps {
        match execute_step(step, roots) {
            Ok(()) => rolled_back.push(step.clone()),
            Err(error) => failures.push((step.clone(), error.to_string())),
        }
    }
    (rolled_back, failures)
}

fn sync_directory(directory: &File, path: &Path) -> Result<(), StorageError> {
    directory
        .sync_all()
        .map_err(|error| StorageError::io(path.display().to_string(), error))
}

fn secure_import(
    source: &Path,
    destination: &Path,
    expected_bytes: u64,
    roots: &[PathBuf],
    check_control: &dyn Fn() -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    check_control()?;
    let (source_parent, source_name) = open_parent(source, roots, false, "import-source")?;
    let (destination_parent, destination_name) =
        open_parent(destination, roots, true, "import-destination")?;
    ensure_destination_available(&destination_parent, &destination_name, destination)?;
    let source_type = entry_type(&source_parent, &source_name, source)?;
    if source_type.is_symlink() {
        return Err(unsafe_symlink(source, "import-source"));
    }
    // Reject a stale expected size before creating a hard link. If the source
    // changes between this check and linkat, the post-link check below still
    // protects the plan and removes the link it just created.
    verify_expected_length(
        &source_parent,
        &source_name,
        expected_bytes,
        source,
        check_control,
    )?;

    // Hard links are the cheap and atomic import path for regular files. A
    // directory cannot be linked, so it falls through to the same secure
    // recursive copy implementation used by cross-filesystem imports.
    let link_result = if source_type.is_file() {
        let source_c = c_name(&source_name, source)?;
        let destination_c = c_name(&destination_name, destination)?;
        unsafe {
            libc::linkat(
                source_parent.as_raw_fd(),
                source_c.as_ptr(),
                destination_parent.as_raw_fd(),
                destination_c.as_ptr(),
                0,
            )
        }
    } else {
        -1
    };
    if link_result == 0 {
        let verification = (|| {
            if entry_type(&destination_parent, &destination_name, destination)?.is_symlink() {
                return Err(unsafe_symlink(source, "import-source"));
            }
            verify_expected_length(
                &destination_parent,
                &destination_name,
                expected_bytes,
                destination,
                check_control,
            )
        })();
        if let Err(error) = verification {
            if let Err(cleanup_error) =
                unlink_at(&destination_parent, &destination_name, destination, 0)
            {
                return Err(staged_uncertain(
                    "import-cleanup",
                    format!(
                        "{error}; failed to remove hard-link destination {}: {cleanup_error}",
                        destination.display()
                    ),
                ));
            }
            if let Err(sync_error) = sync_directory(&destination_parent, destination) {
                return Err(staged_uncertain(
                    "import-cleanup-sync",
                    format!(
                        "{error}; removed hard-link destination but failed to sync its parent: {sync_error}"
                    ),
                ));
            }
            return Err(error);
        }
        sync_directory(&source_parent, source).map_err(|error| {
            staged_uncertain(
                "import-sync",
                format!(
                    "hard-link import committed but failed to sync source directory {}: {error}",
                    source.display()
                ),
            )
        })?;
        sync_directory(&destination_parent, destination).map_err(|error| {
            staged_uncertain(
                "import-sync",
                format!(
                    "hard-link import committed but failed to sync destination directory {}: {error}",
                    destination.display()
                ),
            )
        })?;
        Ok(())
    } else {
        secure_copy_entry(
            &source_parent,
            &source_name,
            &destination_parent,
            &destination_name,
            (source, destination, 0),
            expected_bytes,
            true,
            check_control,
        )
    }
}

fn secure_copy(
    source: &Path,
    destination: &Path,
    expected_bytes: u64,
    roots: &[PathBuf],
    check_control: &dyn Fn() -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    check_control()?;
    let (source_parent, source_name) = open_parent(source, roots, false, "copy-source")?;
    let (destination_parent, destination_name) =
        open_parent(destination, roots, true, "copy-destination")?;
    ensure_destination_available(&destination_parent, &destination_name, destination)?;
    secure_copy_entry(
        &source_parent,
        &source_name,
        &destination_parent,
        &destination_name,
        (source, destination, 0),
        expected_bytes,
        true,
        check_control,
    )
}

const SECURE_COPY_QUANTUM_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy)]
enum FileCopyPhase {
    Copy,
    HashSource,
    HashDestination,
}

/// A root-confined regular-file copy whose disk work is advanced in bounded
/// quanta. The cursor retains only descriptors and hash state between device
/// admissions; the 64 KiB scratch buffer exists only inside `advance_quantum`.
pub(crate) struct FileCopyCursor {
    source_parent: File,
    source_name: OsString,
    source_path: PathBuf,
    destination_parent: File,
    destination_name: OsString,
    destination_path: PathBuf,
    source_copy: Option<File>,
    destination_copy: Option<File>,
    source_verify: Option<File>,
    destination_verify: Option<File>,
    expected_bytes: u64,
    copied_bytes: u64,
    source_hash: Sha1,
    destination_hash: Sha1,
    phase: FileCopyPhase,
    copy_only: bool,
    cleanup_on_error: bool,
    sync_directory_after_copy: bool,
    sync_directory_after_verify: bool,
}

pub(crate) enum CopyCursor {
    File(FileCopyCursor),
    Directory(DirectoryCopyCursor),
}

impl CopyCursor {
    #[cfg(test)]
    pub(crate) fn copied_bytes(&self) -> u64 {
        match self {
            Self::File(cursor) => cursor.copied_bytes,
            Self::Directory(cursor) => cursor.copied_bytes.saturating_add(
                cursor
                    .active_file
                    .as_ref()
                    .map_or(0, |file| file.copied_bytes),
            ),
        }
    }

    pub(crate) fn advance_quantum(
        &mut self,
        check_control: &dyn Fn() -> Result<(), StorageError>,
    ) -> Result<bool, StorageError> {
        match self {
            Self::File(cursor) => cursor.advance_quantum(check_control),
            Self::Directory(cursor) => cursor.advance_quantum(check_control),
        }
    }

    pub(crate) fn discard_partial_destination(&mut self) -> Result<(), StorageError> {
        match self {
            Self::File(cursor) => cursor.discard_partial_destination(),
            Self::Directory(cursor) => cursor.discard_partial_destination(),
        }
    }
}

pub(crate) fn start_file_copy_cursor(
    source: &Path,
    destination: &Path,
    expected_bytes: u64,
    roots: &[PathBuf],
    check_control: &dyn Fn() -> Result<(), StorageError>,
    allow_hard_link: bool,
) -> Result<(Option<CopyCursor>, bool), StorageError> {
    check_control()?;
    let (source_parent, source_name) = open_parent(source, roots, false, "copy-source")?;
    let source_type = entry_type(&source_parent, &source_name, source)?;
    if source_type.is_symlink() {
        return Err(unsafe_symlink(source, "copy-source"));
    }
    if source_type.is_dir() {
        let (destination_parent, destination_name) =
            open_parent(destination, roots, true, "copy-destination")?;
        ensure_destination_available(&destination_parent, &destination_name, destination)?;
        mkdir_at(&destination_parent, &destination_name, destination)?;
        let result = (|| {
            let source_dir = open_dir_at(&source_parent, &source_name, source)
                .map_err(|error| StorageError::io(source.display().to_string(), error))?;
            let destination_dir = open_dir_at(&destination_parent, &destination_name, destination)
                .map_err(|error| StorageError::io(destination.display().to_string(), error))?;
            let frame = DirectoryCopyFrame::new(
                source_dir,
                destination_dir,
                source.to_path_buf(),
                destination.to_path_buf(),
                0,
                DirectoryCopyPhase::Copy,
            )?;
            Ok(DirectoryCopyCursor {
                source_parent: source_parent
                    .try_clone()
                    .map_err(|error| StorageError::io(source.display().to_string(), error))?,
                source_name: source_name.clone(),
                source_path: source.to_path_buf(),
                destination_parent: destination_parent
                    .try_clone()
                    .map_err(|error| StorageError::io(destination.display().to_string(), error))?,
                destination_name: destination_name.clone(),
                destination_path: destination.to_path_buf(),
                expected_bytes,
                copied_bytes: 0,
                active_file: None,
                phase: DirectoryCopyPhase::Copy,
                stack: vec![frame],
            })
        })();
        return match result {
            Ok(cursor) => Ok((Some(CopyCursor::Directory(cursor)), false)),
            Err(error) => {
                let no_control = || Ok(());
                if let Err(cleanup_error) = remove_entry_at(
                    &destination_parent,
                    &destination_name,
                    destination,
                    true,
                    &no_control,
                ) {
                    return Err(staged_uncertain(
                        "copy-cleanup",
                        format!("{error}; failed to remove partial destination: {cleanup_error}"),
                    ));
                }
                Err(error)
            }
        };
    }
    if !source_type.is_file() {
        return Err(staged(
            "copy-source",
            format!("unsupported file type: {}", source.display()),
        ));
    }

    let (destination_parent, destination_name) =
        open_parent(destination, roots, true, "copy-destination")?;
    ensure_destination_available(&destination_parent, &destination_name, destination)?;

    if allow_hard_link {
        verify_expected_length(
            &source_parent,
            &source_name,
            expected_bytes,
            source,
            check_control,
        )?;
        let source_c = c_name(&source_name, source)?;
        let destination_c = c_name(&destination_name, destination)?;
        let linked = unsafe {
            libc::linkat(
                source_parent.as_raw_fd(),
                source_c.as_ptr(),
                destination_parent.as_raw_fd(),
                destination_c.as_ptr(),
                0,
            )
        } == 0;
        if linked {
            let verification = verify_expected_length(
                &destination_parent,
                &destination_name,
                expected_bytes,
                destination,
                check_control,
            );
            if let Err(error) = verification {
                if let Err(cleanup_error) =
                    unlink_at(&destination_parent, &destination_name, destination, 0)
                {
                    return Err(staged_uncertain(
                        "import-cleanup",
                        format!(
                            "{error}; failed to remove hard-link destination {}: {cleanup_error}",
                            destination.display()
                        ),
                    ));
                }
                if let Err(sync_error) = sync_directory(&destination_parent, destination) {
                    return Err(staged_uncertain(
                        "import-cleanup-sync",
                        format!(
                            "{error}; removed hard-link destination but failed to sync its parent: {sync_error}"
                        ),
                    ));
                }
                return Err(error);
            }
            sync_directory(&source_parent, source).map_err(|error| {
                staged_uncertain(
                    "import-sync",
                    format!(
                        "hard-link import committed but failed to sync source directory {}: {error}",
                        source.display()
                    ),
                )
            })?;
            sync_directory(&destination_parent, destination).map_err(|error| {
                staged_uncertain(
                    "import-sync",
                    format!(
                        "hard-link import committed but failed to sync destination directory {}: {error}",
                        destination.display()
                    ),
                )
            })?;
            return Ok((None, true));
        }
    }

    let source_copy = open_file_at(
        &source_parent,
        &source_name,
        source,
        libc::O_RDONLY | libc::O_NONBLOCK,
    )?;
    let source_metadata = source_copy
        .metadata()
        .map_err(|error| StorageError::io(source.display().to_string(), error))?;
    if !source_metadata.file_type().is_file() {
        return Err(staged(
            "copy-source",
            format!("not a regular file: {}", source.display()),
        ));
    }

    let destination_copy = create_file_at(&destination_parent, &destination_name, destination)?;

    Ok((
        Some(CopyCursor::File(FileCopyCursor {
            source_parent,
            source_name,
            source_path: source.to_path_buf(),
            destination_parent,
            destination_name,
            destination_path: destination.to_path_buf(),
            source_copy: Some(source_copy),
            destination_copy: Some(destination_copy),
            source_verify: None,
            destination_verify: None,
            expected_bytes,
            copied_bytes: 0,
            source_hash: Sha1::new(),
            destination_hash: Sha1::new(),
            phase: FileCopyPhase::Copy,
            copy_only: false,
            cleanup_on_error: true,
            sync_directory_after_copy: false,
            sync_directory_after_verify: true,
        })),
        false,
    ))
}

fn start_file_copy_cursor_at(
    source_parent: &File,
    source_name: &OsString,
    source_path: &Path,
    destination_parent: &File,
    destination_name: &OsString,
    destination_path: &Path,
    expected_bytes: u64,
) -> Result<FileCopyCursor, StorageError> {
    let source_type = entry_type(source_parent, source_name, source_path)?;
    if source_type.is_symlink() {
        return Err(unsafe_symlink(source_path, "copy-source"));
    }
    if !source_type.is_file() {
        return Err(staged(
            "copy-source",
            format!("not a regular file: {}", source_path.display()),
        ));
    }
    ensure_destination_available(destination_parent, destination_name, destination_path)?;
    let source_copy = open_file_at(
        source_parent,
        source_name,
        source_path,
        libc::O_RDONLY | libc::O_NONBLOCK,
    )?;
    let source_metadata = source_copy
        .metadata()
        .map_err(|error| StorageError::io(source_path.display().to_string(), error))?;
    if !source_metadata.file_type().is_file() {
        return Err(staged(
            "copy-source",
            format!("not a regular file: {}", source_path.display()),
        ));
    }
    let destination_copy = create_file_at(destination_parent, destination_name, destination_path)?;
    let expected_bytes = if expected_bytes == u64::MAX {
        source_metadata.len()
    } else {
        expected_bytes
    };
    Ok(FileCopyCursor {
        source_parent: source_parent
            .try_clone()
            .map_err(|error| StorageError::io(source_path.display().to_string(), error))?,
        source_name: source_name.clone(),
        source_path: source_path.to_path_buf(),
        destination_parent: destination_parent
            .try_clone()
            .map_err(|error| StorageError::io(destination_path.display().to_string(), error))?,
        destination_name: destination_name.clone(),
        destination_path: destination_path.to_path_buf(),
        source_copy: Some(source_copy),
        destination_copy: Some(destination_copy),
        source_verify: None,
        destination_verify: None,
        expected_bytes,
        copied_bytes: 0,
        source_hash: Sha1::new(),
        destination_hash: Sha1::new(),
        phase: FileCopyPhase::Copy,
        copy_only: true,
        cleanup_on_error: false,
        sync_directory_after_copy: false,
        sync_directory_after_verify: false,
    })
}

fn start_file_verification_cursor_at(
    source_parent: &File,
    source_name: &OsString,
    source_path: &Path,
    destination_parent: &File,
    destination_name: &OsString,
    destination_path: &Path,
) -> Result<FileCopyCursor, StorageError> {
    let source_type = entry_type(source_parent, source_name, source_path)?;
    let destination_type = entry_type(destination_parent, destination_name, destination_path)?;
    if source_type.is_symlink() {
        return Err(unsafe_symlink(source_path, "verify-content-source"));
    }
    if destination_type.is_symlink() {
        return Err(unsafe_symlink(
            destination_path,
            "verify-content-destination",
        ));
    }
    if !source_type.is_file() || !destination_type.is_file() {
        return Err(staged(
            "verify-content",
            format!("file type differs at {}", source_path.display()),
        ));
    }
    let source_verify = open_file_at(
        source_parent,
        source_name,
        source_path,
        libc::O_RDONLY | libc::O_NONBLOCK,
    )?;
    let destination_verify = open_file_at(
        destination_parent,
        destination_name,
        destination_path,
        libc::O_RDONLY | libc::O_NONBLOCK,
    )?;
    let source_len = source_verify
        .metadata()
        .map_err(|error| StorageError::io(source_path.display().to_string(), error))?
        .len();
    let destination_len = destination_verify
        .metadata()
        .map_err(|error| StorageError::io(destination_path.display().to_string(), error))?
        .len();
    if source_len != destination_len {
        return Err(short_copy_io(destination_path, source_len, destination_len));
    }
    Ok(FileCopyCursor {
        source_parent: source_parent
            .try_clone()
            .map_err(|error| StorageError::io(source_path.display().to_string(), error))?,
        source_name: source_name.clone(),
        source_path: source_path.to_path_buf(),
        destination_parent: destination_parent
            .try_clone()
            .map_err(|error| StorageError::io(destination_path.display().to_string(), error))?,
        destination_name: destination_name.clone(),
        destination_path: destination_path.to_path_buf(),
        source_copy: None,
        destination_copy: None,
        source_verify: Some(source_verify),
        destination_verify: Some(destination_verify),
        expected_bytes: source_len,
        copied_bytes: 0,
        source_hash: Sha1::new(),
        destination_hash: Sha1::new(),
        phase: FileCopyPhase::HashSource,
        copy_only: true,
        cleanup_on_error: false,
        sync_directory_after_copy: false,
        sync_directory_after_verify: false,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DirectoryCopyPhase {
    Copy,
    Verify,
}

struct SecureDirectoryReader {
    stream: *mut libc::DIR,
    path: PathBuf,
}

// The cursor is moved between scheduler workers but never accessed
// concurrently; ownership stays in the storage plan's cursor slot.
unsafe impl Send for SecureDirectoryReader {}

impl SecureDirectoryReader {
    fn new(directory: &File, path: &Path) -> Result<Self, StorageError> {
        let stream_file = open_dir_at(directory, &OsString::from("."), path)
            .map_err(|error| StorageError::io(path.display().to_string(), error))?;
        let stream_fd = stream_file.into_raw_fd();
        let stream = unsafe { libc::fdopendir(stream_fd) };
        if stream.is_null() {
            unsafe { libc::close(stream_fd) };
            return Err(StorageError::io(
                path.display().to_string(),
                io::Error::last_os_error(),
            ));
        }
        Ok(Self {
            stream,
            path: path.to_path_buf(),
        })
    }

    fn next_name(&mut self) -> Result<Option<OsString>, StorageError> {
        loop {
            clear_errno();
            let entry = unsafe { libc::readdir(self.stream) };
            if entry.is_null() {
                let error = io::Error::last_os_error();
                if error.raw_os_error().unwrap_or_default() != 0 {
                    return Err(StorageError::io(self.path.display().to_string(), error));
                }
                return Ok(None);
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if name != b"." && name != b".." {
                return Ok(Some(OsString::from_vec(name.to_vec())));
            }
        }
    }
}

impl Drop for SecureDirectoryReader {
    fn drop(&mut self) {
        if !self.stream.is_null() {
            unsafe { libc::closedir(self.stream) };
            self.stream = std::ptr::null_mut();
        }
    }
}

struct DirectoryCopyFrame {
    source_dir: File,
    destination_dir: File,
    source_path: PathBuf,
    destination_path: PathBuf,
    depth: usize,
    source_reader: SecureDirectoryReader,
    destination_reader: Option<SecureDirectoryReader>,
    checking_destination: bool,
}

impl DirectoryCopyFrame {
    fn new(
        source_dir: File,
        destination_dir: File,
        source_path: PathBuf,
        destination_path: PathBuf,
        depth: usize,
        phase: DirectoryCopyPhase,
    ) -> Result<Self, StorageError> {
        let source_reader = SecureDirectoryReader::new(&source_dir, &source_path)?;
        let destination_reader = if phase == DirectoryCopyPhase::Verify {
            Some(SecureDirectoryReader::new(
                &destination_dir,
                &destination_path,
            )?)
        } else {
            None
        };
        Ok(Self {
            source_dir,
            destination_dir,
            source_path,
            destination_path,
            depth,
            source_reader,
            destination_reader,
            checking_destination: false,
        })
    }
}

struct DirectoryCopyCursor {
    source_parent: File,
    source_name: OsString,
    source_path: PathBuf,
    destination_parent: File,
    destination_name: OsString,
    destination_path: PathBuf,
    expected_bytes: u64,
    copied_bytes: u64,
    active_file: Option<FileCopyCursor>,
    phase: DirectoryCopyPhase,
    stack: Vec<DirectoryCopyFrame>,
}

impl DirectoryCopyCursor {
    fn advance_quantum(
        &mut self,
        check_control: &dyn Fn() -> Result<(), StorageError>,
    ) -> Result<bool, StorageError> {
        match self.advance_quantum_inner(check_control) {
            Ok(complete) => Ok(complete),
            Err(error) => Err(self.cleanup_partial_destination(error)),
        }
    }

    fn advance_quantum_inner(
        &mut self,
        check_control: &dyn Fn() -> Result<(), StorageError>,
    ) -> Result<bool, StorageError> {
        check_control()?;
        if self.active_file.is_some() {
            let complete = self
                .active_file
                .as_mut()
                .expect("active file was present")
                .advance_quantum(check_control)?;
            if complete {
                let file = self.active_file.take().expect("active file was present");
                if self.phase == DirectoryCopyPhase::Copy {
                    let next = self
                        .copied_bytes
                        .checked_add(file.copied_bytes)
                        .ok_or_else(|| staged("copy-size", "copied byte count overflow"))?;
                    if self.expected_bytes != u64::MAX && next > self.expected_bytes {
                        return Err(short_copy_io(
                            &self.destination_path,
                            self.expected_bytes,
                            next,
                        ));
                    }
                    self.copied_bytes = next;
                }
            }
            return Ok(false);
        }

        match self.phase {
            DirectoryCopyPhase::Copy => self.advance_copy_tree(check_control),
            DirectoryCopyPhase::Verify => self.advance_verify_tree(check_control),
        }
    }

    fn advance_copy_tree(
        &mut self,
        check_control: &dyn Fn() -> Result<(), StorageError>,
    ) -> Result<bool, StorageError> {
        check_control()?;
        let Some(index) = self.stack.len().checked_sub(1) else {
            if self.expected_bytes != u64::MAX && self.copied_bytes != self.expected_bytes {
                return Err(short_copy_io(
                    &self.destination_path,
                    self.expected_bytes,
                    self.copied_bytes,
                ));
            }
            let source_dir = open_dir_at(&self.source_parent, &self.source_name, &self.source_path)
                .map_err(|error| StorageError::io(self.source_path.display().to_string(), error))?;
            let destination_dir = open_dir_at(
                &self.destination_parent,
                &self.destination_name,
                &self.destination_path,
            )
            .map_err(|error| {
                StorageError::io(self.destination_path.display().to_string(), error)
            })?;
            self.stack.push(DirectoryCopyFrame::new(
                source_dir,
                destination_dir,
                self.source_path.clone(),
                self.destination_path.clone(),
                0,
                DirectoryCopyPhase::Verify,
            )?);
            self.phase = DirectoryCopyPhase::Verify;
            return Ok(false);
        };

        let name = self.stack[index].source_reader.next_name()?;
        let Some(name) = name else {
            sync_directory(
                &self.stack[index].destination_dir,
                &self.stack[index].destination_path,
            )?;
            self.stack.pop();
            return Ok(false);
        };
        let source_path = self.stack[index].source_path.join(&name);
        let destination_path = self.stack[index].destination_path.join(&name);
        let source_type = entry_type(&self.stack[index].source_dir, &name, &source_path)?;
        if source_type.is_symlink() {
            return Err(unsafe_symlink(&source_path, "copy-source"));
        }
        if source_type.is_dir() {
            let depth = self.stack[index].depth.saturating_add(1);
            ensure_storage_tree_depth(depth, &source_path, "copy-tree-depth")?;
            mkdir_at(&self.stack[index].destination_dir, &name, &destination_path)?;
            let source_dir = open_dir_at(&self.stack[index].source_dir, &name, &source_path)
                .map_err(|error| StorageError::io(source_path.display().to_string(), error))?;
            let destination_dir =
                open_dir_at(&self.stack[index].destination_dir, &name, &destination_path).map_err(
                    |error| StorageError::io(destination_path.display().to_string(), error),
                )?;
            self.stack.push(DirectoryCopyFrame::new(
                source_dir,
                destination_dir,
                source_path,
                destination_path,
                depth,
                DirectoryCopyPhase::Copy,
            )?);
            return Ok(false);
        }
        if !source_type.is_file() {
            return Err(staged(
                "copy-source",
                format!("unsupported file type: {}", source_path.display()),
            ));
        }
        self.active_file = Some(start_file_copy_cursor_at(
            &self.stack[index].source_dir,
            &name,
            &source_path,
            &self.stack[index].destination_dir,
            &name,
            &destination_path,
            u64::MAX,
        )?);
        Ok(false)
    }

    fn advance_verify_tree(
        &mut self,
        check_control: &dyn Fn() -> Result<(), StorageError>,
    ) -> Result<bool, StorageError> {
        check_control()?;
        let Some(index) = self.stack.len().checked_sub(1) else {
            sync_directory(&self.destination_parent, &self.destination_path)?;
            return Ok(true);
        };
        if !self.stack[index].checking_destination {
            let name = self.stack[index].source_reader.next_name()?;
            let Some(name) = name else {
                self.stack[index].checking_destination = true;
                return Ok(false);
            };
            let source_path = self.stack[index].source_path.join(&name);
            let destination_path = self.stack[index].destination_path.join(&name);
            let source_type = entry_type(&self.stack[index].source_dir, &name, &source_path)?;
            let destination_type =
                entry_type(&self.stack[index].destination_dir, &name, &destination_path)?;
            if source_type.is_symlink() {
                return Err(unsafe_symlink(&source_path, "verify-content-source"));
            }
            if destination_type.is_symlink() {
                return Err(unsafe_symlink(
                    &destination_path,
                    "verify-content-destination",
                ));
            }
            if source_type.is_dir() && destination_type.is_dir() {
                let depth = self.stack[index].depth.saturating_add(1);
                ensure_storage_tree_depth(depth, &source_path, "verify-tree-depth")?;
                let source_dir = open_dir_at(&self.stack[index].source_dir, &name, &source_path)
                    .map_err(|error| StorageError::io(source_path.display().to_string(), error))?;
                let destination_dir =
                    open_dir_at(&self.stack[index].destination_dir, &name, &destination_path)
                        .map_err(|error| {
                            StorageError::io(destination_path.display().to_string(), error)
                        })?;
                self.stack.push(DirectoryCopyFrame::new(
                    source_dir,
                    destination_dir,
                    source_path,
                    destination_path,
                    depth,
                    DirectoryCopyPhase::Verify,
                )?);
                return Ok(false);
            }
            if !source_type.is_file() || !destination_type.is_file() {
                return Err(staged(
                    "verify-content",
                    format!("file type differs at {}", source_path.display()),
                ));
            }
            self.active_file = Some(start_file_verification_cursor_at(
                &self.stack[index].source_dir,
                &name,
                &source_path,
                &self.stack[index].destination_dir,
                &name,
                &destination_path,
            )?);
            return Ok(false);
        }

        let name = self.stack[index]
            .destination_reader
            .as_mut()
            .expect("verification frame has destination reader")
            .next_name()?;
        let Some(name) = name else {
            self.stack.pop();
            return Ok(false);
        };
        let source_path = self.stack[index].source_path.join(&name);
        let destination_path = self.stack[index].destination_path.join(&name);
        let source_type = entry_type(&self.stack[index].source_dir, &name, &source_path)?;
        let destination_type =
            entry_type(&self.stack[index].destination_dir, &name, &destination_path)?;
        if source_type.is_symlink() {
            return Err(unsafe_symlink(&source_path, "verify-content-source"));
        }
        if destination_type.is_symlink() {
            return Err(unsafe_symlink(
                &destination_path,
                "verify-content-destination",
            ));
        }
        if source_type.is_dir() != destination_type.is_dir()
            || source_type.is_file() != destination_type.is_file()
        {
            return Err(staged(
                "verify-content",
                format!("directory entries differ at {}", destination_path.display()),
            ));
        }
        Ok(false)
    }

    fn cleanup_partial_destination(&mut self, error: StorageError) -> StorageError {
        match self.discard_partial_destination() {
            Ok(()) => error,
            Err(cleanup_error) => staged_uncertain(
                "copy-cleanup",
                format!("{error}; failed to clean partial destination: {cleanup_error}"),
            ),
        }
    }

    fn discard_partial_destination(&mut self) -> Result<(), StorageError> {
        self.active_file.take();
        self.stack.clear();
        let no_control = || Ok(());
        remove_entry_at(
            &self.destination_parent,
            &self.destination_name,
            &self.destination_path,
            true,
            &no_control,
        )
        .map_err(|error| {
            staged_uncertain(
                "copy-cleanup",
                format!("failed to remove partial destination: {error}"),
            )
        })?;
        sync_directory(&self.destination_parent, &self.destination_path).map_err(|error| {
            staged_uncertain(
                "copy-cleanup-sync",
                format!("removed partial destination but failed to sync its parent: {error}"),
            )
        })
    }
}

impl FileCopyCursor {
    pub(crate) fn advance_quantum(
        &mut self,
        check_control: &dyn Fn() -> Result<(), StorageError>,
    ) -> Result<bool, StorageError> {
        match self.advance_quantum_inner(check_control) {
            Ok(complete) => Ok(complete),
            Err(error) if self.cleanup_on_error => Err(self.cleanup_partial_destination(error)),
            Err(error) => Err(error),
        }
    }

    fn advance_quantum_inner(
        &mut self,
        check_control: &dyn Fn() -> Result<(), StorageError>,
    ) -> Result<bool, StorageError> {
        check_control()?;
        let mut buffer = [0u8; SECURE_COPY_QUANTUM_BYTES];
        match self.phase {
            FileCopyPhase::Copy => {
                let read = self
                    .source_copy
                    .as_mut()
                    .expect("copy phase retains source descriptor")
                    .read(&mut buffer)
                    .map_err(|error| {
                        StorageError::io(self.source_path.display().to_string(), error)
                    })?;
                if read != 0 {
                    let next = self
                        .copied_bytes
                        .checked_add(read as u64)
                        .ok_or_else(|| staged("copy-size", "copied byte count overflow"))?;
                    if self.expected_bytes != u64::MAX && next > self.expected_bytes {
                        return Err(short_copy_io(
                            &self.destination_path,
                            self.expected_bytes,
                            next,
                        ));
                    }
                    self.destination_copy
                        .as_mut()
                        .expect("copy phase retains destination descriptor")
                        .write_all(&buffer[..read])
                        .map_err(|error| {
                            StorageError::io(self.destination_path.display().to_string(), error)
                        })?;
                    self.copied_bytes = next;
                    return Ok(false);
                }

                if self.expected_bytes != u64::MAX && self.copied_bytes != self.expected_bytes {
                    return Err(short_copy_io(
                        &self.destination_path,
                        self.expected_bytes,
                        self.copied_bytes,
                    ));
                }
                self.destination_copy
                    .as_ref()
                    .expect("copy phase retains destination descriptor")
                    .sync_all()
                    .map_err(|error| {
                        StorageError::io(self.destination_path.display().to_string(), error)
                    })?;
                self.source_copy.take();
                self.destination_copy.take();
                if self.copy_only {
                    if self.sync_directory_after_copy {
                        sync_directory(&self.destination_parent, &self.destination_path)?;
                    }
                    return Ok(true);
                }
                self.source_verify = Some(open_file_at(
                    &self.source_parent,
                    &self.source_name,
                    &self.source_path,
                    libc::O_RDONLY | libc::O_NONBLOCK,
                )?);
                self.destination_verify = Some(open_file_at(
                    &self.destination_parent,
                    &self.destination_name,
                    &self.destination_path,
                    libc::O_RDONLY | libc::O_NONBLOCK,
                )?);
                self.phase = FileCopyPhase::HashSource;
                Ok(false)
            }
            FileCopyPhase::HashSource => {
                let read = self
                    .source_verify
                    .as_mut()
                    .expect("source hash phase retains source descriptor")
                    .read(&mut buffer)
                    .map_err(|error| {
                        StorageError::io(self.source_path.display().to_string(), error)
                    })?;
                if read == 0 {
                    self.phase = FileCopyPhase::HashDestination;
                } else {
                    self.source_hash.update(&buffer[..read]);
                }
                Ok(false)
            }
            FileCopyPhase::HashDestination => {
                let read = self
                    .destination_verify
                    .as_mut()
                    .expect("destination hash phase retains destination descriptor")
                    .read(&mut buffer)
                    .map_err(|error| {
                        StorageError::io(self.destination_path.display().to_string(), error)
                    })?;
                if read != 0 {
                    self.destination_hash.update(&buffer[..read]);
                    return Ok(false);
                }

                if self.source_hash.clone().finalize() != self.destination_hash.clone().finalize() {
                    return Err(staged(
                        "verify-content",
                        format!(
                            "content hash mismatch after copy: {} != {}",
                            self.source_path.display(),
                            self.destination_path.display()
                        ),
                    ));
                }
                if self.sync_directory_after_verify {
                    sync_directory(&self.destination_parent, &self.destination_path)?;
                }
                Ok(true)
            }
        }
    }

    fn cleanup_partial_destination(&mut self, error: StorageError) -> StorageError {
        match self.discard_partial_destination() {
            Ok(()) => error,
            Err(cleanup_error) => staged_uncertain(
                "copy-cleanup",
                format!("{error}; failed to clean partial destination: {cleanup_error}"),
            ),
        }
    }

    pub(crate) fn discard_partial_destination(&mut self) -> Result<(), StorageError> {
        self.source_copy.take();
        self.destination_copy.take();
        self.source_verify.take();
        self.destination_verify.take();
        let no_control = || Ok(());
        remove_entry_at(
            &self.destination_parent,
            &self.destination_name,
            &self.destination_path,
            true,
            &no_control,
        )
        .map_err(|error| {
            staged_uncertain(
                "copy-cleanup",
                format!(
                    "failed to remove partial destination {}: {error}",
                    self.destination_path.display()
                ),
            )
        })?;
        sync_directory(&self.destination_parent, &self.destination_path).map_err(|error| {
            staged_uncertain(
                "copy-cleanup-sync",
                format!("removed partial destination but failed to sync its parent: {error}"),
            )
        })
    }
}

fn short_copy_io(path: &Path, expected: u64, actual: u64) -> StorageError {
    StorageError::ShortIo {
        path: path.display().to_string(),
        expected: expected.min(usize::MAX as u64) as usize,
        actual: actual.min(usize::MAX as u64) as usize,
    }
}

#[allow(clippy::too_many_arguments)]
fn secure_copy_entry(
    source_parent: &File,
    source_name: &OsString,
    destination_parent: &File,
    destination_name: &OsString,
    paths: (&Path, &Path, usize),
    expected_bytes: u64,
    verify_content: bool,
    check_control: &dyn Fn() -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    let mut destination_created = false;
    let result = secure_copy_entry_inner(
        source_parent,
        source_name,
        destination_parent,
        destination_name,
        paths,
        expected_bytes,
        verify_content,
        check_control,
        &mut destination_created,
    );
    if let Err(error) = result {
        if destination_created {
            // Cleanup is deliberately not interruptible. The copy may have
            // been cancelled or failed after creating a partial destination;
            // returning without removing it would make a retry look like a
            // conflicting operator-created destination.
            let no_control = || Ok(());
            if let Err(cleanup_error) = remove_entry_at(
                destination_parent,
                destination_name,
                paths.1,
                true,
                &no_control,
            ) {
                return Err(staged_uncertain(
                    "copy-cleanup",
                    format!(
                        "{error}; failed to remove partial destination {}: {cleanup_error}",
                        paths.1.display()
                    ),
                ));
            }
            if let Err(sync_error) = sync_directory(destination_parent, paths.1) {
                return Err(staged_uncertain(
                    "copy-cleanup-sync",
                    format!(
                        "{error}; removed partial destination but failed to sync its parent: {sync_error}"
                    ),
                ));
            }
            return Err(error);
        }
        return Err(error);
    }
    sync_directory(destination_parent, paths.1)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn secure_copy_entry_inner(
    source_parent: &File,
    source_name: &OsString,
    destination_parent: &File,
    destination_name: &OsString,
    paths: (&Path, &Path, usize),
    expected_bytes: u64,
    verify_content: bool,
    check_control: &dyn Fn() -> Result<(), StorageError>,
    destination_created: &mut bool,
) -> Result<(), StorageError> {
    let (source_path, destination_path, depth) = paths;
    check_control()?;
    let source_type = entry_type(source_parent, source_name, source_path)?;
    if source_type.is_symlink() {
        return Err(unsafe_symlink(source_path, "copy-source"));
    }
    if source_type.is_dir() {
        ensure_storage_tree_depth(depth, source_path, "copy-tree-depth")?;
    }
    ensure_destination_available(destination_parent, destination_name, destination_path)?;

    if source_type.is_dir() {
        mkdir_at(destination_parent, destination_name, destination_path)?;
        *destination_created = true;
        let source_dir = open_dir_at(source_parent, source_name, source_path)
            .map_err(|error| StorageError::io(source_path.display().to_string(), error))?;
        let destination_dir =
            open_dir_at(destination_parent, destination_name, destination_path)
                .map_err(|error| StorageError::io(destination_path.display().to_string(), error))?;
        copy_directory_contents(
            &source_dir,
            &destination_dir,
            source_path,
            destination_path,
            depth,
            check_control,
        )?;
    } else if source_type.is_file() {
        copy_file_at(
            source_parent,
            source_name,
            destination_parent,
            destination_name,
            source_path,
            destination_path,
            check_control,
            destination_created,
        )?;
    } else {
        return Err(staged(
            "copy-source",
            format!("unsupported file type: {}", source_path.display()),
        ));
    }

    verify_expected_length(
        destination_parent,
        destination_name,
        expected_bytes,
        destination_path,
        check_control,
    )?;
    if !verify_content {
        // A nested entry of a directory copy: the top-level entry verifies
        // the whole tree once, so verifying here too would re-hash this
        // subtree at every ancestor (a leaf d levels deep was hashed d + 1
        // times on each side).
        return Ok(());
    }
    verify_content_at(
        source_parent,
        source_name,
        destination_parent,
        destination_name,
        (source_path, destination_path, depth),
        check_control,
    )
}

fn copy_directory_contents(
    source_dir: &File,
    destination_dir: &File,
    source_path: &Path,
    destination_path: &Path,
    depth: usize,
    check_control: &dyn Fn() -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    visit_directory_names(source_dir, source_path, |name| {
        check_control()?;
        let child_source = source_path.join(&name);
        let child_destination = destination_path.join(&name);
        secure_copy_entry(
            source_dir,
            &name,
            destination_dir,
            &name,
            (&child_source, &child_destination, depth.saturating_add(1)),
            u64::MAX,
            false,
            check_control,
        )?;
        Ok(true)
    })?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn copy_file_at(
    source_parent: &File,
    source_name: &OsString,
    destination_parent: &File,
    destination_name: &OsString,
    source_path: &Path,
    destination_path: &Path,
    check_control: &dyn Fn() -> Result<(), StorageError>,
    destination_created: &mut bool,
) -> Result<(), StorageError> {
    check_control()?;
    let mut source = open_file_at(source_parent, source_name, source_path, libc::O_RDONLY)?;
    let mut destination = create_file_at(destination_parent, destination_name, destination_path)?;
    *destination_created = true;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        check_control()?;
        let read = source
            .read(&mut buffer)
            .map_err(|error| StorageError::io(source_path.display().to_string(), error))?;
        if read == 0 {
            break;
        }
        destination
            .write_all(&buffer[..read])
            .map_err(|error| StorageError::io(destination_path.display().to_string(), error))?;
    }
    destination
        .sync_all()
        .map_err(|error| StorageError::io(destination_path.display().to_string(), error))
}

fn secure_rename(
    source: &Path,
    destination: &Path,
    expected_bytes: u64,
    roots: &[PathBuf],
    check_control: &dyn Fn() -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    check_control()?;
    let _rename_guard = rename_lock_shard(destination)
        .lock()
        .map_err(|_| staged("rename", "rename serialization lock poisoned"))?;
    let (source_parent, source_name) = open_parent(source, roots, false, "rename-source")?;
    let (destination_parent, destination_name) =
        open_parent(destination, roots, true, "rename-destination")?;
    ensure_destination_available(&destination_parent, &destination_name, destination)?;
    if entry_type(&source_parent, &source_name, source)?.is_symlink() {
        return Err(unsafe_symlink(source, "rename-source"));
    }
    // Check the source before the destructive rename. A stale expected size
    // must fail without leaving the source stranded at the destination.
    verify_expected_length(
        &source_parent,
        &source_name,
        expected_bytes,
        source,
        check_control,
    )?;
    check_control()?;
    let source_c = c_name(&source_name, source)?;
    let destination_c = c_name(&destination_name, destination)?;
    if let Err(error) = rename_without_replace(
        &source_parent,
        &source_c,
        &destination_parent,
        &destination_c,
    ) {
        return Err(StorageError::io(destination.display().to_string(), error));
    }
    if let Err(error) = sync_directory(&source_parent, source) {
        return Err(staged_uncertain(
            "rename-sync",
            format!(
                "rename committed but failed to sync source directory {}: {error}",
                source.display()
            ),
        ));
    }
    if let Err(error) = sync_directory(&destination_parent, destination) {
        return Err(staged_uncertain(
            "rename-sync",
            format!(
                "rename committed but failed to sync destination directory {}: {error}",
                destination.display()
            ),
        ));
    }
    if let Err(error) = verify_expected_length(
        &destination_parent,
        &destination_name,
        expected_bytes,
        destination,
        check_control,
    ) {
        // The source has disappeared by this point. Restore it before
        // returning a verification failure, using the same no-replace
        // primitive so a concurrently-created source cannot be overwritten.
        if rename_without_replace(
            &destination_parent,
            &destination_c,
            &source_parent,
            &source_c,
        )
        .is_ok()
        {
            if let Err(sync_error) = sync_directory(&source_parent, source)
                .and_then(|_| sync_directory(&destination_parent, destination))
            {
                return Err(staged_uncertain(
                    "rename-rollback-sync",
                    format!(
                        "{error}; restored {} but failed to sync the rollback: {sync_error}",
                        source.display()
                    ),
                ));
            }
            return Err(error);
        }
        return Err(staged_uncertain(
            "rename-verify",
            format!(
                "{error}; failed to restore {} after verification failure",
                source.display()
            ),
        ));
    }
    Ok(())
}

#[cfg(all(any(target_os = "linux", target_os = "android"), target_env = "musl"))]
fn rename_without_replace(
    source_parent: &File,
    source_name: &CString,
    destination_parent: &File,
    destination_name: &CString,
) -> io::Result<()> {
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            source_parent.as_raw_fd(),
            source_name.as_ptr(),
            destination_parent.as_raw_fd(),
            destination_name.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(all(
    any(target_os = "linux", target_os = "android"),
    not(target_env = "musl")
))]
fn rename_without_replace(
    source_parent: &File,
    source_name: &CString,
    destination_parent: &File,
    destination_name: &CString,
) -> io::Result<()> {
    let result = unsafe {
        libc::renameat2(
            source_parent.as_raw_fd(),
            source_name.as_ptr(),
            destination_parent.as_raw_fd(),
            destination_name.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "macos")]
fn rename_without_replace(
    source_parent: &File,
    source_name: &CString,
    destination_parent: &File,
    destination_name: &CString,
) -> io::Result<()> {
    let result = unsafe {
        libc::renameatx_np(
            source_parent.as_raw_fd(),
            source_name.as_ptr(),
            destination_parent.as_raw_fd(),
            destination_name.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "android", target_os = "macos"))
))]
fn rename_without_replace(
    _source_parent: &File,
    _source_name: &CString,
    _destination_parent: &File,
    _destination_name: &CString,
) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic no-replace rename is unavailable on this Unix platform",
    ))
}

fn secure_delete(
    path: &Path,
    roots: &[PathBuf],
    missing_ok: bool,
    check_control: &dyn Fn() -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    check_control()?;
    let (parent, name) = match open_parent(path, roots, false, "delete-source") {
        Ok(value) => value,
        Err(error) if missing_ok && is_not_found(&error) => return Ok(()),
        Err(error) => return Err(error),
    };
    remove_entry_at(&parent, &name, path, missing_ok, check_control)
}

fn remove_entry_at(
    parent: &File,
    name: &OsString,
    path: &Path,
    missing_ok: bool,
    check_control: &dyn Fn() -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    let mut removed = false;
    match remove_entry_at_inner(
        parent,
        name,
        path,
        missing_ok,
        check_control,
        &mut removed,
        0,
    ) {
        Err(error) if removed => Err(staged_uncertain(
            "delete",
            format!(
                "{error}; deletion of {} was only partially applied",
                path.display()
            ),
        )),
        result => result,
    }
}

fn remove_entry_at_inner(
    parent: &File,
    name: &OsString,
    path: &Path,
    missing_ok: bool,
    check_control: &dyn Fn() -> Result<(), StorageError>,
    removed: &mut bool,
    depth: usize,
) -> Result<(), StorageError> {
    check_control()?;
    let kind = match entry_type(parent, name, path) {
        Ok(kind) => kind,
        Err(error) if missing_ok && is_not_found(&error) => return Ok(()),
        Err(error) => return Err(error),
    };
    if kind.is_dir() {
        ensure_storage_tree_depth(depth, path, "delete-tree-depth")?;
        let child_dir = open_dir_at(parent, name, path)
            .map_err(|error| StorageError::io(path.display().to_string(), error))?;
        loop {
            let children = directory_names_batch(&child_dir, path)?;
            if children.is_empty() {
                break;
            }
            for child in children {
                remove_entry_at_inner(
                    &child_dir,
                    &child,
                    &path.join(&child),
                    missing_ok,
                    check_control,
                    removed,
                    depth.saturating_add(1),
                )?;
            }
        }
        check_control()?;
        let result = unlink_at(parent, name, path, libc::AT_REMOVEDIR);
        if result.is_ok() {
            *removed = true;
            sync_directory(parent, path)?;
        }
        result
    } else if kind.is_file() || kind.is_symlink() {
        check_control()?;
        let result = unlink_at(parent, name, path, 0);
        if result.is_ok() {
            *removed = true;
            sync_directory(parent, path)?;
        }
        result
    } else {
        Err(staged(
            "delete-source",
            format!("unsupported file type: {}", path.display()),
        ))
    }
}

fn secure_prune(
    source: &Path,
    boundary: &Path,
    roots: &[PathBuf],
    check_control: &dyn Fn() -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    check_control()?;
    let (root_path, source_parts) = relative_parts(source, roots, "prune-source")?;
    let (boundary_root, boundary_parts) = relative_parts_allow_root(boundary, roots, "prune-root")?;
    if root_path != boundary_root
        || boundary_parts.len() > source_parts.len()
        || boundary_parts != source_parts[..boundary_parts.len()]
    {
        return Err(staged(
            "prune-root",
            format!(
                "directory {} is outside prune root {}",
                source.display(),
                boundary.display()
            ),
        ));
    }
    if source_parts.len() == boundary_parts.len() {
        return Ok(());
    }

    let root_dir = open_root(&root_path)
        .map_err(|error| StorageError::io(root_path.display().to_string(), error))?;
    let mut directories = vec![root_dir];
    for part in &source_parts {
        let parent = directories.last().expect("root directory exists");
        let directory = match open_dir_at(parent, part, source) {
            Ok(directory) => directory,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(StorageError::io(source.display().to_string(), error)),
        };
        directories.push(directory);
    }

    let mut removed = false;
    for index in (boundary_parts.len()..source_parts.len()).rev() {
        if let Err(error) = check_control() {
            if removed {
                return Err(staged_uncertain(
                    "prune",
                    format!(
                        "{error}; directory pruning of {} was only partially applied",
                        source.display()
                    ),
                ));
            }
            return Err(error);
        }
        match unlink_at(
            &directories[index],
            &source_parts[index],
            source,
            libc::AT_REMOVEDIR,
        ) {
            Ok(()) => {
                removed = true;
                if let Err(error) = sync_directory(&directories[index], source) {
                    return Err(staged_uncertain(
                        "prune-sync",
                        format!(
                            "directory pruning committed but failed to sync {}: {error}",
                            source.display()
                        ),
                    ));
                }
            }
            Err(error) if is_not_found(&error) => {}
            Err(StorageError::Io { source: error, .. })
                if error.kind() == io::ErrorKind::DirectoryNotEmpty =>
            {
                break
            }
            Err(StorageError::Io { source: error, .. })
                if error.raw_os_error() == Some(libc::ENOTEMPTY) =>
            {
                break
            }
            Err(error) => {
                if removed {
                    return Err(staged_uncertain(
                        "prune",
                        format!(
                            "{error}; directory pruning of {} was only partially applied",
                            source.display()
                        ),
                    ));
                }
                return Err(error);
            }
        }
    }
    Ok(())
}

fn verify_content(
    source: &Path,
    destination: &Path,
    roots: &[PathBuf],
) -> Result<(), StorageError> {
    let no_control = || Ok(());
    verify_content_with_control(source, destination, roots, &no_control)
}

fn verify_content_with_control(
    source: &Path,
    destination: &Path,
    roots: &[PathBuf],
    check_control: &dyn Fn() -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    check_control()?;
    let (source_parent, source_name) = open_parent(source, roots, false, "verify-content-source")?;
    let (destination_parent, destination_name) =
        open_parent(destination, roots, false, "verify-content-destination")?;
    verify_content_at(
        &source_parent,
        &source_name,
        &destination_parent,
        &destination_name,
        (source, destination, 0),
        check_control,
    )
}

fn verify_content_at(
    source_parent: &File,
    source_name: &OsString,
    destination_parent: &File,
    destination_name: &OsString,
    paths: (&Path, &Path, usize),
    check_control: &dyn Fn() -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    let (source_path, destination_path, depth) = paths;
    check_control()?;
    let source_type = entry_type(source_parent, source_name, source_path)?;
    let destination_type = entry_type(destination_parent, destination_name, destination_path)?;
    if source_type.is_symlink() {
        return Err(unsafe_symlink(source_path, "verify-content-source"));
    }
    if destination_type.is_symlink() {
        return Err(unsafe_symlink(
            destination_path,
            "verify-content-destination",
        ));
    }
    if source_type.is_dir() {
        ensure_storage_tree_depth(depth, source_path, "verify-tree-depth")?;
        if !destination_type.is_dir() {
            return Err(staged(
                "verify-content",
                format!(
                    "expected a directory at {} to match source directory {}",
                    destination_path.display(),
                    source_path.display()
                ),
            ));
        }
        let source_dir = open_dir_at(source_parent, source_name, source_path)
            .map_err(|error| StorageError::io(source_path.display().to_string(), error))?;
        let destination_dir =
            open_dir_at(destination_parent, destination_name, destination_path)
                .map_err(|error| StorageError::io(destination_path.display().to_string(), error))?;
        visit_directory_names(&source_dir, source_path, |name| {
            check_control()?;
            let child_source = source_path.join(&name);
            let child_destination = destination_path.join(&name);
            verify_content_at(
                &source_dir,
                &name,
                &destination_dir,
                &name,
                (&child_source, &child_destination, depth.saturating_add(1)),
                check_control,
            )?;
            Ok(true)
        })?;
        visit_directory_names(&destination_dir, destination_path, |name| {
            check_control()?;
            let child_source = source_path.join(&name);
            let child_destination = destination_path.join(&name);
            let destination_type = entry_type(&destination_dir, &name, &child_destination)?;
            if destination_type.is_symlink() {
                return Err(unsafe_symlink(
                    &child_destination,
                    "verify-content-destination",
                ));
            }
            let source_type = match entry_type(&source_dir, &name, &child_source) {
                Ok(kind) => kind,
                Err(error) if is_not_found(&error) => {
                    return Err(staged(
                        "verify-content",
                        format!(
                            "destination directory has entries absent from source: {}",
                            destination_path.display()
                        ),
                    ));
                }
                Err(error) => return Err(error),
            };
            if source_type.is_symlink() {
                return Err(unsafe_symlink(&child_source, "verify-content-source"));
            }
            if !source_type.is_file() && !source_type.is_dir() {
                return Err(staged(
                    "verify-content",
                    format!("unsupported source entry type: {}", child_source.display()),
                ));
            }
            Ok(true)
        })?;
        Ok(())
    } else if source_type.is_file() {
        if !destination_type.is_file() {
            return Err(staged(
                "verify-content",
                format!(
                    "expected a regular file at {} to match source file {}",
                    destination_path.display(),
                    source_path.display()
                ),
            ));
        }
        let source_hash = hash_file_at(source_parent, source_name, source_path, check_control)?;
        let destination_hash = hash_file_at(
            destination_parent,
            destination_name,
            destination_path,
            check_control,
        )?;
        if source_hash == destination_hash {
            Ok(())
        } else {
            Err(staged(
                "verify-content",
                format!(
                    "content hash mismatch after copy: {} != {}",
                    source_path.display(),
                    destination_path.display()
                ),
            ))
        }
    } else {
        Err(staged(
            "verify-content",
            format!("unsupported file type: {}", source_path.display()),
        ))
    }
}

#[cfg(test)]
thread_local! {
    /// Files hashed on this thread, for copy-verification cost tests.
    static HASHED_FILES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn hash_file_at(
    parent: &File,
    name: &OsString,
    path: &Path,
    check_control: &dyn Fn() -> Result<(), StorageError>,
) -> Result<[u8; 20], StorageError> {
    check_control()?;
    #[cfg(test)]
    HASHED_FILES.with(|count| count.set(count.get() + 1));
    let mut file = open_file_at(parent, name, path, libc::O_RDONLY)?;
    let mut hasher = Sha1::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        check_control()?;
        let read = file
            .read(&mut buffer)
            .map_err(|error| StorageError::io(path.display().to_string(), error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().into())
}

fn verify_expected_length(
    parent: &File,
    name: &OsString,
    expected_bytes: u64,
    path: &Path,
    check_control: &dyn Fn() -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    if expected_bytes == u64::MAX {
        return Ok(());
    }
    check_control()?;
    let actual = content_len_at(parent, name, path, 0, check_control)?;
    if actual == expected_bytes {
        Ok(())
    } else {
        Err(StorageError::ShortIo {
            path: path.display().to_string(),
            expected: expected_bytes.min(usize::MAX as u64) as usize,
            actual: actual.min(usize::MAX as u64) as usize,
        })
    }
}

fn content_len_at(
    parent: &File,
    name: &OsString,
    path: &Path,
    depth: usize,
    check_control: &dyn Fn() -> Result<(), StorageError>,
) -> Result<u64, StorageError> {
    check_control()?;
    let kind = entry_type(parent, name, path)?;
    if kind.is_file() {
        let file = open_file_at(parent, name, path, libc::O_RDONLY)?;
        return file
            .metadata()
            .map(|metadata| metadata.len())
            .map_err(|error| StorageError::io(path.display().to_string(), error));
    }
    if kind.is_dir() {
        ensure_storage_tree_depth(depth, path, "verify-tree-depth")?;
        let directory = open_dir_at(parent, name, path)
            .map_err(|error| StorageError::io(path.display().to_string(), error))?;
        let mut total = 0u64;
        visit_directory_names(&directory, path, |child| {
            check_control()?;
            total = total.saturating_add(content_len_at(
                &directory,
                &child,
                &path.join(&child),
                depth.saturating_add(1),
                check_control,
            )?);
            Ok(true)
        })?;
        return Ok(total);
    }
    Err(staged(
        "verify",
        format!("unsupported file type: {}", path.display()),
    ))
}

fn relative_parts(
    path: &Path,
    roots: &[PathBuf],
    step: &'static str,
) -> Result<(PathBuf, Vec<OsString>), StorageError> {
    relative_parts_inner(path, roots, step, false)
}

fn relative_parts_allow_root(
    path: &Path,
    roots: &[PathBuf],
    step: &'static str,
) -> Result<(PathBuf, Vec<OsString>), StorageError> {
    relative_parts_inner(path, roots, step, true)
}

fn relative_parts_inner(
    path: &Path,
    roots: &[PathBuf],
    step: &'static str,
    allow_root: bool,
) -> Result<(PathBuf, Vec<OsString>), StorageError> {
    let root = roots
        .iter()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.components().count())
        .ok_or_else(|| {
            staged(
                step,
                format!("path outside configured roots: {}", path.display()),
            )
        })?;
    let relative = path.strip_prefix(root).map_err(|_| {
        staged(
            step,
            format!("path outside configured roots: {}", path.display()),
        )
    })?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(value) => parts.push(value.to_os_string()),
            _ => {
                return Err(staged(
                    step,
                    format!("unsafe path component: {}", path.display()),
                ));
            }
        }
    }
    if parts.is_empty() && !allow_root {
        return Err(staged(
            step,
            format!("refusing to operate on storage root: {}", path.display()),
        ));
    }
    Ok((root.clone(), parts))
}

fn open_parent(
    path: &Path,
    roots: &[PathBuf],
    create: bool,
    step: &'static str,
) -> Result<(File, OsString), StorageError> {
    let (root_path, parts) = relative_parts(path, roots, step)?;
    let name = parts.last().expect("relative path is non-empty").clone();
    let root = open_root(&root_path)
        .map_err(|error| StorageError::io(root_path.display().to_string(), error))?;
    let mut parent = root;
    for part in &parts[..parts.len() - 1] {
        parent = open_or_create_dir(&parent, part, create, path)?;
    }
    Ok((parent, name))
}

fn open_root(path: &Path) -> io::Result<File> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "storage root must be absolute",
        ));
    }
    let name = CString::new("/").expect("literal has no NUL");
    let fd = unsafe {
        libc::open(
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut root = unsafe { File::from_raw_fd(fd) };
    for component in path.components() {
        let Component::Normal(value) = component else {
            if matches!(component, Component::RootDir) {
                continue;
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "storage root contains an unsafe component",
            ));
        };
        let name = value.to_os_string();
        root = open_dir_at(&root, &name, path)?;
    }
    Ok(root)
}

fn open_or_create_dir(
    parent: &File,
    name: &OsString,
    create: bool,
    path: &Path,
) -> Result<File, StorageError> {
    match open_dir_at(parent, name, path) {
        Ok(directory) => Ok(directory),
        Err(error) if create && error.kind() == io::ErrorKind::NotFound => {
            let c_name = c_name(name, path)?;
            let result = unsafe { libc::mkdirat(parent.as_raw_fd(), c_name.as_ptr(), 0o755) };
            if result != 0 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::AlreadyExists {
                    return Err(StorageError::io(path.display().to_string(), error));
                }
            }
            open_dir_at(parent, name, path)
                .map_err(|error| StorageError::io(path.display().to_string(), error))
        }
        Err(error) => Err(StorageError::io(path.display().to_string(), error)),
    }
}

fn open_dir_at(parent: &File, name: &OsString, _path: &Path) -> io::Result<File> {
    let name = CString::new(name.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

fn open_file_at(
    parent: &File,
    name: &OsString,
    path: &Path,
    flags: i32,
) -> Result<File, StorageError> {
    let name = c_name(name, path)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        Err(StorageError::io(
            path.display().to_string(),
            io::Error::last_os_error(),
        ))
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

fn create_file_at(parent: &File, name: &OsString, path: &Path) -> Result<File, StorageError> {
    let name = c_name(name, path)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o644,
        )
    };
    if fd < 0 {
        Err(StorageError::io(
            path.display().to_string(),
            io::Error::last_os_error(),
        ))
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

fn mkdir_at(parent: &File, name: &OsString, path: &Path) -> Result<(), StorageError> {
    let name = c_name(name, path)?;
    let result = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o755) };
    if result == 0 {
        Ok(())
    } else {
        Err(StorageError::io(
            path.display().to_string(),
            io::Error::last_os_error(),
        ))
    }
}

fn unlink_at(parent: &File, name: &OsString, path: &Path, flags: i32) -> Result<(), StorageError> {
    let name = c_name(name, path)?;
    let result = unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if result == 0 {
        Ok(())
    } else {
        Err(StorageError::io(
            path.display().to_string(),
            io::Error::last_os_error(),
        ))
    }
}

fn entry_state(path: &Path, roots: &[PathBuf]) -> Result<Option<EntryType>, StorageError> {
    let (parent, name) = match open_parent(path, roots, false, "reconcile") {
        Ok(value) => value,
        Err(error) if is_not_found(&error) => return Ok(None),
        Err(error) => return Err(error),
    };
    match entry_type(&parent, &name, path) {
        Ok(kind) => Ok(Some(kind)),
        Err(error) if is_not_found(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

fn entry_type(parent: &File, name: &OsString, path: &Path) -> Result<EntryType, StorageError> {
    let mut stat = unsafe { std::mem::zeroed::<libc::stat>() };
    let name = c_name(name, path)?;
    let result = unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            &mut stat,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result != 0 {
        return Err(StorageError::io(
            path.display().to_string(),
            io::Error::last_os_error(),
        ));
    }
    Ok(EntryType::from_mode(stat.st_mode))
}

#[derive(Debug, Clone, Copy)]
struct EntryType(u32);

#[cfg(target_os = "linux")]
const ENTRY_MODE_DIR: u32 = libc::S_IFDIR;
#[cfg(not(target_os = "linux"))]
const ENTRY_MODE_DIR: u32 = libc::S_IFDIR as u32;

#[cfg(target_os = "linux")]
const ENTRY_MODE_FILE: u32 = libc::S_IFREG;
#[cfg(not(target_os = "linux"))]
const ENTRY_MODE_FILE: u32 = libc::S_IFREG as u32;

#[cfg(target_os = "linux")]
const ENTRY_MODE_SYMLINK: u32 = libc::S_IFLNK;
#[cfg(not(target_os = "linux"))]
const ENTRY_MODE_SYMLINK: u32 = libc::S_IFLNK as u32;

impl EntryType {
    fn from_mode(mode: libc::mode_t) -> Self {
        #[cfg(target_os = "linux")]
        {
            Self(mode & libc::S_IFMT)
        }

        #[cfg(not(target_os = "linux"))]
        {
            Self((mode & libc::S_IFMT) as u32)
        }
    }

    fn is_dir(self) -> bool {
        self.0 == ENTRY_MODE_DIR
    }

    fn is_file(self) -> bool {
        self.0 == ENTRY_MODE_FILE
    }

    fn is_symlink(self) -> bool {
        self.0 == ENTRY_MODE_SYMLINK
    }
}

fn directory_names_batch(directory: &File, path: &Path) -> Result<Vec<OsString>, StorageError> {
    let mut names = Vec::with_capacity(DIRECTORY_DELETE_BATCH_ENTRIES);
    visit_directory_names(directory, path, |name| {
        names.push(name);
        Ok(names.len() < DIRECTORY_DELETE_BATCH_ENTRIES)
    })?;
    Ok(names)
}

/// Visit names through a separate open directory description so recursive
/// callbacks can use the pinned directory fd without sharing its read offset.
/// Returning `false` stops after the current entry and is used by bounded
/// deletion batches.
fn visit_directory_names<F>(directory: &File, path: &Path, mut visit: F) -> Result<(), StorageError>
where
    F: FnMut(OsString) -> Result<bool, StorageError>,
{
    let stream_file = open_dir_at(directory, &OsString::from("."), path)
        .map_err(|error| StorageError::io(path.display().to_string(), error))?;
    let stream_fd = stream_file.into_raw_fd();
    let directory_stream = unsafe { libc::fdopendir(stream_fd) };
    if directory_stream.is_null() {
        unsafe { libc::close(stream_fd) };
        return Err(StorageError::io(
            path.display().to_string(),
            io::Error::last_os_error(),
        ));
    }

    let mut visit_error = None;
    let mut read_error = None;
    loop {
        // The callback can perform filesystem operations that change errno,
        // so reset it before every readdir call rather than only once.
        clear_errno();
        let entry = unsafe { libc::readdir(directory_stream) };
        if entry.is_null() {
            let error = io::Error::last_os_error();
            if error.raw_os_error().unwrap_or_default() != 0 {
                read_error = Some(error);
            }
            break;
        }
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if name == b"." || name == b".." {
            continue;
        }
        match visit(OsString::from_vec(name.to_vec())) {
            Ok(true) => {}
            Ok(false) => break,
            Err(error) => {
                visit_error = Some(error);
                break;
            }
        }
    }
    let close_result = unsafe { libc::closedir(directory_stream) };
    if let Some(error) = visit_error {
        return Err(error);
    }
    if let Some(error) = read_error {
        return Err(StorageError::io(path.display().to_string(), error));
    }
    if close_result != 0 {
        return Err(StorageError::io(
            path.display().to_string(),
            io::Error::last_os_error(),
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "hurd", target_os = "redox"))]
fn clear_errno() {
    unsafe { *libc::__errno_location() = 0 };
}

#[cfg(target_os = "android")]
fn clear_errno() {
    unsafe { *libc::__errno() = 0 };
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos",
    target_os = "freebsd"
))]
fn clear_errno() {
    unsafe { *libc::__error() = 0 };
}

#[cfg(any(target_os = "openbsd", target_os = "netbsd"))]
fn clear_errno() {
    unsafe { *libc::__errno() = 0 };
}

#[cfg(any(target_os = "solaris", target_os = "illumos"))]
fn clear_errno() {
    unsafe { *libc::___errno() = 0 };
}

#[cfg(target_os = "dragonfly")]
fn clear_errno() {
    unsafe { *libc::__errno_location() = 0 };
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "hurd",
        target_os = "redox",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "watchos",
        target_os = "visionos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "solaris",
        target_os = "illumos",
        target_os = "dragonfly"
    ))
))]
fn clear_errno() {
    // Keep the Unix fallback buildable on targets whose libc errno accessor
    // is not exposed by the `libc` crate. The supported targets above use a
    // real reset before `readdir`.
}

fn ensure_destination_available(
    parent: &File,
    name: &OsString,
    path: &Path,
) -> Result<(), StorageError> {
    match entry_type(parent, name, path) {
        Ok(_) => Err(staged(
            "destination",
            format!("destination exists: {}", path.display()),
        )),
        Err(error) if is_not_found(&error) => Ok(()),
        Err(error) => Err(error),
    }
}

fn required_path<'a>(
    path: Option<&'a PathBuf>,
    step: &'static str,
) -> Result<&'a Path, StorageError> {
    path.map(PathBuf::as_path)
        .ok_or_else(|| staged(step, "missing path"))
}

fn c_name(name: &OsString, path: &Path) -> Result<CString, StorageError> {
    CString::new(name.as_bytes())
        .map_err(|_| staged("path", format!("path contains NUL: {}", path.display())))
}

fn staged(step: &'static str, reason: impl Into<String>) -> StorageError {
    StorageError::StagedMoveFailed {
        step,
        reason: reason.into(),
    }
}

fn staged_uncertain(step: &'static str, reason: impl Into<String>) -> StorageError {
    StorageError::FilesystemStateUncertain {
        step,
        reason: reason.into(),
    }
}

fn unsafe_symlink(path: &Path, step: &'static str) -> StorageError {
    staged(
        step,
        format!(
            "symlink entries are not allowed in move/import plans: {}",
            path.display()
        ),
    )
}

fn is_not_found(error: &StorageError) -> bool {
    match error {
        StorageError::FileNotFound { .. } => true,
        StorageError::Io { source, .. } => source.kind() == io::ErrorKind::NotFound,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_overdeep_directory_tree(root: &Path) {
        std::fs::create_dir(root).unwrap();
        let mut current = root.to_path_buf();
        for _ in 0..=crate::plan::MAX_STORAGE_TREE_DEPTH {
            current.push("nested");
            std::fs::create_dir(&current).unwrap();
        }
    }

    #[test]
    fn directory_name_stream_visits_all_entries() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("file"), b"data").unwrap();
        std::fs::create_dir(root.path().join("directory")).unwrap();

        let directory = File::open(root.path()).unwrap();
        let mut names = Vec::new();
        visit_directory_names(&directory, root.path(), |name| {
            names.push(name);
            Ok(true)
        })
        .unwrap();
        names.sort();

        assert_eq!(
            names,
            vec![OsString::from("directory"), OsString::from("file")]
        );
    }

    #[test]
    fn directory_deletion_batches_bound_retained_names() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..=DIRECTORY_DELETE_BATCH_ENTRIES {
            std::fs::write(root.path().join(format!("file-{index:04}")), b"").unwrap();
        }
        let directory = File::open(root.path()).unwrap();

        let first_batch = directory_names_batch(&directory, root.path()).unwrap();
        assert_eq!(first_batch.len(), DIRECTORY_DELETE_BATCH_ENTRIES);
        for name in first_batch {
            std::fs::remove_file(root.path().join(name)).unwrap();
        }
        assert_eq!(
            directory_names_batch(&directory, root.path())
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn streamed_tree_verification_rejects_destination_only_entries() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let destination = root.path().join("destination");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(source.join("payload.bin"), b"same bytes").unwrap();
        std::fs::write(destination.join("payload.bin"), b"same bytes").unwrap();
        std::fs::write(destination.join("extra.bin"), b"extra").unwrap();

        let error =
            verify_content(&source, &destination, &[root.path().to_path_buf()]).unwrap_err();
        assert!(error
            .to_string()
            .contains("destination directory has entries absent from source"));
    }

    #[test]
    fn overdeep_directory_walks_fail_closed_without_stack_recursion() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        let destination = directory.path().join("destination");
        create_overdeep_directory_tree(&source);
        create_overdeep_directory_tree(&destination);
        let roots = vec![directory.path().to_path_buf()];
        let no_control = || Ok(());

        let verify_error = verify_content(&source, &destination, &roots).unwrap_err();
        assert!(verify_error.to_string().contains("directory nesting depth"));

        let root = File::open(directory.path()).unwrap();
        let content_error =
            content_len_at(&root, &OsString::from("source"), &source, 0, &no_control).unwrap_err();
        assert!(content_error
            .to_string()
            .contains("directory nesting depth"));

        let copy_destination = directory.path().join("copy-destination");
        let copy_error =
            secure_copy(&source, &copy_destination, 0, &roots, &no_control).unwrap_err();
        assert!(copy_error.to_string().contains("directory nesting depth"));
        assert!(!copy_destination.exists(), "partial copy must be removed");

        let delete_error = secure_delete(&source, &roots, false, &no_control).unwrap_err();
        assert!(delete_error.to_string().contains("directory nesting depth"));
        assert!(source.exists(), "rejected deep tree must remain intact");
        assert!(destination.exists());
    }

    #[test]
    fn deep_copy_hashes_each_leaf_once_per_side() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        let mut leaf_dir = source.clone();
        for level in 0..6 {
            leaf_dir.push(format!("d{level}"));
        }
        std::fs::create_dir_all(&leaf_dir).unwrap();
        std::fs::write(leaf_dir.join("payload.bin"), b"payload").unwrap();
        let destination = directory.path().join("destination");
        let roots = vec![directory.path().to_path_buf()];
        let no_control = || Ok(());

        HASHED_FILES.with(|count| count.set(0));
        secure_copy(&source, &destination, 7, &roots, &no_control).unwrap();
        // One source hash and one destination hash, independent of depth.
        assert_eq!(HASHED_FILES.with(|count| count.get()), 2);
        assert_eq!(
            std::fs::read(destination.join("d0/d1/d2/d3/d4/d5/payload.bin")).unwrap(),
            b"payload"
        );
    }

    #[test]
    fn regular_file_copy_cursor_advances_in_bounded_quanta() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.bin");
        let destination = directory.path().join("destination.bin");
        let payload = vec![0x5a; SECURE_COPY_QUANTUM_BYTES * 3 + 17];
        std::fs::write(&source, &payload).unwrap();
        let roots = vec![directory.path().to_path_buf()];
        let no_control = || Ok(());
        let (cursor, complete) = start_file_copy_cursor(
            &source,
            &destination,
            payload.len() as u64,
            &roots,
            &no_control,
            false,
        )
        .unwrap();
        assert!(!complete);
        let mut cursor = cursor.expect("regular files should use the chunked cursor");

        let mut complete = false;
        while !complete {
            let copied_before = cursor.copied_bytes();
            complete = cursor.advance_quantum(&no_control).unwrap();
            let copied_this_quantum = cursor.copied_bytes() - copied_before;
            assert!(copied_this_quantum <= SECURE_COPY_QUANTUM_BYTES as u64);
        }

        assert_eq!(std::fs::read(destination).unwrap(), payload);
    }

    #[test]
    fn regular_file_copy_cursor_cancellation_removes_partial_destination() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.bin");
        let destination = directory.path().join("destination.bin");
        std::fs::write(&source, vec![0x33; SECURE_COPY_QUANTUM_BYTES * 2]).unwrap();
        let roots = vec![directory.path().to_path_buf()];
        let no_control = || Ok(());
        let (cursor, complete) =
            start_file_copy_cursor(&source, &destination, u64::MAX, &roots, &no_control, false)
                .unwrap();
        assert!(!complete);
        let mut cursor = cursor.expect("regular files should use the chunked cursor");
        assert!(!cursor.advance_quantum(&no_control).unwrap());
        assert!(destination.exists());

        let cancelled = || {
            Err(StorageError::StagedMoveFailed {
                step: "cancel",
                reason: "copy cancelled".to_owned(),
            })
        };
        assert!(cursor.advance_quantum(&cancelled).is_err());
        assert!(
            !destination.exists(),
            "cancelled copy leaves no partial file"
        );
    }

    #[test]
    fn directory_copy_cursor_preempts_and_verifies_streamed_tree() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        let nested = source.join("nested/deeper");
        std::fs::create_dir_all(&nested).unwrap();
        let first = vec![0x51; SECURE_COPY_QUANTUM_BYTES * 3 + 19];
        let second = vec![0xa7; SECURE_COPY_QUANTUM_BYTES + 7];
        std::fs::write(source.join("first.bin"), &first).unwrap();
        std::fs::write(nested.join("second.bin"), &second).unwrap();
        let destination = directory.path().join("destination");
        let roots = vec![directory.path().to_path_buf()];
        let no_control = || Ok(());

        let (cursor, already_complete) = start_file_copy_cursor(
            &source,
            &destination,
            (first.len() + second.len()) as u64,
            &roots,
            &no_control,
            false,
        )
        .unwrap();
        assert!(!already_complete);
        let mut cursor = cursor.expect("directory copies use the resumable cursor");
        let mut quanta = 0;
        loop {
            let before = cursor.copied_bytes();
            let complete = cursor.advance_quantum(&no_control).unwrap();
            let advanced = cursor.copied_bytes().saturating_sub(before);
            assert!(advanced <= SECURE_COPY_QUANTUM_BYTES as u64);
            quanta += 1;
            if complete {
                break;
            }
        }

        assert!(
            quanta > 10,
            "copy and verification should yield between bounded steps"
        );
        assert_eq!(std::fs::read(destination.join("first.bin")).unwrap(), first);
        assert_eq!(
            std::fs::read(destination.join("nested/deeper/second.bin")).unwrap(),
            second
        );
    }

    #[test]
    fn directory_copy_cursor_cancellation_removes_partial_tree() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(
            source.join("payload.bin"),
            vec![0x33; SECURE_COPY_QUANTUM_BYTES * 3],
        )
        .unwrap();
        let destination = directory.path().join("destination");
        let roots = vec![directory.path().to_path_buf()];
        let no_control = || Ok(());
        let (cursor, _) =
            start_file_copy_cursor(&source, &destination, u64::MAX, &roots, &no_control, false)
                .unwrap();
        let mut cursor = cursor.expect("directory copies use the resumable cursor");
        assert!(!cursor.advance_quantum(&no_control).unwrap());
        assert!(!cursor.advance_quantum(&no_control).unwrap());
        assert!(destination.join("payload.bin").exists());

        let cancelled = || {
            Err(StorageError::StagedMoveFailed {
                step: "cancel",
                reason: "copy cancelled".to_owned(),
            })
        };
        assert!(cursor.advance_quantum(&cancelled).is_err());
        assert!(
            !destination.exists(),
            "cancelled tree copy leaves no partial tree"
        );
    }

    #[test]
    fn regular_file_import_keeps_the_hard_link_fast_path() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.bin");
        let destination = directory.path().join("destination.bin");
        std::fs::write(&source, b"shared payload").unwrap();
        let roots = vec![directory.path().to_path_buf()];
        let (cursor, complete) =
            start_file_copy_cursor(&source, &destination, 14, &roots, &|| Ok(()), true).unwrap();

        if complete {
            assert!(cursor.is_none());
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
                std::fs::metadata(&source).unwrap().ino(),
                std::fs::metadata(&destination).unwrap().ino()
            );
        } else {
            let mut cursor = cursor.expect("unsupported hard links should fall back to copying");
            while !cursor.advance_quantum(&|| Ok(())).unwrap() {}
        }
        assert_eq!(std::fs::read(&destination).unwrap(), b"shared payload");
    }
}
