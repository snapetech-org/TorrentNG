#define _GNU_SOURCE
#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <pthread.h>
#include <signal.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/uio.h>
#include <time.h>
#include <unistd.h>

#define FILE_BYTES (64u * 1024u * 1024u)
#define PIECE_BYTES (1024u * 1024u)
#define BLOCK_BYTES (16u * 1024u)
#define BLOCKS_PER_BATCH 8u
#define BATCH_BYTES (BLOCK_BYTES * BLOCKS_PER_BATCH)
#define FRAME_HEADER_BYTES 13u
#define REPEATS 4u
#define ROUNDS 4u
#define FNV_OFFSET UINT64_C(14695981039346656037)
#define FNV_PRIME UINT64_C(1099511628211)

struct reader_state {
    int fd;
    uint64_t expected_bytes;
    uint64_t received_bytes;
    uint64_t hash;
    int error;
};

static void die(const char *what) {
    perror(what);
    exit(1);
}

static void put_be32(unsigned char *dst, uint32_t value) {
    dst[0] = (unsigned char)(value >> 24);
    dst[1] = (unsigned char)(value >> 16);
    dst[2] = (unsigned char)(value >> 8);
    dst[3] = (unsigned char)value;
}

static unsigned char fixture_byte(uint64_t offset) {
    uint64_t value = offset * UINT64_C(131) + (offset >> 8) * UINT64_C(17) + UINT64_C(73);
    value ^= value >> 17;
    value *= UINT64_C(0x9e3779b185ebca87);
    value ^= value >> 29;
    return (unsigned char)value;
}

static void create_fixture(const char *path) {
    int fd = open(path, O_CREAT | O_TRUNC | O_WRONLY | O_CLOEXEC, 0644);
    if (fd < 0) die("open fixture");
    unsigned char buffer[64 * 1024];
    for (uint64_t base = 0; base < FILE_BYTES; base += sizeof(buffer)) {
        for (size_t i = 0; i < sizeof(buffer); i++) buffer[i] = fixture_byte(base + i);
        size_t done = 0;
        while (done < sizeof(buffer)) {
            ssize_t n = write(fd, buffer + done, sizeof(buffer) - done);
            if (n < 0 && errno == EINTR) continue;
            if (n <= 0) die("write fixture");
            done += (size_t)n;
        }
    }
    if (fsync(fd) != 0) die("fsync fixture");
    close(fd);
}

static void pread_exact(int fd, void *buffer, size_t length, uint64_t offset) {
    size_t done = 0;
    while (done < length) {
        ssize_t n = pread(fd, (unsigned char *)buffer + done, length - done, (off_t)(offset + done));
        if (n < 0 && errno == EINTR) continue;
        if (n <= 0) die("pread fixture");
        done += (size_t)n;
    }
}

static void make_header(unsigned char header[FRAME_HEADER_BYTES], uint64_t offset) {
    put_be32(header, 9u + BLOCK_BYTES);
    header[4] = 7;
    put_be32(header + 5, (uint32_t)(offset / PIECE_BYTES));
    put_be32(header + 9, (uint32_t)(offset % PIECE_BYTES));
}

static uint64_t hash_update(uint64_t hash, const unsigned char *data, size_t length) {
    for (size_t i = 0; i < length; i++) {
        hash ^= data[i];
        hash *= FNV_PRIME;
    }
    return hash;
}

static uint64_t expected_wire_hash(int file_fd) {
    unsigned char payload[BATCH_BYTES];
    uint64_t hash = FNV_OFFSET;
    for (unsigned repeat = 0; repeat < REPEATS; repeat++) {
        for (uint64_t base = 0; base < FILE_BYTES; base += BATCH_BYTES) {
            pread_exact(file_fd, payload, sizeof(payload), base);
            for (unsigned block = 0; block < BLOCKS_PER_BATCH; block++) {
                unsigned char header[FRAME_HEADER_BYTES];
                uint64_t offset = base + (uint64_t)block * BLOCK_BYTES;
                make_header(header, offset);
                hash = hash_update(hash, header, sizeof(header));
                hash = hash_update(hash, payload + (size_t)block * BLOCK_BYTES, BLOCK_BYTES);
            }
        }
    }
    return hash;
}

static void *reader_main(void *opaque) {
    struct reader_state *state = opaque;
    unsigned char buffer[64 * 1024];
    state->hash = FNV_OFFSET;
    while (state->received_bytes < state->expected_bytes) {
        ssize_t n = read(state->fd, buffer, sizeof(buffer));
        if (n < 0 && errno == EINTR) continue;
        if (n < 0) { state->error = errno; return NULL; }
        if (n == 0) { state->error = EPIPE; return NULL; }
        state->hash = hash_update(state->hash, buffer, (size_t)n);
        state->received_bytes += (uint64_t)n;
    }
    return NULL;
}

static void make_connection(int *sender_fd, int *receiver_fd) {
    int listener = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (listener < 0) die("socket listener");
    int one = 1;
    if (setsockopt(listener, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one)) != 0) die("reuseaddr");
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    addr.sin_port = 0;
    if (bind(listener, (struct sockaddr *)&addr, sizeof(addr)) != 0) die("bind");
    if (listen(listener, 1) != 0) die("listen");
    socklen_t addr_len = sizeof(addr);
    if (getsockname(listener, (struct sockaddr *)&addr, &addr_len) != 0) die("getsockname");
    int sender = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (sender < 0) die("socket sender");
    if (connect(sender, (struct sockaddr *)&addr, sizeof(addr)) != 0) die("connect");
    int receiver = accept4(listener, NULL, NULL, SOCK_CLOEXEC);
    if (receiver < 0) die("accept");
    close(listener);
    if (setsockopt(sender, IPPROTO_TCP, TCP_NODELAY, &one, sizeof(one)) != 0) die("sender nodelay");
    if (setsockopt(receiver, IPPROTO_TCP, TCP_NODELAY, &one, sizeof(one)) != 0) die("receiver nodelay");
    *sender_fd = sender;
    *receiver_fd = receiver;
}

static void writev_all(int fd, struct iovec *iov, int count) {
    int index = 0;
    while (index < count) {
        ssize_t n = writev(fd, &iov[index], count - index);
        if (n < 0 && errno == EINTR) continue;
        if (n <= 0) die("writev socket");
        size_t remaining = (size_t)n;
        while (index < count && remaining >= iov[index].iov_len) {
            remaining -= iov[index].iov_len;
            index++;
        }
        if (index < count && remaining > 0) {
            iov[index].iov_base = (unsigned char *)iov[index].iov_base + remaining;
            iov[index].iov_len -= remaining;
        }
    }
}

static void send_batch(int socket_fd, const unsigned char *payload, uint64_t base) {
    unsigned char headers[BLOCKS_PER_BATCH][FRAME_HEADER_BYTES];
    struct iovec iov[BLOCKS_PER_BATCH * 2];
    for (unsigned block = 0; block < BLOCKS_PER_BATCH; block++) {
        uint64_t offset = base + (uint64_t)block * BLOCK_BYTES;
        make_header(headers[block], offset);
        iov[block * 2].iov_base = headers[block];
        iov[block * 2].iov_len = FRAME_HEADER_BYTES;
        iov[block * 2 + 1].iov_base = (void *)(payload + (size_t)block * BLOCK_BYTES);
        iov[block * 2 + 1].iov_len = BLOCK_BYTES;
    }
    writev_all(socket_fd, iov, (int)(BLOCKS_PER_BATCH * 2));
}

static double clock_seconds(clockid_t clock_id) {
    struct timespec ts;
    if (clock_gettime(clock_id, &ts) != 0) die("clock_gettime");
    return (double)ts.tv_sec + (double)ts.tv_nsec / 1e9;
}

static void run_method(int file_fd, int mapped, unsigned round, double *cpu_seconds, double *wall_seconds, uint64_t expected_hash) {
    int sender_fd, receiver_fd;
    make_connection(&sender_fd, &receiver_fd);
    uint64_t total_payload = (uint64_t)FILE_BYTES * REPEATS;
    struct reader_state reader = {
        .fd = receiver_fd,
        .expected_bytes = total_payload + (total_payload / BLOCK_BYTES) * FRAME_HEADER_BYTES,
        .received_bytes = 0,
        .hash = FNV_OFFSET,
        .error = 0,
    };
    pthread_t reader_thread;
    int err = pthread_create(&reader_thread, NULL, reader_main, &reader);
    if (err != 0) { errno = err; die("pthread_create reader"); }

    unsigned char buffer[BATCH_BYTES];
    double cpu_start = clock_seconds(CLOCK_THREAD_CPUTIME_ID);
    double wall_start = clock_seconds(CLOCK_MONOTONIC);
    for (unsigned repeat = 0; repeat < REPEATS; repeat++) {
        if (mapped) {
            for (uint64_t piece_base = 0; piece_base < FILE_BYTES; piece_base += PIECE_BYTES) {
                void *mapping = mmap(NULL, PIECE_BYTES, PROT_READ, MAP_SHARED, file_fd, (off_t)piece_base);
                if (mapping == MAP_FAILED) die("mmap piece window");
                const unsigned char *window = mapping;
                for (uint64_t within = 0; within < PIECE_BYTES; within += BATCH_BYTES) {
                    send_batch(sender_fd, window + within, piece_base + within);
                }
                if (munmap(mapping, PIECE_BYTES) != 0) die("munmap piece window");
            }
        } else {
            for (uint64_t base = 0; base < FILE_BYTES; base += BATCH_BYTES) {
                pread_exact(file_fd, buffer, sizeof(buffer), base);
                send_batch(sender_fd, buffer, base);
            }
        }
    }
    if (shutdown(sender_fd, SHUT_WR) != 0) die("shutdown sender");
    double wall_end = clock_seconds(CLOCK_MONOTONIC);
    double cpu_end = clock_seconds(CLOCK_THREAD_CPUTIME_ID);
    err = pthread_join(reader_thread, NULL);
    if (err != 0) { errno = err; die("pthread_join reader"); }
    close(sender_fd);
    close(receiver_fd);
    if (reader.error != 0) { errno = reader.error; die("reader"); }
    if (reader.received_bytes != reader.expected_bytes || reader.hash != expected_hash) {
        fprintf(stderr, "wire verification failed round=%u method=%s bytes=%llu/%llu hash=%016llx/%016llx\n",
                round, mapped ? "mmap" : "pread", (unsigned long long)reader.received_bytes,
                (unsigned long long)reader.expected_bytes, (unsigned long long)reader.hash,
                (unsigned long long)expected_hash);
        exit(2);
    }
    *cpu_seconds = cpu_end - cpu_start;
    *wall_seconds = wall_end - wall_start;
}

int main(int argc, char **argv) {
    if (argc != 2) {
        fprintf(stderr, "usage: %s FIXTURE_PATH\n", argv[0]);
        return 2;
    }
    signal(SIGPIPE, SIG_IGN);
    create_fixture(argv[1]);
    int file_fd = open(argv[1], O_RDONLY | O_CLOEXEC);
    if (file_fd < 0) die("open source fixture");
    if (posix_fadvise(file_fd, 0, FILE_BYTES, POSIX_FADV_WILLNEED) != 0) {
        fprintf(stderr, "warning: posix_fadvise warm hint was unavailable\n");
    }
    unsigned char warm[BATCH_BYTES];
    for (uint64_t offset = 0; offset < FILE_BYTES; offset += sizeof(warm)) pread_exact(file_fd, warm, sizeof(warm), offset);
    uint64_t expected_hash = expected_wire_hash(file_fd);
    printf("round,method,cpu_seconds,cpu_seconds_per_gib,wall_seconds,verified_bytes,wire_hash\n");
    for (unsigned round = 1; round <= ROUNDS; round++) {
        int first = (round & 1u) ? 1 : 0;
        for (int item = 0; item < 2; item++) {
            int mapped = item == 0 ? first : 1 - first;
            double cpu = 0.0, wall = 0.0;
            run_method(file_fd, mapped, round, &cpu, &wall, expected_hash);
            double gib = (double)FILE_BYTES * (double)REPEATS / 1073741824.0;
            printf("%u,%s,%.9f,%.6f,%.6f,%llu,%016llx\n", round, mapped ? "mmap" : "pread",
                   cpu, cpu / gib, wall, (unsigned long long)((uint64_t)FILE_BYTES * REPEATS),
                   (unsigned long long)expected_hash);
            fflush(stdout);
        }
    }
    close(file_fd);
    return 0;
}
