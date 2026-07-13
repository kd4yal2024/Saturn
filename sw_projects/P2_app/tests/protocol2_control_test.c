#include "protocol2_control.h"

#include <assert.h>
#include <stdint.h>
#include <stdio.h>

static void test_run_state_requires_run_for_transmit(void)
{
    TP2RunState State;

    State = P2DecodeRunState(0x00U);
    assert(!State.Run && !State.Transmit);

    State = P2DecodeRunState(0x01U);
    assert(State.Run && !State.Transmit);

    State = P2DecodeRunState(0x02U);
    assert(!State.Run && !State.Transmit);

    State = P2DecodeRunState(0x03U);
    assert(State.Run && State.Transmit);
}

static void test_sequence_acceptance_and_gaps(void)
{
    TP2SequenceTracker Tracker = {0};
    uint32_t Missing = 99U;

    assert(P2SequenceAccept(&Tracker, 10U, &Missing));
    assert(Missing == 0U);
    assert(P2SequenceAccept(&Tracker, 11U, &Missing));
    assert(Missing == 0U);
    assert(P2SequenceAccept(&Tracker, 15U, &Missing));
    assert(Missing == 3U);

    assert(!P2SequenceAccept(&Tracker, 15U, &Missing));
    assert(Missing == 0U);
    assert(!P2SequenceAccept(&Tracker, 14U, &Missing));
    assert(Missing == 0U);
    assert(P2SequenceAccept(&Tracker, 16U, &Missing));
}

static void test_sequence_wrap_and_reset(void)
{
    TP2SequenceTracker Tracker = {0};
    uint32_t Missing = 0U;

    assert(P2SequenceAccept(&Tracker, UINT32_MAX - 1U, &Missing));
    assert(P2SequenceAccept(&Tracker, UINT32_MAX, &Missing));
    assert(P2SequenceAccept(&Tracker, 0U, &Missing));
    assert(P2SequenceAccept(&Tracker, 2U, &Missing));
    assert(Missing == 1U);

    P2SequenceReset(&Tracker);
    assert(P2SequenceAccept(&Tracker, 0U, &Missing));
    assert(Missing == 0U);
}

static void test_fifo_sample_scaling(void)
{
    assert(P2ScaleFifoSamples(16U, 4U, 1U) == 64U);
    assert(P2ScaleFifoSamples(180U, 4U, 3U) == 240U);
    assert(P2ScaleFifoSamples(32U, 2U, 1U) == 64U);
    assert(P2ScaleFifoSamples(UINT32_MAX, 4U, 1U) == UINT16_MAX);
    assert(P2ScaleFifoSamples(1U, 1U, 0U) == UINT16_MAX);
}

int main(void)
{
    test_run_state_requires_run_for_transmit();
    test_sequence_acceptance_and_gaps();
    test_sequence_wrap_and_reset();
    test_fifo_sample_scaling();
    printf("protocol2 control tests passed\n");
    return 0;
}
