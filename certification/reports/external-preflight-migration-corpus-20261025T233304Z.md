# TorrentNG Migration Corpus Certification

- Date UTC: 2026-10-25T23:33:04Z
- Host: kspls0
- Commit: c4329b2
- Corpus directory: /home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/testdata/migration-corpus
- Strict corpus required: 1

## Synthetic Import/Apply Baseline

```text
   Compiling proc-macro2 v1.0.106
   Compiling unicode-ident v1.0.24
   Compiling quote v1.0.45
   Compiling cfg-if v1.0.4
   Compiling libc v0.2.186
   Compiling version_check v0.9.5
   Compiling typenum v1.20.0
   Compiling stable_deref_trait v1.2.1
   Compiling smallvec v1.15.1
   Compiling once_cell v1.21.4
   Compiling serde_core v1.0.228
   Compiling thiserror v2.0.20
   Compiling getrandom v0.4.2
   Compiling writeable v0.6.3
   Compiling pin-project-lite v0.2.17
   Compiling serde v1.0.228
   Compiling litemap v0.8.2
   Compiling icu_normalizer_data v2.2.0
   Compiling memchr v2.8.0
   Compiling utf8_iter v1.0.4
   Compiling icu_properties_data v2.2.0
   Compiling bitflags v2.11.1
   Compiling zerocopy v0.8.48
   Compiling futures-core v0.3.32
   Compiling find-msvc-tools v0.1.9
   Compiling const-oid v0.10.2
   Compiling parking_lot_core v0.9.12
   Compiling futures-sink v0.3.32
   Compiling shlex v1.3.0
   Compiling cpufeatures v0.3.1
   Compiling futures-task v0.3.32
   Compiling pkg-config v0.3.33
   Compiling scopeguard v1.2.0
   Compiling vcpkg v0.2.15
   Compiling futures-channel v0.3.32
   Compiling futures-io v0.3.32
   Compiling slab v0.4.12
   Compiling zmij v1.0.21
   Compiling cpufeatures v0.2.17
   Compiling cc v1.2.62
   Compiling lock_api v0.4.14
   Compiling tracing-core v0.1.36
   Compiling serde_json v1.0.149
   Compiling io-uring v0.7.12
   Compiling percent-encoding v2.3.2
   Compiling bytes v1.11.1
   Compiling itoa v1.0.18
   Compiling hex v0.4.3
   Compiling fallible-streaming-iterator v0.1.9
   Compiling fallible-iterator v0.3.0
   Compiling rustix v1.1.4
   Compiling generic-array v0.14.7
   Compiling ahash v0.8.12
   Compiling linux-raw-sys v0.12.1
   Compiling fastrand v2.4.1
   Compiling form_urlencoded v1.2.2
   Compiling hybrid-array v0.4.15
   Compiling block-buffer v0.12.1
   Compiling crypto-common v0.2.2
   Compiling block-buffer v0.10.4
   Compiling crypto-common v0.1.7
   Compiling digest v0.10.7
   Compiling syn v2.0.117
   Compiling syn v3.0.6
   Compiling sha1 v0.10.7
   Compiling libsqlite3-sys v0.28.0
   Compiling errno v0.3.14
   Compiling mio v1.2.0
   Compiling socket2 v0.6.3
   Compiling digest v0.11.3
   Compiling signal-hook-registry v1.4.8
   Compiling sha2 v0.11.0
   Compiling parking_lot v0.12.5
   Compiling tempfile v3.27.0
   Compiling uuid v1.23.1
   Compiling rt-hash v0.1.0 (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/crates/rt-hash)
   Compiling thiserror-impl v2.0.20
   Compiling rt-bencode v0.1.0 (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/crates/rt-bencode)
   Compiling hashbrown v0.14.5
   Compiling hashlink v0.9.1
   Compiling synstructure v0.13.2
   Compiling zerofrom-derive v0.1.7
   Compiling yoke-derive v0.8.2
   Compiling zerovec-derive v0.11.3
   Compiling displaydoc v0.2.5
   Compiling serde_derive v1.0.228
   Compiling futures-macro v0.3.32
   Compiling tracing-attributes v0.1.31
   Compiling tokio-macros v2.7.0
   Compiling tokio v1.53.3
   Compiling futures-util v0.3.32
   Compiling zerofrom v0.1.8
   Compiling yoke v0.8.2
   Compiling zerovec v0.11.6
   Compiling zerotrie v0.2.4
   Compiling tracing v0.1.44
   Compiling rt-metrics v0.1.0 (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/crates/rt-metrics)
   Compiling tinystr v0.8.3
   Compiling potential_utf v0.1.5
   Compiling icu_collections v2.2.0
   Compiling icu_locale_core v2.2.0
   Compiling icu_provider v2.2.0
   Compiling rt-path v0.1.0 (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/crates/rt-path)
   Compiling icu_properties v2.2.0
   Compiling icu_normalizer v2.2.0
   Compiling rt-piece-map v0.1.0 (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/crates/rt-piece-map)
   Compiling futures-executor v0.3.32
   Compiling futures v0.3.32
   Compiling idna_adapter v1.2.2
   Compiling idna v1.1.0
   Compiling url v2.5.8
   Compiling rt-metainfo v0.1.0 (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/crates/rt-metainfo)
   Compiling rusqlite v0.31.0
   Compiling rt-storage v0.1.0 (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/crates/rt-storage)
   Compiling rt-db v0.1.0 (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/crates/rt-db)
   Compiling rt-fastresume v0.1.0 (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/crates/rt-fastresume)
   Compiling rt-migrate v0.1.0 (/home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/crates/rt-migrate)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 31.26s
     Running unittests src/lib.rs (target/debug/deps/rt_migrate-3a470b3599953cfd)

running 80 tests
test export::tests::export_materialized_byte_admission_is_bounded ... ok
test export::tests::format_parsing_accepts_aliases ... ok
test source_state::tests::a_gone_process_or_unknown_platform_is_a_crash ... ok
test source_state::tests::a_live_process_whose_name_cannot_be_read_counts_as_running ... ok
test source_state::tests::a_lock_from_another_host_cannot_be_checked_and_is_stale ... ok
test source_state::tests::a_live_process_named_like_the_client_means_running ... ok
test source_state::tests::a_stale_deluge_pid_file_is_a_crash ... ok
test source_state::tests::a_reused_pid_owned_by_another_program_is_a_crash ... ok
test source_state::tests::an_empty_name_list_accepts_any_live_process ... ok
test source_state::tests::an_unrecognized_lock_is_a_crash ... ok
test source_state::tests::a_stale_rtorrent_lock_is_a_crash_and_garbage_is_too ... ok
test source_state::tests::garbage_and_empty_locks_do_not_parse ... ok
test source_state::tests::interpreters_are_treated_as_possibly_the_client ... ok
test source_state::tests::a_pid_only_head_is_not_mistaken_for_a_host_pair ... ok
test source_state::tests::qt_lock_files_parse_pid_app_and_host_lines ... ok
test source_state::tests::a_stale_qbittorrent_lockfile_is_found_in_a_parent_directory ... ok
test source_state::tests::rtorrent_absence_is_clean_and_other_sources_are_unknown ... ok
test source_state::tests::an_explicit_lock_held_by_a_live_process_means_running ... ok
test source_state::tests::an_explicit_lock_works_for_any_source_and_absence_means_clean ... ok
test source_state::tests::plain_pid_files_parse_including_deluge_pid_and_port ... ok
test source_state::tests::a_pid_nobody_holds_is_not_running ... ok
test source_state::tests::rtorrent_locks_parse_with_and_without_the_plus ... ok
test source_state::tests::this_process_is_seen_running_with_its_own_name ... ok
test source_state::tests::search_dirs_stops_at_the_filesystem_root ... ok
test tests::auxiliary_classification_ignores_hash_coincidences_in_filenames ... ok
test tests::biglybt_downloads_config_matches_hex_entries ... ok
test tests::bencoded_lifecycle_state_imports_to_native_torrent_row ... ok
test tests::bencoded_tracker_activity_imports_to_native_tracker_rows ... ok
test tests::aggregate_json_resume_matches_base32_info_hash_entries ... ok
test tests::bencoded_file_selection_imports_to_native_file_rows ... ok
test tests::bencoded_file_progress_imports_to_native_file_rows ... ok
test tests::broad_sources_are_scannable_metadata_first ... ok
test tests::db_import_preserves_completion_for_zero_length_torrents ... ok
test tests::db_import_does_not_turn_an_oversized_legacy_counter_into_completion ... ok
test tests::db_import_uses_piece_progress_not_cumulative_download_counter ... ok
test tests::dry_run_preserves_auxiliary_client_artifacts_separately ... ok
test tests::fastresume_apply_persists_imported_state_and_summary ... ok
test tests::file_hints_do_not_trust_final_symlinks ... ok
test tests::json_file_selection_imports_to_native_file_rows ... ok
test tests::json_lifecycle_state_imports_to_native_torrent_row ... ok
test tests::json_tracker_activity_imports_to_native_tracker_rows ... ok
test tests::padding_files_are_never_marked_wanted ... ok
test tests::oversized_resume_sidecar_is_skipped_with_warning ... ok
test tests::migration_plan_materialization_admission_is_bounded ... ok
test tests::path_remap_uses_longest_matching_prefix ... ok
test tests::json_file_progress_imports_to_native_file_rows ... ok
test tests::qbit_dry_run_preserves_resume_metadata ... ok
test tests::path_remap_updates_db_rows_and_file_hint_trust ... ok
test tests::qbit_libtorrent2_resume_unpacks_bit_packed_pieces_field ... ok
test tests::qbit_libtorrent_resume_imports_piece_state ... ok
test tests::short_piece_state_is_padded_and_reported ... ok
test tests::rtorrent_single_file_directory_is_left_unchanged ... ok
test tests::transmission_dry_run_reads_bencoded_resume ... ok
test tests::import_source_matrix_preserves_common_json_resume_fields ... ok
test tests::rtorrent_pairs_hash_torrent_rtorrent_sidecar ... ok
test tests::recursive_scan_does_not_follow_directory_symlinks ... ok
test tests::rtorrent_dry_run_reports_missing_resume ... ok
test tests::require_verification_downgrades_imported_valid_pieces ... ok
test tests::rtorrent_complete_resume_synthesizes_seed_piece_state ... ok
test tests::utorrent_bitfield_resume_imports_piece_state_under_trust_hints ... ok
test tests::rtorrent_multi_file_directory_already_includes_torrent_name ... ok
test tests::tixati_proprietary_state_stays_verification_first ... ok
test tests::rtorrent_multi_file_directory_renamed_by_external_tool_stays_safe_not_silently_broken ... ok
test tests::utorrent_resume_dat_matches_raw_info_hash_entries ... ok
test tests::partial_piece_count_is_bounded ... ok
test tests::partial_piece_blocks_are_sorted_deduped_and_bounded ... ok
test export::tests::oversized_blob_is_skipped_before_reading_contents ... ok
test export::tests::transmission_export_round_trips ... ok
test tests::native_import_applies_db_and_fastresume_together ... ok
test tests::import_plan_applies_native_db_rows ... ok
test export::tests::libtorrent_export_round_trips_through_qbittorrent_importer ... ok
test export::tests::rtorrent_export_complete_is_recheck_free ... ok
test export::tests::missing_blob_is_skipped_not_fatal ... ok
test export::tests::generic_export_copies_torrent_and_manifest ... ok
test export::tests::rtorrent_export_partial_is_metadata_only ... ok
test export::tests::malformed_database_hash_is_skipped_before_path_join ... ok
test tests::migration_scan_budget_rejects_file_and_path_limits ... ok
test export::tests::utorrent_and_biglybt_aggregates_round_trip ... ok
test tests::native_apply_matrix_persists_common_resume_fields_for_all_sources ... ok
test tests::bencoded_import_source_matrix_preserves_client_specific_aliases ... ok

test result: ok. 80 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.22s

     Running tests/round_trip_matrix.rs (target/debug/deps/round_trip_matrix-02cbffb41975a4d2)

running 7 tests
test import_matrix_complete_and_partial_isos ... ok
test production_shape_directory_equals_content_folder_bytes_preserved_and_trusted ... ok
test production_shape_directory_renamed_by_external_tool_stays_safe_metadata_only ... ok
test generic_export_is_universal_exit ... ok
test production_shape_bep47_padding_file_not_wanted_real_files_trusted ... ok
test round_trip_matrix_preserves_state ... ok
test export_matrix_fidelity_and_layout ... ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.23s

     Running tests/scale.rs (target/debug/deps/scale-672d22434f7f2f7f)

running 1 test
test qbit_15k_dry_run_import_is_certified ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.81s

   Doc-tests rt_migrate

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

```

## Bidirectional Round-Trip Matrix

Every supported client x import/export/round-trip direction, with
ISO-shaped single-file and multi-file fixtures and complete + partial
piece state. Source: `crates/rt-migrate/tests/round_trip_matrix.rs`.

```text
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.47s
     Running tests/round_trip_matrix.rs (target/debug/deps/round_trip_matrix-02cbffb41975a4d2)

running 7 tests
test production_shape_directory_renamed_by_external_tool_stays_safe_metadata_only ... ok
test production_shape_directory_equals_content_folder_bytes_preserved_and_trusted ... ok
test import_matrix_complete_and_partial_isos ... ok
test production_shape_bep47_padding_file_not_wanted_real_files_trusted ... ok
test generic_export_is_universal_exit ... ok
test round_trip_matrix_preserves_state ... ok
test export_matrix_fidelity_and_layout ... ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.21s

```

## Exported Corpus Coverage

| Source family | Result | Evidence files | Evidence root |
|---|---|---:|---|
| qbittorrent | PASS | 1 | /home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/testdata/migration-corpus/qbittorrent |
| transmission | PASS | 1 | /home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/testdata/migration-corpus/transmission |
| deluge | PASS | 1 | /home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/testdata/migration-corpus/deluge |
| utorrent | PASS | 1 | /home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/testdata/migration-corpus/utorrent |
| biglybt | PASS | 1 | /home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/testdata/migration-corpus/biglybt |
| tixati | PASS | 1 | /home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/testdata/migration-corpus/tixati |
| rtorrent | PASS | 1 | /home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/testdata/migration-corpus/rtorrent |
| generic | PASS | 1 | /home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/testdata/migration-corpus/generic |

## Corpus Manifest

- Manifest: /home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/testdata/migration-corpus/manifest.toml
- Status: PASS
- Detail: /home/keith/.codex-tasks/torrentng-release-readiness-20261025-02/repo-final/testdata/migration-corpus/manifest.toml

| Source family | Artifact | Source | Permission | SHA-256 |
|---|---|---|---|---|
| qbittorrent | qbittorrent/generated.fastresume | TorrentNG generated qBittorrent/libtorrent-style resume fixture | generated by TorrentNG test corpus; safe to commit | 2a29527633b4d09dfc03858133a23763f37025dfd539088fd9da9540eabf8744 |
| transmission | transmission/generated.resume | TorrentNG generated Transmission-style resume fixture | generated by TorrentNG test corpus; safe to commit | 648fbae32c095778c5580f737cbef519b8be4fd9d1c13e8cc52a64d48bb1ea44 |
| deluge | deluge/generated.state | TorrentNG generated Deluge-style state fixture | generated by TorrentNG test corpus; safe to commit | d760829e14f69c630809793487b8a1bc35cd5a9f674775dcda6308aae9cbfcb0 |
| utorrent | utorrent/resume.dat | TorrentNG generated uTorrent/BitTorrent Classic-style resume fixture | generated by TorrentNG test corpus; safe to commit | 3ec4c23f10503f47e0241f05011cfe9b0e5af152b1a46b557a061763d5f02360 |
| biglybt | biglybt/downloads.config | TorrentNG generated BiglyBT/Vuze-style config fixture | generated by TorrentNG test corpus; safe to commit | ef8b062b3c42b8a4d666ab7f7554b186e935f7cb3a1774226ba6f562ae50f491 |
| tixati | tixati/generated.config | TorrentNG generated Tixati-style config fixture | generated by TorrentNG test corpus; safe to commit | b6de385622b7061e58decadba3760add60201a7fe976cf11f71c8d4725a6cc30 |
| rtorrent | rtorrent/generated.torrent | TorrentNG generated rTorrent/session-style torrent fixture | generated by TorrentNG test corpus; safe to commit | 7c54f6880385036724a550e43ebd05c71a65efa02e3a09317fd3e64b47655b6b |
| generic | generic/generated.resume.json | TorrentNG generated generic JSON resume fixture | generated by TorrentNG test corpus; safe to commit | 450229e110a1aeed102d87f1c792ca5f192f9bc73388d2c34cc3c50dd0f8beae |

## Evidence Inventory

| Source family | File | SHA-256 |
|---|---|---|
| qbittorrent | testdata/migration-corpus/qbittorrent/generated.fastresume | 2a29527633b4d09dfc03858133a23763f37025dfd539088fd9da9540eabf8744 |
| transmission | testdata/migration-corpus/transmission/generated.resume | 648fbae32c095778c5580f737cbef519b8be4fd9d1c13e8cc52a64d48bb1ea44 |
| deluge | testdata/migration-corpus/deluge/generated.state | d760829e14f69c630809793487b8a1bc35cd5a9f674775dcda6308aae9cbfcb0 |
| utorrent | testdata/migration-corpus/utorrent/resume.dat | 3ec4c23f10503f47e0241f05011cfe9b0e5af152b1a46b557a061763d5f02360 |
| biglybt | testdata/migration-corpus/biglybt/downloads.config | ef8b062b3c42b8a4d666ab7f7554b186e935f7cb3a1774226ba6f562ae50f491 |
| tixati | testdata/migration-corpus/tixati/generated.config | b6de385622b7061e58decadba3760add60201a7fe976cf11f71c8d4725a6cc30 |
| rtorrent | testdata/migration-corpus/rtorrent/generated.torrent | 7c54f6880385036724a550e43ebd05c71a65efa02e3a09317fd3e64b47655b6b |
| generic | testdata/migration-corpus/generic/generated.resume.json | 450229e110a1aeed102d87f1c792ca5f192f9bc73388d2c34cc3c50dd0f8beae |

## Required Layout

```text
testdata/migration-corpus/qbittorrent/
testdata/migration-corpus/transmission/
testdata/migration-corpus/deluge/
testdata/migration-corpus/utorrent/
testdata/migration-corpus/biglybt/
testdata/migration-corpus/tixati/
testdata/migration-corpus/rtorrent/
testdata/migration-corpus/generic/
```

Place real exported client resume/config/torrent artifacts under each source family.
When artifacts are present, copy manifest.example.toml to manifest.toml and record source/version/permission metadata.
Strict release mode requires a manifest, family-confined declared artifacts, and declarations for every discovered evidence file.
Set TNG_REQUIRE_MIGRATION_CORPUS=1 to make missing source-family corpora fail this gate.

- Missing source families: 0

Overall status: PASS
