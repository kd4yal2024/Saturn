/////////////////////////////////////////////////////////////
//
// Saturn project: Artix7 FPGA + Raspberry Pi4 Compute Module
// PCI Express interface from linux on Raspberry pi
// this application uses C code to emulate HPSDR protocol 2
//
// copyright Laurence Barker November 2021
// licenced under GNU GPL3
// derived from Pavel Demin code 
//
// threaddata.h:
//
// header defining thread control data
//
//////////////////////////////////////////////////////////////

#ifndef __threaddata_h
#define __threaddata_h


#include <stdint.h>
#include <netinet/in.h>
#include <stdatomic.h>
#include "../common/saturntypes.h"
#include <semaphore.h>



//
// list of port numbers, provided in the general packet
// (port 1024 for discovery and general packets not needed in this list)
#define VPORTTABLESIZE 20
// incoming port numbers
#define VPORTCOMMAND 0
#define VPORTDDCSPECIFIC 1
#define VPORTDUCSPECIFIC 2
#define VPORTHIGHPRIORITYTOSDR 3
#define VPORTSPKRAUDIO 4
#define VPORTDUCIQ 5
// outgoing port numbers:
#define VPORTHIGHPRIORITYFROMSDR 6
#define VPORTMICAUDIO 7
#define VPORTDDCIQ0 8
#define VPORTDDCIQ1 9
#define VPORTDDCIQ2 10
#define VPORTDDCIQ3 11
#define VPORTDDCIQ4 12
#define VPORTDDCIQ5 13
#define VPORTDDCIQ6 14
#define VPORTDDCIQ7 15
#define VPORTDDCIQ8 16
#define VPORTDDCIQ9 17
#define VPORTWIDEBAND0 18
#define VPORTWIDEBAND1 19


//
// a type to hold data for each incoming or outgoing data thread
//
struct ThreadSocketData
{
  uint32_t DDCid;                               // only relevant to DDC threads
  atomic_int Socketid;                          // socket to access internet
  atomic_ushort Portid;                         // port to access
  char *Nameid;                                 // name (for error msg etc)
  atomic_bool Active;                           // true if thread is active
  struct sockaddr_in addr_cmddata;
  atomic_uint_fast32_t Cmdid;                   // command from app to thread - bits set for each command
  uint32_t DDCSampleRate;                       // DDC sample rate
};


extern struct ThreadSocketData SocketData[];        // data for each thread
extern struct sockaddr_in reply_addr;               // destination address for outgoing data
extern pthread_mutex_t g_reply_addr_mutex;          // protect reply_addr updates and snapshots
extern atomic_bool IsTXMode;                        // true if in TX
extern atomic_bool SDRActive;                       // true if this SDR is running at the moment
extern atomic_bool ReplyAddressSet;                 // true when reply address has been set
extern atomic_bool StartBitReceived;                // true when "run" bit has been set
extern atomic_bool NewMessageReceived;              // set whenever a message is received
extern atomic_bool ThreadError;                     // set true if a thread reports an error
extern atomic_bool ExitRequested;                   // true when app-wide shutdown has been requested
extern bool UseDebug;                               // true if debugging enabled
extern uint8_t GlobalFIFOOverflows;                 // FIFO overflow words
extern pthread_mutex_t g_fifo_overflow_mutex;       // protect GlobalFIFOOverflows from race conditions
extern sem_t MicWBDMAMutex;                         // protect one DMA read channel shared by mic and WB read
extern atomic_int DMAReadfile_fd;                   // shared DMA read FD used by mic and wideband paths


#define VBITCHANGEPORT 1                        // if set, thread must close its socket and open a new one on different port
#define VBITINTERLEAVE 2                        // if set, DDC threads should interleave data
#define VBITDDCENABLE 4                         // if set, DDC is enabled

//
// default port numbers, used if incoming port number = 0
//
extern uint16_t DefaultPorts[];




//
// set the port for a given thread. If 0, set the default according to HPSDR spec.
// if port is different from the currently assigned one, set the "change port" bit
//
void SetPort(uint32_t ThreadNum, uint16_t PortNum);


//
// function to make an incoming or outgoing socket, bound to the specified port in the structure
// 1st parameter is a link into the socket data table
//
int MakeSocket(struct ThreadSocketData* Ptr, int DDCid);

//
// helper to get current socket FD for a thread, resolving shared-socket aliases
//
int GetThreadSocketFD(const struct ThreadSocketData* Ptr);

//
// helper to test if a thread uses a shared socket owned by another thread
//
bool ThreadSocketIsSharedAlias(const struct ThreadSocketData* Ptr);

//
// helper to mirror owner socket FD to any alias threads that share it
//
void SyncSocketAliasesForOwner(const struct ThreadSocketData* OwnerPtr);

//
// function ot get program version
//
uint32_t GetP3appVersion(void);





#endif
