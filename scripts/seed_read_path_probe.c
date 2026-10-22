#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

enum {
    BLOCK_BYTES = 16 * 1024,
    FILE_BYTES = 64 * 1024 * 1024,
    WARM_PASSES = 1,
    MEASURE_PASSES = 4,
    PAIRS = 3,
};

struct result {
    double cpu_seconds;
    double wall_seconds;
    uint64_t checksum;
};

static void fail(const char *what) {
    fprintf(stderr, "%s: %s\n", what, strerror(errno));
    exit(EXIT_FAILURE);
}

static double now(clockid_t clock_id) {
    struct timespec value;
    if (clock_gettime(clock_id, &value) != 0) {
        fail("clock_gettime");
    }
    return (double)value.tv_sec + (double)value.tv_nsec / 1e9;
}

static uint64_t touch_bytes(const unsigned char *data, size_t length) {
    uint64_t sum = 0;
    for (size_t index = 0; index < length; index++) {
        sum += data[index];
    }
    return sum;
}

static struct result run_pread(int fd, unsigned char *buffer, unsigned passes) {
    struct result result = {0};
    double wall_start = now(CLOCK_MONOTONIC);
    double cpu_start = now(CLOCK_PROCESS_CPUTIME_ID);
    for (unsigned pass = 0; pass < passes; pass++) {
        for (off_t offset = 0; offset < FILE_BYTES; offset += BLOCK_BYTES) {
            ssize_t count;
            do {
                count = pread(fd, buffer, BLOCK_BYTES, offset);
            } while (count < 0 && errno == EINTR);
            if (count != BLOCK_BYTES) {
                if (count >= 0) {
                    errno = EIO;
                }
                fail("pread");
            }
            result.checksum += touch_bytes(buffer, BLOCK_BYTES);
        }
    }
    result.cpu_seconds = now(CLOCK_PROCESS_CPUTIME_ID) - cpu_start;
    result.wall_seconds = now(CLOCK_MONOTONIC) - wall_start;
    return result;
}

static struct result run_mmap(const unsigned char *mapping, unsigned passes) {
    struct result result = {0};
    double wall_start = now(CLOCK_MONOTONIC);
    double cpu_start = now(CLOCK_PROCESS_CPUTIME_ID);
    for (unsigned pass = 0; pass < passes; pass++) {
        for (size_t offset = 0; offset < FILE_BYTES; offset += BLOCK_BYTES) {
            result.checksum += touch_bytes(mapping + offset, BLOCK_BYTES);
        }
    }
    result.cpu_seconds = now(CLOCK_PROCESS_CPUTIME_ID) - cpu_start;
    result.wall_seconds = now(CLOCK_MONOTONIC) - wall_start;
    return result;
}

static void write_fixture(int fd) {
    const size_t chunk_bytes = 1024 * 1024;
    unsigned char *chunk = malloc(chunk_bytes);
    if (chunk == NULL) {
        fail("malloc");
    }
    uint32_t state = 0x6d2b79f5u;
    for (size_t index = 0; index < chunk_bytes; index++) {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        chunk[index] = (unsigned char)(state >> 24);
    }
    for (off_t offset = 0; offset < FILE_BYTES; offset += (off_t)chunk_bytes) {
        size_t written = 0;
        while (written < chunk_bytes) {
            ssize_t count = pwrite(fd, chunk + written, chunk_bytes - written,
                                   offset + (off_t)written);
            if (count < 0 && errno == EINTR) {
                continue;
            }
            if (count <= 0) {
                fail("pwrite fixture");
            }
            written += (size_t)count;
        }
    }
    free(chunk);
    if (fdatasync(fd) != 0) {
        fail("fdatasync fixture");
    }
}

static void print_result(unsigned pair, const char *path, struct result result) {
    const double gib = (double)FILE_BYTES * MEASURE_PASSES / 1073741824.0;
    printf("pair=%u method=%s bytes=%" PRIu64
           " cpu_s=%.6f wall_s=%.6f cpu_s_per_GiB=%.4f checksum=%" PRIu64 "\n",
           pair, path, (uint64_t)FILE_BYTES * MEASURE_PASSES,
           result.cpu_seconds, result.wall_seconds,
           result.cpu_seconds / gib, result.checksum);
}

int main(int argc, char **argv) {
    if (argc != 2) {
        fprintf(stderr, "usage: %s NEW_TASK_LOCAL_FILE_PATH\n", argv[0]);
        return EXIT_FAILURE;
    }
    const char *path = argv[1];
    int fd = open(path, O_CREAT | O_EXCL | O_RDWR | O_CLOEXEC, 0600);
    if (fd < 0) {
        fail("create fixture (path must not already exist)");
    }
    write_fixture(fd);

    unsigned char *mapping = mmap(NULL, FILE_BYTES, PROT_READ, MAP_SHARED, fd, 0);
    if (mapping == MAP_FAILED) {
        fail("mmap");
    }
    unsigned char *buffer = malloc(BLOCK_BYTES);
    if (buffer == NULL) {
        fail("malloc read buffer");
    }

    // Warm the file and the mapping before timing; this isolates steady-state
    // user-space read-path cost from fixture creation and first-touch faults.
    struct result warm_pread = run_pread(fd, buffer, WARM_PASSES);
    struct result warm_mmap = run_mmap(mapping, WARM_PASSES);
    if (warm_pread.checksum != warm_mmap.checksum) {
        fprintf(stderr, "warm-up checksums differed\n");
        return EXIT_FAILURE;
    }

    for (unsigned pair = 1; pair <= PAIRS; pair++) {
        struct result first;
        struct result second;
        if (pair == 2) {
            first = run_mmap(mapping, MEASURE_PASSES);
            second = run_pread(fd, buffer, MEASURE_PASSES);
            print_result(pair, "mmap", first);
            print_result(pair, "pread", second);
        } else {
            first = run_pread(fd, buffer, MEASURE_PASSES);
            second = run_mmap(mapping, MEASURE_PASSES);
            print_result(pair, "pread", first);
            print_result(pair, "mmap", second);
        }
        if (first.checksum != second.checksum) {
            fprintf(stderr, "pair %u checksums differed\n", pair);
            return EXIT_FAILURE;
        }
    }

    free(buffer);
    if (munmap(mapping, FILE_BYTES) != 0) {
        fail("munmap");
    }
    if (close(fd) != 0) {
        fail("close fixture");
    }
    if (unlink(path) != 0) {
        fail("unlink fixture");
    }
    return EXIT_SUCCESS;
}
