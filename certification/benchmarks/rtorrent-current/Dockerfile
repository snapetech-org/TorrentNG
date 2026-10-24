FROM debian:trixie-slim@sha256:62bd423aeee912639ca8ee681c2b762cb3e25137e0a92b20be455898a9f034ad

ARG DEBIAN_SNAPSHOT=20261021T000000Z
ARG LIBTORRENT_VERSION=v0.16.26
ARG LIBTORRENT_COMMIT=21daa3b96a3b8cfe14961545b5115de7ed12da95
ARG RTORRENT_VERSION=v0.16.26
ARG RTORRENT_COMMIT=62899bd053c39b70380464e8f64dbcfce92579aa

ENV DEBIAN_FRONTEND=noninteractive

RUN rm -f /etc/apt/sources.list.d/debian.sources \
    && printf 'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/%s/ trixie main\ndeb [check-valid-until=no] http://snapshot.debian.org/archive/debian/%s/ trixie-updates main\ndeb [check-valid-until=no] http://snapshot.debian.org/archive/debian-security/%s/ trixie-security main\n' "$DEBIAN_SNAPSHOT" "$DEBIAN_SNAPSHOT" "$DEBIAN_SNAPSHOT" > /etc/apt/sources.list \
    && apt-get -o Acquire::Check-Valid-Until=false update \
    && apt-get install -y --no-install-recommends \
        autoconf \
        automake \
        build-essential \
        ca-certificates \
        git \
        libcurl4-openssl-dev \
        libncurses-dev \
        libssl-dev \
        libtool \
        libxmlrpc-core-c3-dev \
        pkg-config \
        zlib1g-dev \
    && rm -rf /var/lib/apt/lists/*

RUN git clone --depth 1 --branch "$LIBTORRENT_VERSION" \
        https://github.com/rakshasa/libtorrent.git /tmp/libtorrent \
    && test "$(git -C /tmp/libtorrent rev-parse HEAD)" = "$LIBTORRENT_COMMIT" \
    && cd /tmp/libtorrent \
    && autoreconf -ivf \
    && ./configure --prefix=/usr/local --disable-debug \
    && make -j4 \
    && make install \
    && ldconfig \
    && rm -rf /tmp/libtorrent

RUN git clone --depth 1 --branch "$RTORRENT_VERSION" \
        https://github.com/rakshasa/rtorrent.git /tmp/rtorrent \
    && test "$(git -C /tmp/rtorrent rev-parse HEAD)" = "$RTORRENT_COMMIT" \
    && cd /tmp/rtorrent \
    && autoreconf -ivf \
    && ./configure --prefix=/usr/local --disable-debug --with-xmlrpc-c \
    && make -j4 \
    && make install \
    && ldconfig \
    && mkdir -p /usr/local/share/torrentng-benchmark \
    && printf 'rTorrent=%s (%s)\nlibtorrent=%s (%s)\nDebian snapshot=%s\n' \
        "$RTORRENT_VERSION" "$RTORRENT_COMMIT" \
        "$LIBTORRENT_VERSION" "$LIBTORRENT_COMMIT" \
        "$DEBIAN_SNAPSHOT" \
        > /usr/local/share/torrentng-benchmark/rtorrent-build-info.txt \
    && rm -rf /tmp/rtorrent

LABEL org.opencontainers.image.title="TorrentNG current upstream rTorrent benchmark" \
      org.opencontainers.image.version="$RTORRENT_VERSION" \
      org.opencontainers.image.revision="$RTORRENT_COMMIT" \
      org.torrentng.benchmark.libtorrent.version="$LIBTORRENT_VERSION" \
      org.torrentng.benchmark.libtorrent.revision="$LIBTORRENT_COMMIT" \
      org.torrentng.benchmark.debian.snapshot="$DEBIAN_SNAPSHOT"

ENV HOME=/config
RUN groupadd --gid 1000 torrent \
    && useradd --uid 1000 --gid 1000 --home-dir /config --shell /usr/sbin/nologin torrent \
    && mkdir -p /config \
    && chown torrent:torrent /config
USER torrent
ENTRYPOINT ["/usr/local/bin/rtorrent"]
CMD ["-n", "-o", "import=/etc/rtorrent/rtorrent.rc", "-o", "system.daemon.set=true"]
