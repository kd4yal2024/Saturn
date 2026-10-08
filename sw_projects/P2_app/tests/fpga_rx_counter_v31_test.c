#define _POSIX_C_SOURCE 200809L
#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "fpga_rx_counter_v31.h"

/* Independent snapshot-boundary oracle, never derived from register readback. */
static const uint64_t Expected[RXC1_DDC_COUNT][RXC1_COUNTER_COUNT] = {
  {100, 3, 7, 1, 2}, {200, 9, 11, 2, 3}, {300, 15, 17, 3, 4},
  {400, 21, 23, 4, 5}, {500, 27, 29, 5, 6}, {600, 33, 35, 6, 7},
  {700, 39, 41, 7, 8}, {800, 45, 47, 8, 9}, {900, 51, 53, 9, 10},
  {1000, 57, 59, 10, 11}
};

typedef enum { Normal, BadCounter, BadReceiver, BadConfig, BadToken,
               ReadFailure, StaleSerial, ResetDuringRead, ResetOnAck, Unsupported,
               Saturated, RequestAcceptedWriteError, ResetCompletedOnAck,
               SessionChangedOnAck, SerialChangedOnAck, TokenChangedOnAck } EMode;
static EMode Mode;
static uint32_t Token, Serial, Session, ConfigReads;
static unsigned int Selected;
static unsigned int CorruptReceiver = 3U, CorruptCounter = 1U;
static bool Valid, Reset;
static bool FaultFired;
static uint32_t FailedRequestToken;
static unsigned int AckFailuresRemaining, AckAttempts;
static unsigned int RegisterReads, RegisterWrites;

bool RegisterReadChecked(uint32_t Address, uint32_t *Value)
{
  RegisterReads++;
  uint32_t Offset = Address - 0x8000U;
  uint64_t Counter;
  unsigned int Index;
  if (Address < 0x8000U || Value == NULL) return false;
  if (Mode == ResetDuringRead && Valid && Selected == 6U && Offset == 0x30U)
  {
    Valid = false;
    Reset = true;
    Session++;
    Serial = 0;
  }
  if (Mode == ReadFailure && Valid && Offset == 0x30U) return false;
  if ((Offset >= 0x14U && Offset <= 0x48U) ||
      Offset == 0x50U || Offset == 0x54U || Offset == 0x58U)
    if (!Valid || Reset) return false;
  switch (Offset)
  {
    case 0x00U: *Value = Mode == Unsupported ? 0 : 0x52584331U; break;
    case 0x08U: *Value = (Valid ? 1U : 0U) | (Reset ? 8U : 0U) |
                         (Valid && Mode == Saturated ? 4U : 0U) | (Token ? 0x40U : 0U); break;
    case 0x0cU: case 0x14U: *Value = Session; break;
    case 0x10U: *Value = Serial; break;
    case 0x18U:
      ConfigReads++;
      *Value = Mode == BadConfig && ConfigReads >= 2U ? 2U : 1U;
      break;
    case 0x1cU: *Value = 0x40U | (Mode == BadReceiver ? (Selected + 1U) % RXC1_DDC_COUNT : Selected); break;
    case 0x48U: *Value = Mode == Saturated ? 1U : 0U; break;
    case 0x58U: *Value = 0; break;
    case 0x4cU: *Value = Token; break;
    case 0x50U: *Value = Mode == BadToken ? Token ^ 1U : Token; break;
    case 0x54U: *Value = Selected + 1U; break;
    default:
      if (Offset < 0x20U || Offset > 0x44U || (Offset & 3U)) return false;
      Index = (Offset - 0x20U) / 8U;
      Counter = Expected[Selected][Index];
      if (Mode == Saturated && Selected == 0U && Index == 1U) Counter = UINT64_MAX;
      if (Mode == BadCounter && Selected == CorruptReceiver && Index == CorruptCounter) Counter++;
      *Value = (Offset & 4U) ? (uint32_t)(Counter >> 32) : (uint32_t)Counter;
      break;
  }
  return true;
}

bool RegisterWriteChecked(uint32_t Address, uint32_t Value)
{
  RegisterWrites++;
  if (Reset) return false;
  if (Address == 0x804cU && !Valid && Value != 0)
  { Token = Value; return true; }
  if (Address != 0x8004U) return false;
  if (Value == 0x40000000U && Valid)
  {
    AckAttempts++;
    if (AckFailuresRemaining != 0U)
    { AckFailuresRemaining--; return false; }
    Valid = false;
    if (Mode == ResetOnAck && Selected == 6U)
    { Reset = true; Session++; Serial = 0; }
    if (Selected == RXC1_DDC_COUNT - 1U)
    {
      if (Mode == ResetCompletedOnAck) { Session++; Serial = 0; Reset = false; }
      if (Mode == SessionChangedOnAck) Session++;
      if (Mode == SerialChangedOnAck) Serial++;
      if (Mode == TokenChangedOnAck) Token ^= 1U;
    }
    return true;
  }
  if ((Value & 0x80000000U) && !Valid && Token)
  {
    Selected = Value & 0x0fU;
    if (Selected >= RXC1_DDC_COUNT) return false;
    ConfigReads = 0;
    Valid = true;
    if (Mode != StaleSerial) Serial++;
    if (Mode == RequestAcceptedWriteError && !FaultFired)
    { FaultFired = true; FailedRequestToken = Token; return false; }
    return true;
  }
  return false;
}

static void ResetFake(EMode NewMode)
{
  Mode = NewMode;
  Token = Serial = Session = ConfigReads = 0;
  Selected = 0;
  CorruptReceiver = 3U;
  CorruptCounter = 1U;
  Valid = Reset = FaultFired = false;
  FailedRequestToken = 0;
  AckFailuresRemaining = AckAttempts = 0;
  RegisterReads = RegisterWrites = 0;
  RXC1Init();
}

static void TestPollingDisabled(void)
{
  TRXC1Snapshot Snapshot;
  FILE *File;
  char Buffer[8192];
  size_t Length;
  unsigned int Receiver;
  assert(setenv("SATURN_RXC1_POLL_ENABLED", "0", 1) == 0);
  ResetFake(Normal);
  RXC1Sample();
  RXC1MaybeSample();
  RXC1GetSnapshot(&Snapshot);
  assert(RegisterReads == 0 && RegisterWrites == 0);
  assert(Snapshot.SampledAtMs == 0 && Snapshot.HostAcquisitionFailures == 0);
  for (Receiver = 0; Receiver < RXC1_DDC_COUNT; Receiver++)
    assert(Snapshot.DDC[Receiver].Status == eRXC1Disabled);
  File = tmpfile();
  assert(File != NULL);
  RXC1WriteJSON(File, &Snapshot);
  rewind(File);
  Length = fread(Buffer, 1, sizeof(Buffer) - 1U, File);
  Buffer[Length] = 0;
  fclose(File);
  assert(strstr(Buffer, "\"status\":\"disabled\"") != NULL);
  assert(strstr(Buffer, "\"refused_pre_fir_pair_candidates\":null") != NULL);
  assert(unsetenv("SATURN_RXC1_POLL_ENABLED") == 0);
  ResetFake(Normal);
  RXC1Sample();
  assert(RegisterReads != 0 && RegisterWrites != 0);
}

static void TestNormalAndRestart(void)
{
  TRXC1Snapshot Snapshot;
  uint32_t FirstToken;
  unsigned int Receiver, Counter;
  ResetFake(Normal);
  RXC1Sample();
  RXC1GetSnapshot(&Snapshot);
  for (Receiver = 0; Receiver < RXC1_DDC_COUNT; Receiver++)
  {
    assert(Snapshot.DDC[Receiver].Status == eRXC1Valid);
    assert(Snapshot.DDC[Receiver].Receiver == Receiver);
    assert(Snapshot.DDC[Receiver].SnapshotSerial == Receiver + 1U);
    assert(Snapshot.DDC[Receiver].ConfigurationGeneration == 1U);
    assert(Snapshot.DDC[Receiver].RateCode == 4U);
    for (Counter = 0; Counter < RXC1_COUNTER_COUNT; Counter++)
      assert(Snapshot.DDC[Receiver].Counters[Counter] == Expected[Receiver][Counter]);
  }
  FirstToken = Snapshot.DDC[0].HostToken;
  assert(FirstToken != 0);
  assert(RegisterWriteChecked(0x8004U, 0x80000000U)); /* old pending snapshot */
  RXC1Init(); /* new acquisition owner, same hardware */
  RXC1Sample();
  RXC1GetSnapshot(&Snapshot);
  assert(Snapshot.DDC[0].Status == eRXC1Valid);
  assert(Snapshot.DDC[0].HostToken != FirstToken);
  assert(!Valid);
}

static void TestCorruptions(void)
{
  static const EMode Cases[] = { BadCounter, BadReceiver, BadConfig, BadToken,
                                 ReadFailure, StaleSerial, ResetDuringRead, ResetOnAck };
  TRXC1Snapshot Snapshot;
  unsigned int Case, Receiver, Counter;
  bool SoftwareRejected, OracleRejected;
  for (Case = 0; Case < sizeof(Cases) / sizeof(Cases[0]); Case++)
  {
    ResetFake(Cases[Case]);
    RXC1Sample();
    RXC1GetSnapshot(&Snapshot);
    SoftwareRejected = OracleRejected = false;
    for (Receiver = 0; Receiver < RXC1_DDC_COUNT; Receiver++)
    {
      if (Snapshot.DDC[Receiver].Status != eRXC1Valid)
      { SoftwareRejected = true; continue; }
      for (Counter = 0; Counter < RXC1_COUNTER_COUNT; Counter++)
        if (Snapshot.DDC[Receiver].Counters[Counter] != Expected[Receiver][Counter])
          OracleRejected = true; /* independent boundary oracle */
    }
    if (Cases[Case] == BadCounter)
    {
      assert(!SoftwareRejected);
      assert(OracleRejected);
    }
    else
      assert(SoftwareRejected);
    if (Cases[Case] == ResetDuringRead || Cases[Case] == ResetOnAck)
      for (Receiver = 0; Receiver < RXC1_DDC_COUNT; Receiver++)
        assert(Snapshot.DDC[Receiver].Status != eRXC1Valid);
  }
}

static void TestEveryCounterOracle(void)
{
  TRXC1Snapshot Snapshot;
  unsigned int Receiver, Counter, DDC, Field, Discrepancies;
  for (Receiver = 0; Receiver < RXC1_DDC_COUNT; Receiver++)
    for (Counter = 0; Counter < RXC1_COUNTER_COUNT; Counter++)
    {
      ResetFake(BadCounter);
      CorruptReceiver = Receiver;
      CorruptCounter = Counter;
      RXC1Sample();
      RXC1GetSnapshot(&Snapshot);
      Discrepancies = 0;
      for (DDC = 0; DDC < RXC1_DDC_COUNT; DDC++)
      {
        assert(Snapshot.DDC[DDC].Status == eRXC1Valid);
        for (Field = 0; Field < RXC1_COUNTER_COUNT; Field++)
          if (Snapshot.DDC[DDC].Counters[Field] != Expected[DDC][Field])
          {
            assert(DDC == Receiver && Field == Counter);
            Discrepancies++;
          }
      }
      assert(Discrepancies == 1U);
    }
}

static void AssertAllInvalid(const TRXC1Snapshot *Snapshot)
{
  unsigned int Receiver;
  for (Receiver = 0; Receiver < RXC1_DDC_COUNT; Receiver++)
    assert(Snapshot->DDC[Receiver].Status != eRXC1Valid);
  assert(Snapshot->SampledAtMs == 0);
}

static void TestUncertainRequestRecovery(void)
{
  TRXC1Snapshot Snapshot;
  uint32_t ReboundToken;
  ResetFake(RequestAcceptedWriteError);
  RXC1Sample();
  RXC1GetSnapshot(&Snapshot);
  AssertAllInvalid(&Snapshot);
  assert(Snapshot.DDC[0].Status == eRXC1WriteError);
  assert(FaultFired);
  assert(!Valid); /* checked recovery ACKed the accepted request */
  assert(Token != FailedRequestToken); /* recovery bound a fresh token */
  ReboundToken = Token;
  RXC1Sample();
  RXC1GetSnapshot(&Snapshot);
  assert(Snapshot.DDC[0].Status == eRXC1Valid);
  assert(Snapshot.DDC[0].HostToken == ReboundToken);
  assert(!Valid);

  ResetFake(Normal);
  RXC1Sample();
  assert(RegisterWriteChecked(0x8004U, 0x80000000U));
  assert(Valid);
  ReboundToken = Token;
  RXC1Sample();
  RXC1GetSnapshot(&Snapshot);
  AssertAllInvalid(&Snapshot);
  assert(Snapshot.DDC[0].Status == eRXC1StaleSnapshot);
  assert(!Valid);
  assert(Token != ReboundToken);
  RXC1Sample();
  RXC1GetSnapshot(&Snapshot);
  assert(Snapshot.DDC[0].Status == eRXC1Valid);

  ResetFake(RequestAcceptedWriteError);
  AckFailuresRemaining = 1U;
  RXC1Sample();
  RXC1GetSnapshot(&Snapshot);
  AssertAllInvalid(&Snapshot);
  assert(Valid); /* the one bounded recovery attempt failed */
  assert(AckAttempts == 1U);
  RXC1Sample();
  RXC1GetSnapshot(&Snapshot);
  assert(Snapshot.DDC[0].Status == eRXC1Valid);
  assert(!Valid);
  assert(Token != FailedRequestToken);
}

static void TestPostAckIdentityChanges(void)
{
  static const EMode Cases[] = { ResetCompletedOnAck, SessionChangedOnAck,
                                 SerialChangedOnAck,
                                 TokenChangedOnAck };
  TRXC1Snapshot Snapshot;
  unsigned int Case;
  for (Case = 0; Case < sizeof(Cases) / sizeof(Cases[0]); Case++)
  {
    ResetFake(Cases[Case]);
    RXC1Sample();
    RXC1GetSnapshot(&Snapshot);
    /* The fault occurs on the last ACK. A check at the next DDC cannot
     * mask removal of the required post-ACK session/serial/token reads. */
    AssertAllInvalid(&Snapshot);
    assert(!Reset);
    assert(!Valid);
  }
}

static void TestUnavailableJSON(void)
{
  TRXC1Snapshot Snapshot;
  FILE *File;
  char Buffer[8192];
  size_t Length;
  ResetFake(Unsupported);
  RXC1Sample();
  RXC1GetSnapshot(&Snapshot);
  assert(Snapshot.DDC[0].Status == eRXC1Unsupported);
  File = tmpfile();
  assert(File != NULL);
  RXC1WriteJSON(File, &Snapshot);
  rewind(File);
  Length = fread(Buffer, 1, sizeof(Buffer) - 1U, File);
  Buffer[Length] = 0;
  fclose(File);
  assert(strstr(Buffer, "\"refused_pre_fir_pair_candidates\":null") != NULL);
  assert(strstr(Buffer, "\"status\":\"unsupported\"") != NULL);
}

static void TestSaturation(void)
{
  TRXC1Snapshot Snapshot;
  FILE *File;
  char Buffer[8192];
  size_t Length;
  ResetFake(Saturated);
  RXC1Sample();
  RXC1GetSnapshot(&Snapshot);
  assert(Snapshot.DDC[0].Status == eRXC1Valid);
  assert(Snapshot.DDC[0].Counters[1] == UINT64_MAX);
  assert(Snapshot.DDC[0].Overflow);
  File = tmpfile();
  assert(File != NULL);
  RXC1WriteJSON(File, &Snapshot);
  rewind(File);
  Length = fread(Buffer, 1, sizeof(Buffer) - 1U, File);
  Buffer[Length] = 0;
  fclose(File);
  assert(strstr(Buffer, "\"refused_pre_fir_pair_candidates\":\"18446744073709551615\"") != NULL);
  assert(strstr(Buffer, "\"exact\":false") != NULL);
}

int main(void)
{
  TestNormalAndRestart();
  TestCorruptions();
  TestEveryCounterOracle();
  TestUncertainRequestRecovery();
  TestPostAckIdentityChanges();
  TestUnavailableJSON();
  TestSaturation();
  TestPollingDisabled();
  puts("RXC1 checked acquisition tests passed");
  return 0;
}
