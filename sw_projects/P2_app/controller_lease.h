#ifndef SATURN_CONTROLLER_LEASE_H
#define SATURN_CONTROLLER_LEASE_H

#include <stdbool.h>
#include <netinet/in.h>

bool ControllerLeaseClaim(const struct sockaddr_in *source);
bool ControllerLeaseMatches(const struct sockaddr_in *source);
bool ControllerLeaseRelease(const struct sockaddr_in *source);
void ControllerLeaseClear(void);
bool ControllerLeaseIsOwned(void);

#endif
