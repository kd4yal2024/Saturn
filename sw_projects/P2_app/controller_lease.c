#include "controller_lease.h"

#include <pthread.h>
#include <string.h>

static pthread_mutex_t LeaseMutex = PTHREAD_MUTEX_INITIALIZER;
static bool LeaseOwned = false;
static struct in_addr LeaseAddress;

static bool valid_source(const struct sockaddr_in *source)
{
    return source != NULL && source->sin_family == AF_INET &&
           source->sin_addr.s_addr != htonl(INADDR_ANY);
}

bool ControllerLeaseClaim(const struct sockaddr_in *source)
{
    bool accepted = false;
    if(!valid_source(source))
        return false;

    pthread_mutex_lock(&LeaseMutex);
    if(!LeaseOwned)
    {
        LeaseAddress = source->sin_addr;
        LeaseOwned = true;
        accepted = true;
    }
    else
        accepted = LeaseAddress.s_addr == source->sin_addr.s_addr;
    pthread_mutex_unlock(&LeaseMutex);
    return accepted;
}

bool ControllerLeaseMatches(const struct sockaddr_in *source)
{
    bool accepted;
    if(!valid_source(source))
        return false;

    pthread_mutex_lock(&LeaseMutex);
    accepted = LeaseOwned && LeaseAddress.s_addr == source->sin_addr.s_addr;
    pthread_mutex_unlock(&LeaseMutex);
    return accepted;
}

bool ControllerLeaseRelease(const struct sockaddr_in *source)
{
    bool released = false;
    if(!valid_source(source))
        return false;

    pthread_mutex_lock(&LeaseMutex);
    if(LeaseOwned && LeaseAddress.s_addr == source->sin_addr.s_addr)
    {
        memset(&LeaseAddress, 0, sizeof(LeaseAddress));
        LeaseOwned = false;
        released = true;
    }
    pthread_mutex_unlock(&LeaseMutex);
    return released;
}

void ControllerLeaseClear(void)
{
    pthread_mutex_lock(&LeaseMutex);
    memset(&LeaseAddress, 0, sizeof(LeaseAddress));
    LeaseOwned = false;
    pthread_mutex_unlock(&LeaseMutex);
}

bool ControllerLeaseIsOwned(void)
{
    bool owned;
    pthread_mutex_lock(&LeaseMutex);
    owned = LeaseOwned;
    pthread_mutex_unlock(&LeaseMutex);
    return owned;
}
