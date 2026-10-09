#define _POSIX_C_SOURCE 200809L
#include "fpga_rx_counter_v31.h"

#include <inttypes.h>
#include <fcntl.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#include "hwaccess.h"

#define RXC1_BASE 0x8000U
#define RXC1_MAGIC 0x52584331U
#define RXC1_CANDIDATE_BUILD_ID 0x53460004U
#define RXC1_REQUEST 0x80000000U
#define RXC1_ACK 0x40000000U
#define RXC1_VALID 0x01U
#define RXC1_OVERFLOW 0x04U
#define RXC1_RESET 0x08U
#define RXC1_EXHAUSTED 0x30U
#define RXC1_TOKEN_BOUND 0x40U
#define RXC1_TRIAL_ARM "RXC1-TRIAL-0x53460004\n"
#define RXC1_TRIAL_MAX_AGE_SECONDS 600

static TRXC1Snapshot g_snapshot;
static uint32_t g_token;
static uint32_t g_session;
static bool g_bound;
static time_t g_last_sample_second;
static uint32_t g_token_sequence;
static bool g_poll_enabled;

/* Spend the durable arm before any RXC1 BAR access. A hard reset or process
 * restart then comes back with polling off, even when the unit still says 1.
 * The installer creates the StateDirectory but never creates "armed". */
static bool ConsumeTrialArm(void)
{
  const char *Directory = getenv("STATE_DIRECTORY");
  char Contents[sizeof(RXC1_TRIAL_ARM)];
  struct stat Stat;
  int DirectoryFd, ArmFd;
  ssize_t Length;
  bool Allowed = false;
  bool ValidArm;
  time_t Now;

  if (Directory == NULL || Directory[0] != '/' || strchr(Directory, ':') != NULL)
    return false;
  DirectoryFd = open(Directory, O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW);
  if (DirectoryFd < 0) return false;
  ArmFd = openat(DirectoryFd, "armed", O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
  if (ArmFd >= 0)
  {
    Length = read(ArmFd, Contents, sizeof(Contents));
    Now = time(NULL);
    ValidArm = fstat(ArmFd, &Stat) == 0 && S_ISREG(Stat.st_mode) &&
               Stat.st_uid == geteuid() &&
               Length == (ssize_t)(sizeof(RXC1_TRIAL_ARM) - 1U) &&
               memcmp(Contents, RXC1_TRIAL_ARM, sizeof(RXC1_TRIAL_ARM) - 1U) == 0 &&
               Now >= RXC1_TRIAL_MAX_AGE_SECONDS &&
               Stat.st_mtime <= Now &&
               Stat.st_mtime >= Now - RXC1_TRIAL_MAX_AGE_SECONDS;
    /* Spend invalid arms too: a later clock correction or image change must
     * not turn an old request into an automatic hardware probe. */
    if (renameat(DirectoryFd, "armed", DirectoryFd, "spent") == 0 &&
        fsync(ArmFd) == 0 && fsync(DirectoryFd) == 0)
      Allowed = ValidArm;
    close(ArmFd);
  }
  close(DirectoryFd);
  return Allowed;
}

static uint64_t NowMs(void)
{
  struct timespec Now;
  if (clock_gettime(CLOCK_REALTIME, &Now) != 0)
    return 0;
  return (uint64_t)Now.tv_sec * 1000U + (uint64_t)Now.tv_nsec / 1000000U;
}

static uint32_t NewToken(uint32_t OldToken)
{
  struct timespec Now = {0};
  uint32_t Token;
  (void)clock_gettime(CLOCK_REALTIME, &Now);
  g_token_sequence++;
  Token = (uint32_t)Now.tv_nsec ^ (uint32_t)Now.tv_sec ^
          ((uint32_t)getpid() << 16) ^ (g_token_sequence * 0x9e3779b9U);
  if (Token == 0 || Token == OldToken)
    Token = OldToken + 1U;
  if (Token == 0)
    Token = 1U;
  return Token;
}

static bool Read(uint32_t Offset, uint32_t *Value)
{
  return RegisterReadChecked(RXC1_BASE + Offset, Value);
}

static bool Write(uint32_t Offset, uint32_t Value)
{
  return RegisterWriteChecked(RXC1_BASE + Offset, Value);
}

static bool Read64(uint32_t Offset, uint64_t *Value)
{
  uint32_t Low, High;
  if (!Read(Offset, &Low) || !Read(Offset + 4U, &High))
    return false;
  *Value = (uint64_t)Low | ((uint64_t)High << 32);
  return true;
}

static void Invalidate(TRXC1DDC *DDC, unsigned int Receiver, ERXC1Status Status)
{
  memset(DDC, 0, sizeof(*DDC));
  DDC->Receiver = Receiver;
  DDC->Status = Status;
}

void RXC1Init(uint32_t FpgaBuildId)
{
  unsigned int Receiver;
  const char *PollEnabled = getenv("SATURN_RXC1_POLL_ENABLED");
  bool PollRequested = PollEnabled != NULL && strcmp(PollEnabled, "1") == 0;
  ERXC1Status InitialStatus;
  /* An RXC1 BAR read on an image without that register bank may not complete.
   * Never probe the known 0x53460003 baseline or an unidentified image. */
  g_poll_enabled = PollRequested && FpgaBuildId == RXC1_CANDIDATE_BUILD_ID &&
                   ConsumeTrialArm();
  if (PollRequested && FpgaBuildId == RXC1_CANDIDATE_BUILD_ID && !g_poll_enabled)
    fprintf(stderr, "p2app: RXC1 polling refused: no durable one-use trial arm\n");
  if (g_poll_enabled)
    fprintf(stderr, "p2app: RXC1 one-use trial arm spent before polling\n");
  InitialStatus = g_poll_enabled ? eRXC1Unavailable :
                  !PollRequested ? eRXC1Disabled :
                  FpgaBuildId != RXC1_CANDIDATE_BUILD_ID ? eRXC1Unsupported :
                  eRXC1Unarmed;
  memset(&g_snapshot, 0, sizeof(g_snapshot));
  for (Receiver = 0; Receiver < RXC1_DDC_COUNT; Receiver++)
    Invalidate(&g_snapshot.DDC[Receiver], Receiver, InitialStatus);
  g_bound = false;
  g_token = 0;
  g_session = 0;
  g_last_sample_second = 0;
}

static ERXC1Status Bind(void)
{
  uint32_t Magic, Status, Session, Serial, OldToken, Token;
  if (!Read(0x00U, &Magic)) return eRXC1ReadError;
  if (Magic != RXC1_MAGIC) return eRXC1Unsupported;
  if (!Read(0x08U, &Status) || !Read(0x0cU, &Session) ||
      !Read(0x10U, &Serial) || !Read(0x4cU, &OldToken)) return eRXC1ReadError;
  if (Status & RXC1_RESET) return eRXC1ResetActive;
  if (Status & RXC1_EXHAUSTED) return eRXC1GenerationExhausted;
  if (Status & RXC1_VALID)
  {
    if (!Write(0x04U, RXC1_ACK) || !Read(0x08U, &Status) ||
        (Status & RXC1_VALID)) return eRXC1AckFailed;
  }
  Token = NewToken(OldToken);
  if (!Write(0x4cU, Token)) return eRXC1WriteError;
  if (!Read(0x4cU, &OldToken) || !Read(0x0cU, &Serial) ||
      !Read(0x08U, &Status)) return eRXC1ReadError;
  if (OldToken != Token) return eRXC1TokenMismatch;
  if (Serial != Session || (Status & RXC1_RESET)) return eRXC1ResetChanged;
  g_token = Token;
  g_session = Session;
  g_bound = true;
  return eRXC1Valid;
}

static ERXC1Status Acquire(unsigned int Receiver, TRXC1DDC *Result)
{
  uint32_t BeforeStatus, Session, PreviousSerial, Token, Status, Serial;
  uint32_t FrozenSession, ConfigurationGeneration, Metadata, FrozenToken;
  uint32_t FrozenOverflow, EndStatus, Word;
  uint64_t Configuration, CheckConfiguration;
  unsigned int Counter;
  ERXC1Status Outcome = eRXC1ReadError;
  TRXC1DDC Candidate;

  if (!Read(0x08U, &BeforeStatus)) return eRXC1ReadError;
  if (BeforeStatus & RXC1_RESET) return eRXC1ResetActive;
  if (BeforeStatus & RXC1_EXHAUSTED) return eRXC1GenerationExhausted;
  if (BeforeStatus & RXC1_VALID) return eRXC1StaleSnapshot;
  if (!(BeforeStatus & RXC1_TOKEN_BOUND)) return eRXC1TokenMismatch;
  if (!Read(0x4cU, &Token) || !Read(0x0cU, &Session) ||
      !Read(0x10U, &PreviousSerial)) return eRXC1ReadError;
  if (Token != g_token) return eRXC1TokenMismatch;
  if (Session != g_session) return eRXC1ResetChanged;
  if (PreviousSerial == UINT32_MAX) return eRXC1GenerationExhausted;
  if (!Write(0x04U, RXC1_REQUEST | Receiver)) return eRXC1WriteError;

  do
  {
    if (!Read(0x08U, &Status) || !Read(0x10U, &Serial)) break;
    if (Status & RXC1_RESET) { Outcome = eRXC1ResetChanged; break; }
    if (Status & RXC1_EXHAUSTED) { Outcome = eRXC1GenerationExhausted; break; }
    if (!(Status & RXC1_VALID)) { Outcome = eRXC1SnapshotUnavailable; break; }
    if (Serial != PreviousSerial + 1U) { Outcome = eRXC1StaleSerial; break; }
    if (!Read(0x14U, &FrozenSession) ||
        !Read(0x18U, &ConfigurationGeneration) ||
        !Read(0x1cU, &Metadata) || !Read(0x50U, &FrozenToken) ||
        !Read64(0x54U, &Configuration)) break;
    if (FrozenSession != Session) { Outcome = eRXC1ResetChanged; break; }
    if (ConfigurationGeneration == 0 || Configuration >> 39 != 0)
    { Outcome = eRXC1ConfigurationMismatch; break; }
    if ((Metadata & 0xFFFFFF80U) || (Metadata & 0x0fU) != Receiver)
    { Outcome = eRXC1ReceiverMismatch; break; }
    if (FrozenToken != g_token) { Outcome = eRXC1TokenMismatch; break; }
    Invalidate(&Candidate, Receiver, eRXC1Valid);
    Candidate.SnapshotSerial = Serial;
    Candidate.HostToken = g_token;
    Candidate.SessionGeneration = Session;
    Candidate.ConfigurationGeneration = ConfigurationGeneration;
    Candidate.ObservedConfigurationWord = Configuration;
    Candidate.RateCode = (Metadata >> 4) & 7U;
    for (Counter = 0; Counter < RXC1_COUNTER_COUNT; Counter++)
      if (!Read64(0x20U + Counter * 8U, &Candidate.Counters[Counter]))
        break;
    if (Counter != RXC1_COUNTER_COUNT || !Read(0x48U, &FrozenOverflow)) break;
    if (FrozenOverflow > 1U || ((Status & RXC1_OVERFLOW) != 0) != (FrozenOverflow != 0))
    { Outcome = eRXC1OverflowMismatch; break; }
    Candidate.Overflow = FrozenOverflow != 0;
    if (!Read(0x08U, &EndStatus) || !Read(0x10U, &Word)) break;
    if (EndStatus & RXC1_RESET) { Outcome = eRXC1ResetChanged; break; }
    if (EndStatus != Status || Word != Serial)
    { Outcome = eRXC1SnapshotChanged; break; }
    if (!Read(0x0cU, &Word) || Word != Session ||
        !Read(0x14U, &Word) || Word != FrozenSession ||
        !Read(0x18U, &Word) || Word != ConfigurationGeneration ||
        !Read(0x1cU, &Word) || Word != Metadata ||
        !Read(0x50U, &Word) || Word != FrozenToken ||
        !Read(0x4cU, &Word) || Word != g_token ||
        !Read64(0x54U, &CheckConfiguration) || CheckConfiguration != Configuration ||
        !Read(0x48U, &Word) || Word != FrozenOverflow)
    { Outcome = eRXC1SnapshotChanged; break; }
    Outcome = eRXC1Valid;
  } while (0);

  if (!Write(0x04U, RXC1_ACK) || !Read(0x08U, &Status))
    return eRXC1AckFailed;
  if (Status & RXC1_RESET) return eRXC1ResetChanged;
  if (Status & RXC1_VALID) return eRXC1AckFailed;
  if (!(Status & RXC1_TOKEN_BOUND)) return eRXC1TokenMismatch;
  if (Outcome == eRXC1Valid)
  {
    if (!Read(0x0cU, &Word) || !Read(0x10U, &Serial) ||
        !Read(0x4cU, &Token)) return eRXC1ReadError;
    if (Word != Candidate.SessionGeneration || Serial != Candidate.SnapshotSerial ||
        Token != Candidate.HostToken) return eRXC1ResetChanged;
  }
  if (Outcome == eRXC1Valid)
    *Result = Candidate;
  return Outcome;
}

void RXC1Sample(void)
{
  unsigned int Receiver;
  ERXC1Status Status;
  bool AnyValid = false;
  if (!g_poll_enabled) return;
  if (!g_bound)
  {
    Status = Bind();
    if (Status != eRXC1Valid)
    {
      for (Receiver = 0; Receiver < RXC1_DDC_COUNT; Receiver++)
        Invalidate(&g_snapshot.DDC[Receiver], Receiver, Status);
      g_snapshot.SampledAtMs = 0;
      if (Status != eRXC1Unsupported)
        g_snapshot.HostAcquisitionFailures++;
      return;
    }
  }
  for (Receiver = 0; Receiver < RXC1_DDC_COUNT; Receiver++)
  {
    Status = Acquire(Receiver, &g_snapshot.DDC[Receiver]);
    if (Status == eRXC1Valid)
    {
      AnyValid = true;
      continue;
    }
    Invalidate(&g_snapshot.DDC[Receiver], Receiver, Status);
    g_snapshot.HostAcquisitionFailures++;
    if (Status == eRXC1ResetChanged || Status == eRXC1ResetActive ||
        Status == eRXC1TokenMismatch || Status == eRXC1GenerationExhausted ||
        Status == eRXC1AckFailed || Status == eRXC1WriteError ||
        Status == eRXC1StaleSnapshot || Status == eRXC1SnapshotUnavailable)
    {
      g_bound = false;
      for (Receiver = 0; Receiver < RXC1_DDC_COUNT; Receiver++)
        Invalidate(&g_snapshot.DDC[Receiver], Receiver, Status);
      AnyValid = false;
      /* A failed request write may have been accepted by the FPGA.  Never
       * publish this poll; make one bounded checked recovery attempt now.
       * Bind inspects status, ACKs pending data, verifies clear, then binds a
       * fresh token.  A failed recovery is retried on the next owner poll. */
      if (Status == eRXC1WriteError || Status == eRXC1StaleSnapshot ||
          Status == eRXC1SnapshotUnavailable)
        (void)Bind();
      break;
    }
  }
  g_snapshot.SampledAtMs = AnyValid ? NowMs() : 0;
}

void RXC1MaybeSample(void)
{
  time_t Now = time(NULL);
  if (!g_poll_enabled) return;
  if (Now == (time_t)-1 || (g_last_sample_second != 0 && Now - g_last_sample_second < 5))
    return;
  g_last_sample_second = Now;
  RXC1Sample();
}

void RXC1GetSnapshot(TRXC1Snapshot *Snapshot)
{
  if (Snapshot != NULL)
    *Snapshot = g_snapshot;
}
