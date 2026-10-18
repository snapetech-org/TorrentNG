# rt-testkit

test fixtures, synthetic torrent generators, interop helpers.

## Status

Database helpers are implemented. The transfer corpus is authored; the
requested bulk pass has not run.

Provides migrated in-memory database setup, deterministic torrent row
generators, and the TorrentNG-client scale certification matrix:
1k, 5k, 10k, 15k, and 50k torrents.

The transfer compatibility corpus generates deterministic, private v1, pure-v2,
and hybrid metainfo from real synthetic payload bytes. It includes a file that
crosses a piece boundary, a nested file, an empty file, authenticated v2 piece
layers, and BEP 47 hybrid padding. Its regression tests exercise the production
metainfo parser and piece map; the bulk pass has not run on this revision.
