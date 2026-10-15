# Release-note fragments

For any user-facing change, add one structured fragment under `release-notes/` describing it. A
single fragment may cover related user-facing changes in the same pull request. Keep shipped fragments unchanged.

```md
---
category: fixed
audience: users, operators
area: webui
action: none
breaking: false
---
The torrent list now keeps the selected rows when filters change, so bulk actions can be applied to the intended matches.
```

The frontmatter records the category (`added`, `changed`, `fixed`, `security`,
`removed`, or `deprecated`), audience (`users`, `operators`, or both), product
area, required action (or `none`), and breaking-change status. Write the body for
the person using or operating TorrentNG; keep it between 30 and 400 characters,
start with a capitalized sentence, and end with punctuation.

For internal-only work, no fragment is needed. In a pull request, either select the internal-only
option in the template or put `release-note: none` on its own line in the description. When a
direct push or merge check reads commit messages, put the same marker on its own line in the
commit message. The tag
workflow assembles fragments since the previous published release and uses the
same text for the GitHub release and Discord announcement. If a release range
contains no user-facing changes, its release page says so explicitly.

Preview a release range with:

```bash
python3 scripts/release_notes.py preview --base <previous-tag> --head <tag>
```
