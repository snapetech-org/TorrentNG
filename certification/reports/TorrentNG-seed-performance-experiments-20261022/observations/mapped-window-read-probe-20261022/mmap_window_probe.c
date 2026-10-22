#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>

enum { FILE_BYTES = 64 * 1024 * 1024, BLOCK_BYTES = 16 * 1024, BATCH_BYTES = 4 * BLOCK_BYTES, WINDOW_BYTES = 1024 * 1024, PASSES = 4, PAIRS = 3 };
struct result { double cpu, wall; uint64_t checksum; };
static double now(clockid_t id) { struct timespec t; if (clock_gettime(id, &t)) { perror("clock_gettime"); exit(1); } return (double)t.tv_sec + (double)t.tv_nsec / 1e9; }
static void fail(const char *what) { perror(what); exit(1); }
static uint64_t touch(const unsigned char *p, size_t n) { uint64_t s=0; for (size_t i=0;i<n;i++) s+=p[i]; return s; }
static struct result run_pread(int fd, unsigned passes) {
  unsigned char buffer[BATCH_BYTES]; struct result r={0}; double w=now(CLOCK_MONOTONIC), c=now(CLOCK_PROCESS_CPUTIME_ID);
  for (unsigned p=0;p<passes;p++) for (off_t o=0;o<FILE_BYTES;o+=BATCH_BYTES) {
    ssize_t n; do { n=pread(fd,buffer,BATCH_BYTES,o); } while(n<0 && errno==EINTR);
    if(n!=BATCH_BYTES) { if(n>=0) errno=EIO; fail("pread"); }
    r.checksum += touch(buffer,BATCH_BYTES);
  }
  r.cpu=now(CLOCK_PROCESS_CPUTIME_ID)-c; r.wall=now(CLOCK_MONOTONIC)-w; return r;
}
static struct result run_mmap(int fd, unsigned passes, size_t window) {
  struct result r={0}; double w=now(CLOCK_MONOTONIC), c=now(CLOCK_PROCESS_CPUTIME_ID);
  for(unsigned p=0;p<passes;p++) for(off_t base=0;base<FILE_BYTES;base+=(off_t)window) {
    size_t length=(size_t)((FILE_BYTES-base)<(off_t)window ? FILE_BYTES-base : (off_t)window);
    const unsigned char *m=mmap(NULL,length,PROT_READ,MAP_SHARED,fd,base);
    if(m==MAP_FAILED) fail("mmap");
    for(size_t o=0;o<length;o+=BLOCK_BYTES) r.checksum+=touch(m+o,BLOCK_BYTES);
    if(munmap((void*)m,length)) fail("munmap");
  }
  r.cpu=now(CLOCK_PROCESS_CPUTIME_ID)-c; r.wall=now(CLOCK_MONOTONIC)-w; return r;
}
static void report(unsigned pair,const char* method,size_t window,struct result r) {
  double gib=(double)FILE_BYTES*PASSES/1073741824.0;
  printf("pair=%u method=%s window_bytes=%zu bytes=%u cpu_s=%.6f wall_s=%.6f cpu_s_per_GiB=%.5f checksum=%" PRIu64 "\n",pair,method,window,(unsigned)(FILE_BYTES*PASSES),r.cpu,r.wall,r.cpu/gib,r.checksum);
}
int main(int argc,char**argv) {
  if(argc!=2){fprintf(stderr,"usage: %s EXISTING_HOT_FILE\n",argv[0]);return 2;}
  int fd=open(argv[1],O_RDONLY|O_CLOEXEC); if(fd<0)fail("open");
  struct result warm_p=run_pread(fd,1),warm_w=run_mmap(fd,1,WINDOW_BYTES);
  if(warm_p.checksum!=warm_w.checksum){fprintf(stderr,"warm checksums differ\n");return 1;}
  for(unsigned pair=1;pair<=PAIRS;pair++) {
    struct result a,b,d;
    if(pair==2){a=run_mmap(fd,PASSES,BATCH_BYTES);b=run_pread(fd,PASSES);d=run_mmap(fd,PASSES,WINDOW_BYTES);report(pair,"mmap_batch",BATCH_BYTES,a);report(pair,"pread_batch",BATCH_BYTES,b);report(pair,"mmap_piece_window",WINDOW_BYTES,d);}
    else {a=run_pread(fd,PASSES);b=run_mmap(fd,PASSES,BATCH_BYTES);d=run_mmap(fd,PASSES,WINDOW_BYTES);report(pair,"pread_batch",BATCH_BYTES,a);report(pair,"mmap_batch",BATCH_BYTES,b);report(pair,"mmap_piece_window",WINDOW_BYTES,d);}
    if(a.checksum!=b.checksum || a.checksum!=d.checksum){fprintf(stderr,"checksum mismatch in pair %u\n",pair);return 1;}
  }
  close(fd); return 0;
}
