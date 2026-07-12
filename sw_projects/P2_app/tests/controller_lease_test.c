#include "../controller_lease.h"

#include <assert.h>
#include <stdio.h>
#include <string.h>
#include <arpa/inet.h>

static struct sockaddr_in source(const char *address, unsigned short port)
{
    struct sockaddr_in value;
    memset(&value, 0, sizeof(value));
    value.sin_family = AF_INET;
    value.sin_port = htons(port);
    assert(inet_pton(AF_INET, address, &value.sin_addr) == 1);
    return value;
}

int main(void)
{
    struct sockaddr_in first = source("192.0.2.10", 1024);
    struct sockaddr_in same_host = source("192.0.2.10", 22000);
    struct sockaddr_in other = source("192.0.2.11", 1024);

    ControllerLeaseClear();
    assert(!ControllerLeaseIsOwned());
    assert(!ControllerLeaseMatches(&first));
    assert(ControllerLeaseClaim(&first));
    assert(ControllerLeaseIsOwned());
    assert(ControllerLeaseMatches(&same_host));
    assert(!ControllerLeaseClaim(&other));
    assert(!ControllerLeaseMatches(&other));
    assert(!ControllerLeaseRelease(&other));
    assert(ControllerLeaseRelease(&same_host));
    assert(!ControllerLeaseIsOwned());
    assert(ControllerLeaseClaim(&other));
    ControllerLeaseClear();
    assert(!ControllerLeaseIsOwned());

    puts("controller lease tests passed");
    return 0;
}
