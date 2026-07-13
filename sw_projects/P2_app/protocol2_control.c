#include "protocol2_control.h"

#include <stddef.h>

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

uint16_t P2ScaleFifoSamples(uint32_t Locations, uint32_t SamplesPerGroup, uint32_t LocationsPerGroup)
{
    uint64_t Samples;

    if(LocationsPerGroup == 0U)
        return UINT16_MAX;

    Samples = ((uint64_t)Locations * SamplesPerGroup) / LocationsPerGroup;
    return (Samples > UINT16_MAX) ? UINT16_MAX : (uint16_t)Samples;
}
