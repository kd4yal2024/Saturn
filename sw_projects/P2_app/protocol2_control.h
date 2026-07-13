#ifndef PROTOCOL2_CONTROL_H
#define PROTOCOL2_CONTROL_H

#include <stdbool.h>
#include <stdint.h>

typedef struct
{
    bool Run;
    bool Transmit;
} TP2RunState;

typedef struct
{
    bool Valid;
    uint32_t LastAccepted;
} TP2SequenceTracker;

TP2RunState P2DecodeRunState(uint8_t Flags);
void P2SequenceReset(TP2SequenceTracker *Tracker);
bool P2SequenceAccept(TP2SequenceTracker *Tracker, uint32_t Sequence, uint32_t *MissingPackets);
uint16_t P2ScaleFifoSamples(uint32_t Locations, uint32_t SamplesPerGroup, uint32_t LocationsPerGroup);

#endif
