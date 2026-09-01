// pubbench: isolate the cost of xsync's per-file publication sequence.
//
// Replays the exact syscall sequence run/sink.rs performs for a small file,
// and variants that remove one factor each, so the responsible factor can be
// named rather than guessed.
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <time.h>
#include <unistd.h>

static const char *HEX = "0123456789abcdef";

static int    n_files, n_dirs, n_threads, f_size;
static char   root[512];
static int    variant;
static char  *payload;

// variants
enum { V_XSYNC, V_SHORTNAME, V_FDMETA, V_NOMETA, V_RSYNC, V_DIRECT, V_NVARIANTS };
static const char *VNAME[] = {"xsync","shortname","fdmeta","nometa","rsync","direct"};

static double now(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec + ts.tv_nsec * 1e-9;
}

// 64 hex chars, like blake3 hex of the relative path
static void long_temp(char *out, size_t cap, const char *dir, unsigned long id) {
    // Must be unique per id, like blake3 of the relative path: derive every
    // hex digit from a full 64-bit mix, not from a few low bits of one.
    char hex[65];
    for (int b = 0; b < 8; b++) {
        unsigned long long z = id + 0x9e3779b97f4a7c15ULL * (unsigned long long)(b + 1);
        z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9ULL;
        z = (z ^ (z >> 27)) * 0x94d049bb133111ebULL;
        z ^= z >> 31;
        for (int k = 0; k < 8; k++) hex[b * 8 + k] = HEX[(z >> (k * 4)) & 0xf];
    }
    hex[64] = 0;
    snprintf(out, cap, "%s/.xsync.tmp.%s", dir, hex);
}

// rsync-ish: .<name>.XXXXXX
static void short_temp(char *out, size_t cap, const char *dir, unsigned long id) {
    snprintf(out, cap, "%s/.f%lu.tmp", dir, id);
}

static void *worker(void *arg) {
    long t = (long)arg;
    struct timespec times[2];
    times[0].tv_sec = 1700000000; times[0].tv_nsec = 0;   // atime
    times[1].tv_sec = 1700000000; times[1].tv_nsec = 0;   // mtime

    for (long i = t; i < n_files; i += n_threads) {
        char dir[600], fin[900], tmp[900];
        snprintf(dir, sizeof dir, "%s/d%03ld", root, i % n_dirs);
        snprintf(fin, sizeof fin, "%s/file%08ld.dat", dir, i);

        int use_temp = (variant != V_DIRECT);
        int fd_meta  = (variant == V_FDMETA || variant == V_RSYNC || variant == V_DIRECT);
        int no_meta  = (variant == V_NOMETA);
        int shortnm  = (variant == V_SHORTNAME || variant == V_RSYNC);

        if (use_temp) {
            if (shortnm) short_temp(tmp, sizeof tmp, dir, i);
            else         long_temp(tmp, sizeof tmp, dir, i);
        } else {
            strcpy(tmp, fin);
        }

        int fd = open(tmp, O_WRONLY | O_CREAT | O_TRUNC | O_NOFOLLOW, 0600);
        if (fd < 0) { fprintf(stderr, "open %s: %s\n", tmp, strerror(errno)); exit(1); }
        if (write(fd, payload, f_size) != f_size) { perror("write"); exit(1); }

        if (fd_meta && !no_meta) {
            if (futimens(fd, times) < 0) { perror("futimens"); exit(1); }
            if (fchmod(fd, 0644) < 0)    { perror("fchmod"); exit(1); }
        }
        close(fd);
        if (!fd_meta && !no_meta) {
            if (utimensat(AT_FDCWD, tmp, times, 0) < 0) { perror("utimensat"); exit(1); }
            if (chmod(tmp, 0644) < 0)                   { perror("chmod"); exit(1); }
        }
        if (use_temp && rename(tmp, fin) < 0) { perror("rename"); exit(1); }
    }
    return NULL;
}

int main(int argc, char **argv) {
    if (argc != 7) {
        fprintf(stderr, "usage: %s ROOT VARIANT NFILES NDIRS NTHREADS SIZE\n", argv[0]);
        return 2;
    }
    snprintf(root, sizeof root, "%s", argv[1]);
    variant = -1;
    for (int i = 0; i < V_NVARIANTS; i++)
        if (!strcmp(argv[2], VNAME[i])) variant = i;
    if (variant < 0) { fprintf(stderr, "bad variant\n"); return 2; }
    n_files = atoi(argv[3]); n_dirs = atoi(argv[4]);
    n_threads = atoi(argv[5]); f_size = atoi(argv[6]);

    payload = malloc(f_size);
    for (int i = 0; i < f_size; i++) payload[i] = (char)(i & 0xff);

    // Directory creation is off the timed path -- xsync caches it too.
    mkdir(root, 0755);
    for (int i = 0; i < n_dirs; i++) {
        char dir[600];
        snprintf(dir, sizeof dir, "%s/d%03d", root, i);
        if (mkdir(dir, 0755) < 0 && errno != EEXIST) { perror("mkdir"); return 1; }
    }

    pthread_t th[512];
    double t0 = now();
    for (long t = 0; t < n_threads; t++) pthread_create(&th[t], NULL, worker, (void *)t);
    for (long t = 0; t < n_threads; t++) pthread_join(th[t], NULL);
    double t1 = now();
    // syncfs, not sync: a global sync would also flush the other filesystem
    // and the previous cell's teardown into this cell's timer.
    int rfd = open(root, O_RDONLY | O_DIRECTORY);
    if (rfd < 0 || syncfs(rfd) < 0) { perror("syncfs"); return 1; }
    close(rfd);
    double t2 = now();

    printf("%-10s threads=%-3d create=%7.3f sync=%7.3f total=%7.3f  files/s=%9.0f\n",
           VNAME[variant], n_threads, t1 - t0, t2 - t1, t2 - t0, n_files / (t2 - t0));
    return 0;
}
