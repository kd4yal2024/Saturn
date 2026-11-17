#include "protocol2_control.h"

#include <stddef.h>

#define P2_STATUS_RX_PERIOD_NS 50000000ULL
#define P2_STATUS_TX_PERIOD_NS 1000000ULL
#define P2_STATUS_INPUT_POLL_US 500U

void P2StatusScheduleReset(TP2StatusSchedule *Schedule)
{
    if(Schedule != NULL)
        *Schedule = (TP2StatusSchedule){0};
}

static uint64_t StatusDeadline(uint64_t Origin, uint64_t Period)
{
    return Origin > UINT64_MAX - Period ? UINT64_MAX : Origin + Period;
}

void P2StatusScheduleSent(TP2StatusSchedule *Schedule, uint64_t NowNs, bool Transmit)
{
    const uint64_t Period = Transmit ? P2_STATUS_TX_PERIOD_NS : P2_STATUS_RX_PERIOD_NS;

    if(Schedule == NULL)
        return;
    if(!Schedule->Valid || Schedule->Transmit != Transmit)
        Schedule->NextPeriodicNs = StatusDeadline(NowNs, Period);
    else if(NowNs >= Schedule->NextPeriodicNs)
    {
        /* Coalesce missed status periods; do not burst old snapshots or add
         * the time spent gathering telemetry to every future deadline. */
        const uint64_t Phase = (NowNs - Schedule->NextPeriodicNs) % Period;
        Schedule->NextPeriodicNs = StatusDeadline(NowNs - Phase, Period);
    }
    /* An early PTT/ADC event does not postpone the periodic deadline. */
    Schedule->Valid = true;
    Schedule->Transmit = Transmit;
}

uint32_t P2StatusScheduleWaitUs(const TP2StatusSchedule *Schedule, uint64_t NowNs,
                              bool Transmit)
{
    uint64_t RemainingNs;

    if(Schedule == NULL || !Schedule->Valid || Schedule->Transmit != Transmit ||
       NowNs >= Schedule->NextPeriodicNs)
        return 0U;
    RemainingNs = Schedule->NextPeriodicNs - NowNs;
    if(RemainingNs >= (uint64_t)P2_STATUS_INPUT_POLL_US * 1000U)
        return P2_STATUS_INPUT_POLL_US;
    return (uint32_t)((RemainingNs + 999U) / 1000U);
}

bool P2FirmwareProtocolCompatible(unsigned int Major, unsigned int Minor)
{
    // Minor 31 keeps the major-1 Protocol 2 and TX register interface.
    // Do not add a minor-version TX hold to this compatibility check.
    (void)Minor;
    return Major == P2_FIRMWARE_REQUIRED_MAJOR;
}

TP2RunState P2DecodeRunState(uint8_t Flags)
{
    TP2RunState State;

    State.Run = (Flags & 0x01U) != 0U;
    State.Transmit = State.Run && ((Flags & 0x02U) != 0U);
    return State;
}

void P2SequenceReset(TP2SequenceTracker *Tracker)
{
    if(Tracker == NULL)
        return;

    Tracker->Valid = false;
    Tracker->LastAccepted = 0U;
}

bool P2SequenceAccept(TP2SequenceTracker *Tracker, uint32_t Sequence, uint32_t *MissingPackets)
{
    uint32_t Delta;

    if(MissingPackets != NULL)
        *MissingPackets = 0U;
    if(Tracker == NULL)
        return false;

    if(!Tracker->Valid)
    {
        Tracker->Valid = true;
        Tracker->LastAccepted = Sequence;
        return true;
    }

    Delta = Sequence - Tracker->LastAccepted;
    if((Delta == 0U) || (Delta > 0x7fffffffU))
        return false;

    Tracker->LastAccepted = Sequence;
    if(MissingPackets != NULL)
        *MissingPackets = Delta - 1U;
    return true;
}

bool P2ControlSequenceAccept(TP2SequenceTracker *Tracker, uint32_t Sequence, uint32_t *MissingPackets)
{
    uint32_t Delta;

    if(MissingPackets != NULL)
        *MissingPackets = 0U;
    if(Tracker == NULL)
        return false;

    if(!Tracker->Valid)
    {
        Tracker->Valid = true;
        Tracker->LastAccepted = Sequence;
        return true;
    }

    // Control packets are idempotent state snapshots, and Thetis transmits
    // every high-priority and DDC/DUC-specific control packet with sequence
    // zero. A repeated sequence number is therefore current state that must
    // be applied, not a replayed datagram. Only a strictly backward sequence
    // (a genuinely stale packet from a client that does increment) is dropped.
    Delta = Sequence - Tracker->LastAccepted;
    if(Delta == 0U)
        return true;
    if(Delta > 0x7fffffffU)
        return false;

    Tracker->LastAccepted = Sequence;
    if(MissingPackets != NULL)
        *MissingPackets = Delta - 1U;
    return true;
}

uint16_t P2ScaleFifoSamples(uint32_t Locations, uint32_t SamplesPerGroup, uint32_t LocationsPerGroup)
{
    uint64_t Samples;

    if(LocationsPerGroup == 0U)
        return UINT16_MAX;

    Samples = ((uint64_t)Locations * SamplesPerGroup) / LocationsPerGroup;
    return (Samples > UINT16_MAX) ? UINT16_MAX : (uint16_t)Samples;
}
