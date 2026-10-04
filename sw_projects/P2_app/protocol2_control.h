#ifndef PROTOCOL2_CONTROL_H
#define PROTOCOL2_CONTROL_H

#include <stdbool.h>
#include <stdint.h>

#define P2_FIRMWARE_REQUIRED_MAJOR 1U

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
bool P2FirmwareProtocolCompatible(unsigned int Major, unsigned int Minor);
void P2SequenceReset(TP2SequenceTracker *Tracker);
bool P2SequenceAccept(TP2SequenceTracker *Tracker, uint32_t Sequence, uint32_t *MissingPackets);
bool P2ControlSequenceAccept(TP2SequenceTracker *Tracker, uint32_t Sequence, uint32_t *MissingPackets);
uint16_t P2ScaleFifoSamples(uint32_t Locations, uint32_t SamplesPerGroup, uint32_t LocationsPerGroup);

#endif
