#define _GNU_SOURCE
#include <fcntl.h>
#include <malloc.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

/* Diagnostic preload only. One explicit FIFO request after recheck triggers
 * allocator observation/reclamation on a helper thread. Never used for scores. */
static void *observe(void *unused) {
    (void)unused;
    const char *fifo = getenv("PROBE_TRIM_FIFO");
    const char *result = getenv("PROBE_TRIM_RESULT");
    if (!fifo || !result) return NULL;
    int fd = open(fifo, O_RDONLY);
    char command = 0;
    if (fd < 0 || read(fd, &command, 1) != 1 || command != 'T') {
        if (fd >= 0) close(fd);
        return NULL;
    }
    close(fd);
    struct mallinfo2 before = mallinfo2();
    int reclaimed = malloc_trim(0);
    struct mallinfo2 after = mallinfo2();
    FILE *out = fopen(result, "w");
    if (out) {
        fprintf(out, "{\"diagnostic_only\":true,\"reclaimed\":%d,"
                "\"before\":{\"arena\":%zu,\"used\":%zu,\"free\":%zu,\"mmap\":%zu},"
                "\"after\":{\"arena\":%zu,\"used\":%zu,\"free\":%zu,\"mmap\":%zu}}\n",
                reclaimed, before.arena, before.uordblks, before.fordblks, before.hblkhd,
                after.arena, after.uordblks, after.fordblks, after.hblkhd);
        fclose(out);
    }
    return NULL;
}

__attribute__((constructor)) static void initialize(void) {
    pthread_t worker;
    if (pthread_create(&worker, NULL, observe, NULL) == 0) pthread_detach(worker);
}
