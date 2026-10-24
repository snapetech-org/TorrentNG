#define _GNU_SOURCE
#include <fcntl.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
extern int ProfilerStart(const char *path);
extern void ProfilerStop(void);

/* Diagnostic helper; no allocator replacement and no debugger attachment. */
static void *control(void *unused) {
    (void)unused;
    const char *fifo = getenv("PROBE_PROFILE_FIFO");
    const char *ack = getenv("PROBE_PROFILE_ACK");
    const char *profile = getenv("PROBE_PROFILE_PATH");
    if (!fifo || !ack || !profile) return NULL;
    for (int index = 0; index < 2; ++index) {
        int fd = open(fifo, O_RDONLY);
        char action = 0;
        if (fd < 0 || read(fd, &action, 1) != 1) return NULL;
        close(fd);
        int ok = 1;
        if (action == 'S') ok = ProfilerStart(profile);
        else if (action == 'E') ProfilerStop();
        else ok = 0;
        FILE *out = fopen(ack, "w");
        if (!out) return NULL;
        fprintf(out, "%c %d\n", action, ok);
        fclose(out);
    }
    return NULL;
}
__attribute__((constructor)) static void initialize(void) {
    pthread_t worker;
    if (pthread_create(&worker, NULL, control, NULL) == 0) pthread_detach(worker);
}
