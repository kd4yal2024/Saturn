// Test-only LD_PRELOAD shim: lets the unmodified Saturn Bridge receive a paced,
// repeatable stream with no XDMA hardware.
//
// It never ships with the Bridge and is loaded only by run_replay_bridge.sh.
// It does three things:
//
//  1. pread()/pread64() on the placeholder file named by SATURN_REPLAY_RX_DEVICE
//     return DDC frames in the exact format the Bridge's parser expects, paced
//     in real time at 384 kS/s (3,456,000 bytes/s). The real device ignores the
//     offset; a plain file cannot do that, which is why this is needed.
//  2. sched_setscheduler()/sched_getscheduler()/sched_getparam() pretend that
//     SCHED_FIFO was granted, because the Bridge refuses to start its reader
//     and TX threads without it and an unprivileged user cannot have it.
//     Nothing is actually boosted; thread timing is ordinary.
//  3. close() clears the per-descriptor cache.
//  4. Optionally (SATURN_REPLAY_TCP_NODELAY=1) sets TCP_NODELAY on every
//     accepted socket. The Bridge sets it nowhere; this exists only to run
//     the experiment "does the audio arrive more evenly with it" without
//     changing the Bridge. It is independent of the receive replay.
//
// Everything else reaches the real libc. No register or DUC behavior is
// emulated here: the register space is an ordinary file made by
// make_register_file.py and the DUC device is /dev/null.
//
// Environment:
//   SATURN_REPLAY_RX_DEVICE    path of the placeholder file (required)
//   SATURN_REPLAY_SOURCE       "synthetic" (default) or a raw DMA capture file,
//                              replayed in a loop
//   SATURN_REPLAY_TONES        synthetic carriers as "offset_hz:dbfs,..."
//                              (default "1500:-30")
//   SATURN_REPLAY_NOISE_DBFS   white noise level (default -100)
//   SATURN_REPLAY_SEED         noise seed (default 1)
//   SATURN_REPLAY_PACING       "realtime" (default) or "free" (as fast as asked)
//   SATURN_REPLAY_FAKE_RT      "1" (default) to fake the scheduler calls
//   SATURN_REPLAY_STATS_PATH   write a small JSON summary here at exit
//   SATURN_REPLAY_TCP_NODELAY  "1" to set TCP_NODELAY on accepted sockets

#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <math.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <sys/socket.h>
#include <pthread.h>
#include <sched.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

#define FRAME_BYTES 72           // 8-byte header + 8 sample words of 8 bytes
#define SAMPLES_PER_FRAME 8
#define SAMPLE_RATE_HZ 384000.0
#define BYTES_PER_SECOND 3456000ULL   // 384000 / 8 frames * 72 bytes
#define RATE_WORD 0x00100000u    // rate code 4 on DDC6 (4 << (6 * 3))
#define FULL_SCALE 8388607.0     // 24-bit signed
#define MAX_TONES 16
#define FD_CACHE 4096

typedef struct { double step; double phase; double amplitude; } Tone;

static int configured;
static dev_t target_dev;
static ino_t target_ino;
static uint8_t fd_state[FD_CACHE];   // 0 unknown, 1 replay device, 2 other
static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;

static Tone tones[MAX_TONES];
static int tone_count;
static double noise_amplitude;
static uint64_t rng = 1;
static int realtime_pacing = 1;
static int fake_rt = 1;

static uint8_t *capture;             // optional recording, replayed in a loop
static size_t capture_len;
static size_t capture_pos;

static uint8_t frame[FRAME_BYTES];
static size_t frame_off = FRAME_BYTES;
static uint32_t frame_counter;

static struct timespec t0;
static int t0_set;
static uint64_t served_bytes;
static uint64_t read_calls;
static uint64_t frames_made;
static uint64_t slept_ns;
static const char *stats_path;

static __thread int fake_policy = SCHED_OTHER;
static __thread int fake_priority;

static int nodelay_accepted;
static int (*real_accept4)(int, struct sockaddr *, socklen_t *, int);
static int (*real_accept)(int, struct sockaddr *, socklen_t *);
static ssize_t (*real_pread64)(int, void *, size_t, off_t);
static ssize_t (*real_pread)(int, void *, size_t, off_t);
static int (*real_close)(int);
static int (*real_setscheduler)(pid_t, int, const struct sched_param *);
static int (*real_getscheduler)(pid_t);
static int (*real_getparam)(pid_t, struct sched_param *);

static double dbfs_to_amplitude(double dbfs) { return pow(10.0, dbfs / 20.0); }

static double next_noise(void) {
  // xorshift64*, mapped to [-1, 1)
  rng ^= rng >> 12;
  rng ^= rng << 25;
  rng ^= rng >> 27;
  uint64_t v = rng * 0x2545F4914F6CDD1DULL;
  return ((double)(v >> 11) / 4503599627370496.0) - 1.0;
}

static void parse_tones(const char *spec) {
  char *copy = strdup(spec);
  char *save = NULL;
  for (char *item = strtok_r(copy, ",", &save); item && tone_count < MAX_TONES;
       item = strtok_r(NULL, ",", &save)) {
    double offset = 0, dbfs = -30;
    if (sscanf(item, "%lf:%lf", &offset, &dbfs) >= 1) {
      tones[tone_count].step = 2.0 * M_PI * offset / SAMPLE_RATE_HZ;
      tones[tone_count].phase = 0.0;
      tones[tone_count].amplitude = dbfs_to_amplitude(dbfs);
      tone_count++;
    }
  }
  free(copy);
}

static void write_stats(void) {
  // Child processes of the Bridge (it runs systemctl) load this library too;
  // only the process that actually served data reports.
  if (!stats_path || read_calls == 0) return;
  FILE *f = fopen(stats_path, "w");
  if (!f) return;
  fprintf(f,
          "{\"read_calls\":%llu,\"served_bytes\":%llu,\"frames_generated\":%llu,"
          "\"slept_ms\":%llu,\"source\":\"%s\",\"pacing\":\"%s\"}\n",
          (unsigned long long)read_calls, (unsigned long long)served_bytes,
          (unsigned long long)frames_made, (unsigned long long)(slept_ns / 1000000ULL),
          capture ? "file" : "synthetic", realtime_pacing ? "realtime" : "free");
  fclose(f);
}

__attribute__((constructor)) static void shim_init(void) {
  real_pread64 = dlsym(RTLD_NEXT, "pread64");
  real_pread = dlsym(RTLD_NEXT, "pread");
  real_close = dlsym(RTLD_NEXT, "close");
  real_setscheduler = dlsym(RTLD_NEXT, "sched_setscheduler");
  real_getscheduler = dlsym(RTLD_NEXT, "sched_getscheduler");
  real_getparam = dlsym(RTLD_NEXT, "sched_getparam");
  real_accept4 = dlsym(RTLD_NEXT, "accept4");
  real_accept = dlsym(RTLD_NEXT, "accept");
  const char *nodelay = getenv("SATURN_REPLAY_TCP_NODELAY");
  nodelay_accepted = nodelay && strcmp(nodelay, "1") == 0;

  const char *fake = getenv("SATURN_REPLAY_FAKE_RT");
  fake_rt = !(fake && strcmp(fake, "0") == 0);

  const char *device = getenv("SATURN_REPLAY_RX_DEVICE");
  if (!device) return;           // loaded but not configured: pass everything through
  struct stat st;
  if (stat(device, &st) != 0) {
    fprintf(stderr, "xdma_replay_shim: SATURN_REPLAY_RX_DEVICE %s: %s\n", device, strerror(errno));
    return;
  }
  target_dev = st.st_dev;
  target_ino = st.st_ino;

  const char *pacing = getenv("SATURN_REPLAY_PACING");
  realtime_pacing = !(pacing && strcmp(pacing, "free") == 0);
  const char *seed = getenv("SATURN_REPLAY_SEED");
  rng = (seed ? strtoull(seed, NULL, 10) : 1ULL) * 0x9E3779B97F4A7C15ULL + 1;
  if (!rng) rng = 1;
  const char *noise = getenv("SATURN_REPLAY_NOISE_DBFS");
  noise_amplitude = dbfs_to_amplitude(noise ? atof(noise) : -100.0);
  stats_path = getenv("SATURN_REPLAY_STATS_PATH");

  const char *source = getenv("SATURN_REPLAY_SOURCE");
  if (source && strcmp(source, "synthetic") != 0) {
    FILE *f = fopen(source, "rb");
    if (!f) {
      fprintf(stderr, "xdma_replay_shim: cannot open SATURN_REPLAY_SOURCE %s: %s\n", source, strerror(errno));
      return;
    }
    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    fseek(f, 0, SEEK_SET);
    if (size < FRAME_BYTES) {
      fprintf(stderr, "xdma_replay_shim: %s is shorter than one frame\n", source);
      fclose(f);
      return;
    }
    capture = malloc((size_t)size);
    capture_len = capture && fread(capture, 1, (size_t)size, f) == (size_t)size ? (size_t)size : 0;
    fclose(f);
    if (!capture_len) return;
  } else {
    parse_tones(getenv("SATURN_REPLAY_TONES") ? getenv("SATURN_REPLAY_TONES") : "1500:-30");
  }
  configured = 1;
  fprintf(stderr,
          "xdma_replay_shim: active: device=%s source=%s pacing=%s tones=%d fake_rt=%d\n",
          device, capture ? "file" : "synthetic", realtime_pacing ? "realtime" : "free",
          tone_count, fake_rt);
}

__attribute__((destructor)) static void shim_fini(void) {
  if (configured) write_stats();
}

static int is_replay_fd(int fd) {
  if (!configured || fd < 0 || fd >= FD_CACHE) return 0;
  uint8_t state = __atomic_load_n(&fd_state[fd], __ATOMIC_RELAXED);
  if (state) return state == 1;
  struct stat st;
  if (fstat(fd, &st) != 0) return 0;
  int match = st.st_ino == target_ino && st.st_dev == target_dev;
  __atomic_store_n(&fd_state[fd], match ? 1 : 2, __ATOMIC_RELAXED);
  return match;
}

static void put_sample24(uint8_t *out, double value) {
  double scaled = value * FULL_SCALE;
  if (scaled > FULL_SCALE) scaled = FULL_SCALE;
  if (scaled < -FULL_SCALE) scaled = -FULL_SCALE;
  int32_t v = (int32_t)lrint(scaled);
  out[0] = (uint8_t)((v >> 16) & 0xff);   // big-endian, as after the network byte order switch
  out[1] = (uint8_t)((v >> 8) & 0xff);
  out[2] = (uint8_t)(v & 0xff);
}

static void make_frame(void) {
  memset(frame, 0, sizeof(frame));
  uint32_t word = RATE_WORD;
  memcpy(frame, &word, 4);                    // rate word, little-endian
  frame[4] = (uint8_t)(frame_counter & 0xff);  // free-running counter (not checked)
  frame[5] = (uint8_t)((frame_counter >> 8) & 0xff);
  frame[6] = (uint8_t)((frame_counter >> 16) & 0xff);
  frame[7] = 0x80;                             // header marker
  frame_counter++;
  for (int s = 0; s < SAMPLES_PER_FRAME; s++) {
    double i = 0.0, q = 0.0;
    for (int t = 0; t < tone_count; t++) {
      i += tones[t].amplitude * cos(tones[t].phase);
      q += tones[t].amplitude * sin(tones[t].phase);
      tones[t].phase += tones[t].step;
      if (tones[t].phase > M_PI) tones[t].phase -= 2.0 * M_PI;
      if (tones[t].phase < -M_PI) tones[t].phase += 2.0 * M_PI;
    }
    if (noise_amplitude > 0.0) {
      i += noise_amplitude * next_noise();
      q += noise_amplitude * next_noise();
    }
    uint8_t *word_out = frame + 8 + (size_t)s * 8;
    put_sample24(word_out, i);
    put_sample24(word_out + 3, q);
  }
  frames_made++;
  frame_off = 0;
}

static void fill(uint8_t *dst, size_t n) {
  if (capture) {
    while (n) {
      size_t take = capture_len - capture_pos;
      if (take > n) take = n;
      memcpy(dst, capture + capture_pos, take);
      capture_pos = (capture_pos + take) % capture_len;
      dst += take;
      n -= take;
    }
    return;
  }
  while (n) {
    if (frame_off >= FRAME_BYTES) make_frame();
    size_t take = FRAME_BYTES - frame_off;
    if (take > n) take = n;
    memcpy(dst, frame + frame_off, take);
    frame_off += take;
    dst += take;
    n -= take;
  }
}

// Block until `bytes` more bytes would have arrived at the hardware rate.
static void pace(size_t bytes) {
  if (!realtime_pacing) return;
  if (!t0_set) {
    clock_gettime(CLOCK_MONOTONIC, &t0);
    t0_set = 1;
  }
  uint64_t due_ns = (uint64_t)(((__uint128_t)(served_bytes + bytes) * 1000000000ULL) / BYTES_PER_SECOND);
  struct timespec target = t0;
  target.tv_sec += (time_t)(due_ns / 1000000000ULL);
  target.tv_nsec += (long)(due_ns % 1000000000ULL);
  if (target.tv_nsec >= 1000000000L) {
    target.tv_sec++;
    target.tv_nsec -= 1000000000L;
  }
  struct timespec before, after;
  clock_gettime(CLOCK_MONOTONIC, &before);
  while (clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &target, NULL) == EINTR) {
  }
  clock_gettime(CLOCK_MONOTONIC, &after);
  int64_t slept = (int64_t)(after.tv_sec - before.tv_sec) * 1000000000LL + (after.tv_nsec - before.tv_nsec);
  if (slept > 0) slept_ns += (uint64_t)slept;
}

static ssize_t serve(void *buf, size_t n) {
  pthread_mutex_lock(&lock);
  pace(n);
  fill((uint8_t *)buf, n);
  served_bytes += n;
  read_calls++;
  pthread_mutex_unlock(&lock);
  return (ssize_t)n;
}

ssize_t pread64(int fd, void *buf, size_t count, off_t offset) {
  if (is_replay_fd(fd)) return serve(buf, count);
  if (!real_pread64) real_pread64 = dlsym(RTLD_NEXT, "pread64");
  return real_pread64(fd, buf, count, offset);
}

ssize_t pread(int fd, void *buf, size_t count, off_t offset) {
  if (is_replay_fd(fd)) return serve(buf, count);
  if (!real_pread) real_pread = dlsym(RTLD_NEXT, "pread");
  return real_pread(fd, buf, count, offset);
}

int close(int fd) {
  if (fd >= 0 && fd < FD_CACHE) __atomic_store_n(&fd_state[fd], 0, __ATOMIC_RELAXED);
  if (!real_close) real_close = dlsym(RTLD_NEXT, "close");
  return real_close(fd);
}

int sched_setscheduler(pid_t pid, int policy, const struct sched_param *param) {
  if (fake_rt && pid == 0 && (policy == SCHED_FIFO || policy == SCHED_RR) && param) {
    fake_policy = policy;
    fake_priority = param->sched_priority;
    return 0;
  }
  if (!real_setscheduler) real_setscheduler = dlsym(RTLD_NEXT, "sched_setscheduler");
  return real_setscheduler(pid, policy, param);
}

int sched_getscheduler(pid_t pid) {
  if (fake_rt && pid == 0 && fake_policy != SCHED_OTHER) return fake_policy;
  if (!real_getscheduler) real_getscheduler = dlsym(RTLD_NEXT, "sched_getscheduler");
  return real_getscheduler(pid);
}

int sched_getparam(pid_t pid, struct sched_param *param) {
  if (fake_rt && pid == 0 && fake_policy != SCHED_OTHER && param) {
    param->sched_priority = fake_priority;
    return 0;
  }
  if (!real_getparam) real_getparam = dlsym(RTLD_NEXT, "sched_getparam");
  return real_getparam(pid, param);
}

static int with_nodelay(int accepted) {
  if (accepted >= 0 && nodelay_accepted) {
    int one = 1;
    setsockopt(accepted, IPPROTO_TCP, TCP_NODELAY, &one, sizeof(one));
  }
  return accepted;
}

int accept4(int fd, struct sockaddr *addr, socklen_t *len, int flags) {
  if (!real_accept4) real_accept4 = dlsym(RTLD_NEXT, "accept4");
  return with_nodelay(real_accept4(fd, addr, len, flags));
}

int accept(int fd, struct sockaddr *addr, socklen_t *len) {
  if (!real_accept) real_accept = dlsym(RTLD_NEXT, "accept");
  return with_nodelay(real_accept(fd, addr, len));
}
