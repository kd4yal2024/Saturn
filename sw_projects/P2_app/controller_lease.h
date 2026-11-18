#ifndef SATURN_CONTROLLER_LEASE_H
#define SATURN_CONTROLLER_LEASE_H

#include <stdbool.h>
#include <stdint.h>
#include <netinet/in.h>

bool ControllerLeaseClaim(const struct sockaddr_in *source);
bool ControllerLeaseMatches(const struct sockaddr_in *source);
/* Match and stream identity are sampled together under the lease lock. A
 * released/reclaimed lease changes identity even when RUN=0 is not polled. */
bool ControllerLeaseMatchesEpoch(const struct sockaddr_in *source, uint64_t *epoch);
bool ControllerLeaseRelease(const struct sockaddr_in *source);
void ControllerLeaseClear(void);
bool ControllerLeaseIsOwned(void);

#endif
