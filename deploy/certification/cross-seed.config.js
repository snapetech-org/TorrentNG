const token = process.env.TNG_API_TOKEN || "local-cert-api-token-20260904";
const username = encodeURIComponent(token);
const password = encodeURIComponent(token);
const qbitHost = process.env.TNG_CROSS_SEED_QBIT_HOST || "http://torrentng:8080";
const qbitScheme = qbitHost.startsWith("https://") ? "https" : "http";
const qbitAuthority = qbitHost.replace(/^https?:\/\//, "");

module.exports = {
  apiKey: "certification-token-000000000000",
  host: "0.0.0.0",
  port: 2468,
  action: "inject",
  matchMode: "strict",
  useClientTorrents: true,
  torrentClients: [`qbittorrent:${qbitScheme}://${username}:${password}@${qbitAuthority}`],
  outputDir: null,
  torznab: [],
  linkDirs: ["/downloads"],
  dataDirs: [],
  seasonFromEpisodes: null,
};
