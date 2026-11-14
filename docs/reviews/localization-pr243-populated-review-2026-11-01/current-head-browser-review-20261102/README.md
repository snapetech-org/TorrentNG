# TorrentNG PR #243: current-head browser review

Reviewed 2026-11-02 against PR head `93a95bdae25f4ea64db432c1ab28412924c326c1` (tree `2ceab2404276500421ba845c2e3554bfee4cc8a1`). The headed browser ran in the isolated Agent GUI VM at `http://192.168.50.148:4173/`.

## Preview boundary

The Vite preview proxied `/api` and `/health` only to the task-owned synthetic fixture server on `127.0.0.1:4174`. It did not proxy `/ws` and did not connect to the host qBittorrent service. The visible list contained 28 synthetic torrent fixtures. No real torrent operations were sent.

## Findings

- **Arabic appearance:** navigation and controls were mirrored and right-aligned. The screenshot shows no clipping or overlap in the reviewed support and appearance panel.
- **French server settings:** the accessible snapshot exposed the pending autosave live status as `Sauvegarde automatique en attente · modifications : 1 · applicables sans redémarrage : 1 · redémarrage : 0.` This confirms the changed live-versus-restart wording in context. The count values came from the synthetic pending setting.
- **Vietnamese torrent list:** the tracker column header rendered as `tracker`; the matching synthetic row exposed `tracker.example`. The label remains legible in the list layout.

## Evidence

- `appearance-ar.png` — Arabic support and appearance panel; SHA-256 `659d71e59686baa6b8f8fd5334b4f3af282ac8b9988ac2bc6b78e44c81820e1d`.
- `torrents-vi-tracker.png` — Vietnamese torrent list with tracker column; SHA-256 `9a1c968589f4de6eaf33f3851d6134a20eeb18d16325fff33a83a58f8aa73fc7`.

This is a headed UI review of the listed surfaces and locales, not native-speaker approval for the full locale set.
