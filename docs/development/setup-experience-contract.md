# Setup experience contract

TorrentNG already has three distinct product arrangements and a corresponding
first-run path in [Desktop](../DESKTOP.md), [deployment](../DEPLOYMENT.md), and
[configuration](../CONFIGURATION.md): Desktop can run its bundled local engine
or connect to a remote Server; Server is the headless first-party engine;
WebUI Bridge fronts an existing supported torrent client. Preserve those
separate setup paths and their config ownership.

## Product-specific flow

- **Desktop, local:** the existing setup card chooses local mode and exposes
  the download folder and peer port. Explain each choice in place, validate the
  folder/port, then start the bundled engine and show connection status.
- **Desktop, remote:** use the existing Server URL and API-token form. Explain
  that a remote connection needs a reachable Server and protected transport.
  Validate the connection without echoing or storing the token in UI state
  beyond what the existing credential handling allows.
- **Server:** keep deployment-specific configuration in `torrentngd` config
  and its existing authenticated Settings/API. Ask about storage only when the
  default directory is unsuitable; surface the active listen port and auth
  readiness before accepting remote use.
- **WebUI Bridge:** start with the existing-client choice (rTorrent,
  qBittorrent, Transmission, or Deluge), then use that adapter's current
  credentials and compatibility checks. Do not ask the operator to configure
  TorrentNG's owned-transfer storage for transfers the existing client owns.

## Contract

- Route by the user's actual installation mode before asking backend-specific
  questions.
- Reuse current connection forms, config files, runtime settings, and API
  checks; do not add a cross-mode config layer.
- Explain which application owns files, state, credentials, and transfers in
  the selected mode.
- Keep storage changes explicit and previewable. Never move, recursively
  chown, or overwrite a download tree as a setup shortcut.
- Save only confirmed settings, then report the precise connection or storage
  status and the next actionable repair when validation fails.
- Leave advanced networking, tracker identity, crash-safety, and seeding
  controls in their existing Settings/docs unless the operator asks for them.

The shared visual language may be consistent across Desktop and WebUI, while
questions and validation remain specific to each backend.
