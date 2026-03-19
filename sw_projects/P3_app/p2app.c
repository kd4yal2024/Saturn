/////////////////////////////////////////////////////////////
//
// Saturn project: Artix7 FPGA + Raspberry Pi4 Compute Module
// PCI Express interface from linux on Raspberry pi
// this application uses C code to emulate HPSDR protocol 2
//
// copyright Laurence Barker November 2021
// licenced under GNU GPL3
//
// p2app.c:
//
// Protocol2 is defined by "openHPSDR Ethernet Protocol V3.8"
// unlike protocol 1, it uses multiple ports for the data endpoints
//
//////////////////////////////////////////////////////////////


#include <stdio.h>
#include <errno.h>
#include <stdlib.h>
#include <limits.h>
#include <stdint.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <math.h>
#include <pthread.h>
#include <termios.h>
#include <sys/time.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <net/if.h>
#include <semaphore.h>
#include <signal.h>
#include <ifaddrs.h>
#include <netdb.h>
#include <sys/types.h>
#include <dirent.h>
#include <pthread.h>
#include <syscall.h>


#include "../common/saturntypes.h"
#include "../common/hwaccess.h"                     // access to PCIe read & write
#include "../common/saturnregisters.h"              // register I/O for Saturn
#include "../common/codecwrite.h"                   // codec register I/O for Saturn
#include "../common/version.h"                      // version I/O for Saturn
#include "../common/auxadc.h"                       // version I/O for Saturn
#include "../common/p23_perf_telemetry.h"

#include "threaddata.h"
#include "generalpacket.h"
#include "IncomingDDCSpecific.h"
#include "IncomingDUCSpecific.h"
#include "InHighPriority.h"
#include "InDUCIQ.h"
#include "InSpkrAudio.h"
#include "OutMicAudio.h"
#include "OutDDCIQ.h"
#include "OutHighPriority.h"
#include "Outwideband.h"
#include "cathandler.h"
#include "LDGATU.h"
#include "AriesATU.h"
#include "GanymedePAControl.h"
#include "frontpanelhandler.h"

#define P3APPVERSION 45
#define FWREQUIREDMAJORVERSION 1                  // major version that is required. Only altered if programming interface changes. 
//
// the Firmware version is a protection to make sure that if a p3app update is required by the new firmware,
// it won't work with an old version. This means p3app will always need to be updated if the firmware is updated to new major version.
//
//------------------------------------------------------------------------------------------
// VERSION History
// V45, 16/03/2026. encodes ADC1/ADC2 peak amplitudes into the high priority status message.
// V44, 31/01/2026.  Support for Thetis "push" CAT commands for Ganymede, g2v2 indicators & Aries instead of polling
// V43, 19/01/2026.  Initial support for Ganymede PA controller if stared with -g switch.
// V42, 05/01/2026.  Support for new codec; debug mode with startup switch to enable interlinked DDC at different frequencies
// V41, 30/9/2025:   Added detection of PCB version, so CODEC can be initialised for a different type
// V40: 29/6/2025:   Changes to accommodate a different XMDA device driver, if required in the future. No functional impact. 
// V39: 18/02/2025:  ADC overflow now reported immediately while in RX
// V38: 13/2/2025:   added FPGA version 18 required to run wideband code
// V37: 3/2/2025:    CAT reliability issues addressed to prevent crash if thetis CAT server turned off. 
// V36: 2/2/2025:    fixed G2V2 panel ID sent to Thetis. Fixed CAT thread 100% loading after 30s operation if no keepalive message sent (added keepalive thread). Thread PIDs displayed.
// V35: 1/2/2025:    removed warnings; no functional change
// V34: 21/01/2025:  changed code to find ethernet device name, not fixed eth0
// V33: 16/01/2025:  fix for OC outputs in wrong bit positions. CAT serial reliability fixed for front panel.
// V32: 30/12/2024:  added support for Arduino G2V1 front panel adapter, for processor upgrades. 
// V31: 21/11/2024:  added CW keyer keydown bit to high priority status byte 4 bit 7 for Thetis generated sidetone
// V30: 17/11/2024:  wideband record added.
// V29: 15/10/2024   DL1YCF CW ramp; CW amplitude corrected; added support to detect & check FPGA major version
// V27: 4/8/2024:    merged G2V2 panel support code into main
// V26: 17/7/2024:   initial support for G2V2 panel implemented. Polling CAT for LED states.
// V25: 22/6/2024:   merged branch with beta code for G2 panel controls to communicate via CAT over TCP/IP
// V24: 17/6/2024:   support for V17 firmware (fixed latency CW ramp sidetone)
// V23: 07/5/2024:   no functional change. Recognises firmware V16.
// V22: 06/05/2024:  CW ramp calculated by different C code (same shape). Enabled firmware V15.
// V21: 02/05/2024:  max CW ramp length extended to 20ms. Needs firmware V14.
// V20: 29/4/2024:   PA bit from Alex word 1 removed from code: wasn't being set by Thetis and 
//                   "general packet to SDR" has PA disable bit too
//
// V19: 7/4/2024:    PA disable bit supported. Checks for FPGA version: won't run with incompatible version
//
// V18: 1/4/2024:    matching updates for FW V 13. DUC FIFO =4096 depth; 
//                   TX scaling factor changed aster DUC firmware adjusted for TX noise improvement 
//
// V17: 13/3/2024:   CW ramp period is settable by client application.
//
// V16: 6/3/2024:    added interface for LDG ATU via CAT, requesting tune power when needed by ATU
//                   bare bones interface for G2 front panel
//                     

// V15: 16/01/2024:  added specific TXant bits from revised protocol 2 high priority message to resolve CW
//                   TX power generated momentarily into RX antenna if different
//                   reads CAT over TCP/IP port number
// V14: 17/12/2023:  added ATU tune request to IO6 bit position; FIFO under and overflow detection;
//                   changed FIFO sizes; debug can be enabled as runtime setting; enable/disable ext speaker;
//                   network timeout
// V13, 18/8/2023:   inverted IO8 sense for piHPSDR-initiated CW
// V12, 29/7/2023:   CW changes to set RX attenuation on TX from protocol bytes 58, 59;
//                   CW breakin properly enabled; CW keyer disabled if p2app not active;
//                   CW changes to minimise delay reporting to prototol 2



extern sem_t DDCInSelMutex;                 // protect access to shared DDC input select register
extern sem_t DDCResetFIFOMutex;             // protect access to FIFO reset register
extern sem_t RFGPIOMutex;                   // protect access to RF GPIO register
extern sem_t CodecRegMutex;                 // protect writes to codec
sem_t MicWBDMAMutex;                        // protect one DMA read channel shared by mic and WB read

struct sockaddr_in reply_addr;              // destination address for outgoing data
pthread_mutex_t g_reply_addr_mutex = PTHREAD_MUTEX_INITIALIZER;

atomic_bool IsTXMode = false;               // true if in TX
atomic_bool SDRActive = false;              // true if this SDR is running at the moment
atomic_bool ReplyAddressSet = false;        // true when reply address has been set
atomic_bool StartBitReceived = false;       // true when "run" bit has been set
atomic_bool NewMessageReceived = false;     // set whenever a message is received
atomic_bool ExitRequested = false;          // true if "exit checking" thread requests shutdown
bool SkipExitCheck = false;                 // true to skip "exit checking", if running as a service
atomic_bool ThreadError = false;            // true if a thread reports an error
bool UseDebug = false;                      // true if to enable debugging
bool UseControlPanel = false;               // true if to use a control panel
bool UseGanymede = false;                   // true if to use Ganymede PA protection
bool UseLDGATU = false;                     // true if to use an LDG ATU via CAT
bool UseAriesATU = false;                   // true if to use an Aries ATU
uint32_t LODebugDDC1Frequency;              // -x debug mode: LO frequency for DDC1
bool InterleavedDDCDebugMode = false;       // true if interleaved DDC for debug are allowed
static volatile sig_atomic_t g_signal_exit_requested = 0;

static void SyncSignalExitRequest(void)
{
  if(g_signal_exit_requested != 0)
    atomic_store(&ExitRequested, true);
}


#define SDRBOARDID 1                        // Hermes
#define SDRSWVERSION 1                      // version of this software
#define VDISCOVERYSIZE 60                   // discovery packet
#define VDISCOVERYREPLYSIZE 60              // reply packet
#define VWIDEBANDSIZE 1028                  // wideband scalar samples
#define VCONSTTXAMPLSCALEFACTOR 0x0001FFFF  // 18 bit scale value - set to 1/2 of full scale
#define VCONSTTXAMPLSCALEFACTOR_13 0x0002000  // 18 bit scale value - set to 1/32 of full scale FWV13+
#define VCONSTTXAMPLSCALEFACTOR_17 0x0002000  // 18 bit scale value - set to 1/32 of full scale FWV17+
//#define VCONSTTXAMPLSCALEFACTOR_17 0x0002800  // 18 bit scale value - set to 1/32 of full scale FWV17+
#define VCONSTTXAMPLSCALEFACTOR_PCBV3 0x0002A00  // 18 bit scale value - set to 1/32 of full scale for PCB V3

struct ThreadSocketData SocketData[VPORTTABLESIZE] =
{
  {0, 0, 1024, "Cmd", false,{}, 0, 0},                      // command (incoming) thread
  {0, 0, 1025, "DDC Specific", false,{}, 0, 0},             // DDC specifc (incoming) thread
  {0, 0, 1026, "DUC Specific", false,{}, 0, 0},             // DUC specific (incoming) thread
  {0, 0, 1027, "High Priority In", false,{}, 0, 0},         // High Priority (incoming) thread
  {0, 0, 1028, "Spkr Audio", false,{}, 0, 0},               // Speaker Audio (incoming) thread
  {0, 0, 1029, "DUC I/Q", false,{}, 0, 0},                  // DUC IQ (incoming) thread
  {0, 0, 1025, "High Priority Out", false,{}, 0, 0},        // High Priority (outgoing) thread
  {0, 0, 1026, "Mic Audio", false,{}, 0, 0},                // Mic Audio (outgoing) thread
  {0, 0, 1035, "DDC I/Q 0", false,{}, 0, 0},                // DDC IQ 0 (outgoing) thread
  {0, 0, 1036, "DDC I/Q 1", false,{}, 0, 0},                // DDC IQ 1 (outgoing) thread
  {0, 0, 1037, "DDC I/Q 2", false,{}, 0, 0},                // DDC IQ 2 (outgoing) thread
  {0, 0, 1038, "DDC I/Q 3", false,{}, 0, 0},                // DDC IQ 3 (outgoing) thread
  {0, 0, 1039, "DDC I/Q 4", false,{}, 0, 0},                // DDC IQ 4 (outgoing) thread
  {0, 0, 1040, "DDC I/Q 5", false,{}, 0, 0},                // DDC IQ 5 (outgoing) thread
  {0, 0, 1041, "DDC I/Q 6", false,{}, 0, 0},                // DDC IQ 6 (outgoing) thread
  {0, 0, 1042, "DDC I/Q 7", false,{}, 0, 0},                // DDC IQ 7 (outgoing) thread
  {0, 0, 1043, "DDC I/Q 8", false,{}, 0, 0},                // DDC IQ 8 (outgoing) thread
  {0, 0, 1044, "DDC I/Q 9", false,{}, 0, 0},                // DDC IQ 9 (outgoing) thread
  {0, 0, 1027, "Wideband 0", false,{}, 0, 0},               // Wideband 0 (outgoing) thread
  {0, 0, 1028, "Wideband 1", false,{}, 0, 0}                // Wideband 1 (outgoing) thread
};


//
// default port numbers, used if incoming port number = 0
//
uint16_t DefaultPorts[VPORTTABLESIZE] =
{
  1024, 1025, 1026, 1027, 1028, 
  1029, 1025, 1026, 1035, 1036, 
  1037, 1038, 1039, 1040, 1041, 
  1042, 1043, 1044, 1027, 1028
};


pthread_t DDCSpecificThread;
pthread_t DUCSpecificThread;
pthread_t HighPriorityToSDRThread;
pthread_t SpkrAudioThread;
pthread_t DUCIQThread;
pthread_t DDCIQThread[VNUMDDC];               // array, but not sure how many
pthread_t MicThread;
pthread_t HighPriorityFromSDRThread;
pthread_t WidebandDataThread;
static bool DDCSpecificThreadStarted = false;
static bool DUCSpecificThreadStarted = false;
static bool HighPriorityToSDRThreadStarted = false;
static bool SpkrAudioThreadStarted = false;
static bool DUCIQThreadStarted = false;
static bool DDCIQThreadStarted[VNUMDDC] = {false};
static bool MicThreadStarted = false;
static bool HighPriorityFromSDRThreadStarted = false;
static bool WidebandDataThreadStarted = false;

pthread_t CheckForExitThread;                 // thread looks for types "exit" command
pthread_t CheckForNoActivityThread;           // thread looks for inactvity
static bool CheckForExitThreadStarted = false;
static bool CheckForNoActivityThreadStarted = false;
static pthread_mutex_t g_general_packet_mutex = PTHREAD_MUTEX_INITIALIZER;
static uint8_t g_pending_general_packet[VDISCOVERYSIZE];
static uint8_t g_last_applied_general_packet[VDISCOVERYSIZE];
static bool g_pending_general_packet_valid = false;
static bool g_last_applied_general_packet_valid = false;
static atomic_bool g_startup_discovery_logged = false;
static atomic_bool g_startup_general_rx_logged = false;
static atomic_bool g_startup_general_applied_logged = false;
static atomic_bool g_startup_run_logged = false;
static atomic_bool g_startup_active_logged = false;

static void MaybeLogStartupEvent(atomic_bool* EventFlag, const char* EventText)
{
  if((EventFlag == NULL) || (EventText == NULL))
    return;

  if(!atomic_exchange(EventFlag, true))
  {
    printf("STARTUP: %s [reply=%d run=%d active=%d]\n",
            EventText,
            atomic_load(&ReplyAddressSet),
            atomic_load(&StartBitReceived),
            atomic_load(&SDRActive));
  }
}

void MarkStartupRunBitSeen(void)
{
  MaybeLogStartupEvent(&g_startup_run_logged, "HighPriority run-bit received");
}

void MarkStartupHandshakeComplete(void)
{
  MaybeLogStartupEvent(&g_startup_active_logged, "Startup handshake complete");
}

void ResetStartupTraceFlags(void)
{
  atomic_store(&g_startup_discovery_logged, false);
  atomic_store(&g_startup_general_rx_logged, false);
  atomic_store(&g_startup_general_applied_logged, false);
  atomic_store(&g_startup_run_logged, false);
  atomic_store(&g_startup_active_logged, false);
}

static bool QueueGeneralPacketForApply(const uint8_t* PacketBuffer, size_t PacketLen)
{
  bool Updated;

  if((PacketBuffer == NULL) || (PacketLen != VDISCOVERYSIZE))
    return false;

  pthread_mutex_lock(&g_general_packet_mutex);
  Updated = (!g_pending_general_packet_valid) ||
            (memcmp(g_pending_general_packet, PacketBuffer, VDISCOVERYSIZE) != 0);
  if(Updated)
    memcpy(g_pending_general_packet, PacketBuffer, VDISCOVERYSIZE);
  g_pending_general_packet_valid = true;
  pthread_mutex_unlock(&g_general_packet_mutex);
  return Updated;
}

static void MaybeActivateFromStartupHandshake(void);

static int ApplyQueuedGeneralPacketIfStable(void)
{
  uint8_t LocalPacket[VDISCOVERYSIZE];
  bool HasPending;
  bool IsNoOp;

  pthread_mutex_lock(&g_general_packet_mutex);
  HasPending = g_pending_general_packet_valid;
  if(!HasPending)
  {
    pthread_mutex_unlock(&g_general_packet_mutex);
    return 0;
  }

  memcpy(LocalPacket, g_pending_general_packet, VDISCOVERYSIZE);
  g_pending_general_packet_valid = false;
  IsNoOp = g_last_applied_general_packet_valid &&
           (memcmp(g_last_applied_general_packet, LocalPacket, VDISCOVERYSIZE) == 0);
  if(!IsNoOp)
  {
    memcpy(g_last_applied_general_packet, LocalPacket, VDISCOVERYSIZE);
    g_last_applied_general_packet_valid = true;
  }
  pthread_mutex_unlock(&g_general_packet_mutex);

  if(IsNoOp)
  {
    // Duplicate general packets still refresh startup handshake state.
    atomic_store(&ReplyAddressSet, true);
    return 0;
  }

  HandleGeneralPacket(LocalPacket);
  MaybeLogStartupEvent(&g_startup_general_applied_logged, "General packet applied");
  atomic_store(&ReplyAddressSet, true);
  return 1;
}

static void MaybeActivateFromStartupHandshake(void)
{
  if(!atomic_load(&SDRActive) &&
      atomic_load(&ReplyAddressSet) &&
      atomic_load(&StartBitReceived))
  {
    atomic_store(&SDRActive, true);
    SetTXEnable(true);
    MarkStartupHandshakeComplete();
  }
}

static int ApplyQueuedOutgoingPortRebinds(void)
{
  int i;

  for(i = VPORTHIGHPRIORITYFROMSDR; i < VPORTTABLESIZE; i++)
  {
    struct ThreadSocketData* ThreadData = SocketData + i;
    if(!(atomic_load(&ThreadData->Cmdid) & VBITCHANGEPORT))
      continue;

    if(ThreadSocketIsSharedAlias(ThreadData))
    {
      int Socketfd = atomic_load(&ThreadData->Socketid);
      int SharedSocketfd = GetThreadSocketFD(ThreadData);
      if((Socketfd > 0) && (Socketfd != SharedSocketfd))
      {
        close(Socketfd);
        atomic_store(&ThreadData->Socketid, 0);
      }
    }
    else
    {
      int Socketfd = atomic_load(&ThreadData->Socketid);
      if(Socketfd > 0)
      {
        close(Socketfd);
        atomic_store(&ThreadData->Socketid, 0);
      }
      if(MakeSocket(ThreadData, ThreadData->DDCid) != 0)
      {
        perror("control-plane MakeSocket");
        atomic_store(&ThreadError, true);
        return -1;
      }
    }
    atomic_fetch_and(&ThreadData->Cmdid, ~((uint_fast32_t)VBITCHANGEPORT));
  }
  return 0;
}


//
// socket ownership mapping for shared-port threads.
// by default these streams share sockets; if a general packet assigns
// different ports, the stream falls back to an independent socket.
//
static uint32_t ResolveSocketOwnerIndex(uint32_t ThreadNum)
{
  switch(ThreadNum)
  {
    case VPORTMICAUDIO:
      return VPORTDUCSPECIFIC;

    case VPORTHIGHPRIORITYFROMSDR:
      return VPORTDDCSPECIFIC;

    case VPORTWIDEBAND0:
      return VPORTHIGHPRIORITYTOSDR;

    case VPORTWIDEBAND1:
      return VPORTSPKRAUDIO;

    default:
      return ThreadNum;
  }
}

static bool ThreadSocketShouldShareAlias(const struct ThreadSocketData* Ptr)
{
  uint32_t ThreadNum;
  uint32_t OwnerNum;

  if((Ptr == NULL) || (Ptr < SocketData) || (Ptr >= (SocketData + VPORTTABLESIZE)))
    return false;

  ThreadNum = (uint32_t)(Ptr - SocketData);
  OwnerNum = ResolveSocketOwnerIndex(ThreadNum);
  if(OwnerNum == ThreadNum)
    return false;

  return (atomic_load(&SocketData[ThreadNum].Portid) == atomic_load(&SocketData[OwnerNum].Portid));
}

int GetThreadSocketFD(const struct ThreadSocketData* Ptr)
{
  uint32_t ThreadNum;
  uint32_t OwnerNum;
  int Socketfd;

  if((Ptr == NULL) || (Ptr < SocketData) || (Ptr >= (SocketData + VPORTTABLESIZE)))
    return -1;

  ThreadNum = (uint32_t)(Ptr - SocketData);
  OwnerNum = ResolveSocketOwnerIndex(ThreadNum);

  // alias streams can split to dedicated sockets if their port differs.
  // fallback to owner socket until dedicated socket is actually open.
  if((OwnerNum != ThreadNum) && !ThreadSocketShouldShareAlias(Ptr))
  {
    Socketfd = atomic_load(&SocketData[ThreadNum].Socketid);
    if(Socketfd > 0)
      return Socketfd;
  }
  return atomic_load(&SocketData[OwnerNum].Socketid);
}

bool ThreadSocketIsSharedAlias(const struct ThreadSocketData* Ptr)
{
  return ThreadSocketShouldShareAlias(Ptr);
}

void SyncSocketAliasesForOwner(const struct ThreadSocketData* OwnerPtr)
{
  (void)OwnerPtr;
  // no-op: shared socket aliases were removed for protocol compatibility.
}


//
// function ot get program version
//
uint32_t GetP3appVersion(void)
{
  return P3APPVERSION;
}

void sig_handler(int signo)
{
    if (signo == SIGINT)
        g_signal_exit_requested = 1;
}

//
// function to check if any threads are still active
// loop through the table; report if any are true.
// parameter is to allow the "command" socket to stay open
//
bool CheckActiveThreads(int StartingPoint)
{
  struct ThreadSocketData* Ptr = SocketData+StartingPoint;
  bool Result = false;

  for (int i = StartingPoint; i < VPORTTABLESIZE; i++)          // loop through the socket table
  {
    if(atomic_load(&Ptr->Active))                   // check this thread
      Result = true;
    Ptr++;
  }
    if(Result)
      printf("found an active thread\n");
    return Result;
}



//
// set the port for a given thread. If 0, set the default according to HPSDR spec.
// if port is different from the currently assigned one, set the "change port" bit
//
void SetPort(uint32_t ThreadNum, uint16_t PortNum)
{
  uint16_t CurrentPort;
  uint16_t NewPort;
  bool WasShared = false;
  bool IsShared = false;

  if(ResolveSocketOwnerIndex(ThreadNum) != ThreadNum)
    WasShared = ThreadSocketShouldShareAlias(&SocketData[ThreadNum]);
  CurrentPort = atomic_load(&SocketData[ThreadNum].Portid);
  NewPort = (PortNum == 0) ? DefaultPorts[ThreadNum] : PortNum;
  atomic_store(&SocketData[ThreadNum].Portid, NewPort);
  P23PerfTelemetrySetPort(ThreadNum, NewPort);
  if(ResolveSocketOwnerIndex(ThreadNum) != ThreadNum)
    IsShared = ThreadSocketShouldShareAlias(&SocketData[ThreadNum]);

  if ((NewPort != CurrentPort) || (WasShared != IsShared))
    atomic_fetch_or(&SocketData[ThreadNum].Cmdid, (uint_fast32_t)VBITCHANGEPORT);

  if(NewPort != CurrentPort)
  {
    if(ThreadNum == VPORTDUCSPECIFIC)
      atomic_fetch_or(&SocketData[VPORTMICAUDIO].Cmdid, (uint_fast32_t)VBITCHANGEPORT);
    else if(ThreadNum == VPORTDDCSPECIFIC)
      atomic_fetch_or(&SocketData[VPORTHIGHPRIORITYFROMSDR].Cmdid, (uint_fast32_t)VBITCHANGEPORT);
    else if(ThreadNum == VPORTHIGHPRIORITYTOSDR)
      atomic_fetch_or(&SocketData[VPORTWIDEBAND0].Cmdid, (uint_fast32_t)VBITCHANGEPORT);
    else if(ThreadNum == VPORTSPKRAUDIO)
      atomic_fetch_or(&SocketData[VPORTWIDEBAND1].Cmdid, (uint_fast32_t)VBITCHANGEPORT);
  }
}



//
// function to make an incoming or outgoing socket, bound to the specified port in the structure
// 1st parameter is a link into the socket data table
//
int MakeSocket(struct ThreadSocketData* Ptr, int DDCid)
{
  struct timeval ReadTimeout;                                       // read timeout
  int yes = 1;
  int ReceiveBufferSize = 512 * 1024;                               // absorb short scheduler/network jitter bursts
  int SendBufferSize = 256 * 1024;                                  // keep outbound UDP writes from stalling on tiny buffers
  int Socketfd;
  uint16_t Portid;
//  struct sockaddr_in addr_cmddata;
  //
  // create socket for incoming data
  //
  Socketfd = socket(AF_INET, SOCK_DGRAM, 0);
  if(Socketfd < 0)
  {
    perror("socket fail");
    return EXIT_FAILURE;
  }
  atomic_store(&Ptr->Socketid, Socketfd);

  //
  // set 1ms timeout, and re-use any recently open ports
  //
  setsockopt(Socketfd, SOL_SOCKET, SO_REUSEADDR, (void *)&yes , sizeof(yes));
  setsockopt(Socketfd, SOL_SOCKET, SO_RCVBUF, (void *)&ReceiveBufferSize, sizeof(ReceiveBufferSize));
  setsockopt(Socketfd, SOL_SOCKET, SO_SNDBUF, (void *)&SendBufferSize, sizeof(SendBufferSize));
  ReadTimeout.tv_sec = 0;
  ReadTimeout.tv_usec = 1000;
  setsockopt(Socketfd, SOL_SOCKET, SO_RCVTIMEO, (void *)&ReadTimeout , sizeof(ReadTimeout));

  //
  // bind application to the specified port
  //
  Portid = atomic_load(&Ptr->Portid);
  memset(&Ptr->addr_cmddata, 0, sizeof(struct sockaddr_in));
  Ptr->addr_cmddata.sin_family = AF_INET;
  Ptr->addr_cmddata.sin_addr.s_addr = htonl(INADDR_ANY);
  Ptr->addr_cmddata.sin_port = htons(Portid);

  if(bind(Socketfd, (struct sockaddr *)&Ptr->addr_cmddata, sizeof(struct sockaddr_in)) < 0)
  {
    perror("bind");
    close(Socketfd);
    atomic_store(&Ptr->Socketid, 0);
    return EXIT_FAILURE;
  }

  struct sockaddr_in checkin;
  socklen_t len = sizeof(checkin);
  if(getsockname(Socketfd, (struct sockaddr *)&checkin, &len)==-1)
    perror("getsockname");

  Ptr->DDCid = DDCid;                       // set DDC number, for outgoing ports
  SyncSocketAliasesForOwner(Ptr);           // mirror owner socket FD to any shared-alias thread entries
  return 0;
}


//
// this runs as its own thread to monitor command line activity. A string "exist" exits the application. 
// thread initiated at the start.
//
void* CheckForExitCommand(__attribute__((unused)) void *arg)
{
  int Flags;
  int ReadCount;
  char ch;
  printf("spinning up Check For Exit thread, pid=%ld\n", syscall(SYS_gettid));

  Flags = fcntl(STDIN_FILENO, F_GETFL, 0);
  if(Flags != -1)
    fcntl(STDIN_FILENO, F_SETFL, Flags | O_NONBLOCK);

  while (!atomic_load(&ExitRequested))
  {
    usleep(10000);
    ReadCount = read(STDIN_FILENO, &ch, 1);
    if(ReadCount > 0)
    {
      if((ch == 'x') || (ch == 'X'))
      {
        atomic_store(&ExitRequested, true);
        break;
      }
    }
  }

  if(Flags != -1)
    fcntl(STDIN_FILENO, F_SETFL, Flags);

  return NULL;
}


//
// this runs as its own thread to see if messages have stopped being received.
// if nomessages in a second, goes back to "inactive" state.
//
void* CheckForActivity(__attribute__((unused)) void *arg)
{
  bool PreviouslyActiveState;               
  printf("Started check for activity thread, pid=%ld\n", syscall(SYS_gettid));
  while(!atomic_load(&ExitRequested))
  {
    sleep(1);                                   // wait for 1 second
    PreviouslyActiveState = atomic_load(&SDRActive);          // see if active on entry
    if (!atomic_load(&NewMessageReceived) && atomic_load(&HW_Timer_Enable)) // if no messages received,
    {
      atomic_store(&SDRActive, false);          // set back to inactive
      atomic_store(&IsTXMode, false);
      SetMOX(false);
      SetTXEnable(false);
      EnableCW(false, false);
      atomic_store(&ReplyAddressSet, false);
      atomic_store(&StartBitReceived, false);
      if(PreviouslyActiveState)
      {
        printf("Reverted to Inactive State after no activity\n");
        ResetStartupTraceFlags();
      }
    }
    atomic_store(&NewMessageReceived, false);
  }
  return NULL;
}




//
// Shutdown()
// perform ordely shutdown of the program
//
void Shutdown()
{
  int i;

  atomic_store(&ExitRequested, true);
  if(CheckForExitThreadStarted)
  {
    pthread_join(CheckForExitThread, NULL);
    CheckForExitThreadStarted = false;
  }
  if(CheckForNoActivityThreadStarted)
  {
    pthread_join(CheckForNoActivityThread, NULL);
    CheckForNoActivityThreadStarted = false;
  }
  if(DDCSpecificThreadStarted)
  {
    pthread_join(DDCSpecificThread, NULL);
    DDCSpecificThreadStarted = false;
  }
  if(DUCSpecificThreadStarted)
  {
    pthread_join(DUCSpecificThread, NULL);
    DUCSpecificThreadStarted = false;
  }
  if(HighPriorityToSDRThreadStarted)
  {
    pthread_join(HighPriorityToSDRThread, NULL);
    HighPriorityToSDRThreadStarted = false;
  }
  if(SpkrAudioThreadStarted)
  {
    pthread_join(SpkrAudioThread, NULL);
    SpkrAudioThreadStarted = false;
  }
  if(DUCIQThreadStarted)
  {
    pthread_join(DUCIQThread, NULL);
    DUCIQThreadStarted = false;
  }
  if(MicThreadStarted)
  {
    pthread_join(MicThread, NULL);
    MicThreadStarted = false;
  }
  if(HighPriorityFromSDRThreadStarted)
  {
    pthread_join(HighPriorityFromSDRThread, NULL);
    HighPriorityFromSDRThreadStarted = false;
  }
  for(i = 0; i < VNUMDDC; i++)
  {
    if(DDCIQThreadStarted[i])
    {
      pthread_join(DDCIQThread[i], NULL);
      DDCIQThreadStarted[i] = false;
    }
  }
  if(WidebandDataThreadStarted)
  {
    pthread_join(WidebandDataThread, NULL);
    WidebandDataThreadStarted = false;
  }

  ShutdownCATHandler();                                   // close CAT connection socket
  if(UseControlPanel)
    ShutdownFrontPanelHandler();
  if(UseAriesATU)
    ShutdownAriesHandler();
  if(UseGanymede)
    ShutdownGanymedeHandler();

  close(atomic_load(&SocketData[0].Socketid));            // close incoming data socket
  sem_destroy(&DDCInSelMutex);
  sem_destroy(&DDCResetFIFOMutex);
  sem_destroy(&RFGPIOMutex);
  sem_destroy(&CodecRegMutex);
  sem_destroy(&MicWBDMAMutex);                            // for DMA
  SetMOX(false);
  SetTXEnable(false);
  EnableCW(false, false);
}



//
// main program. Initialise, then handle incoming command/general data
// has a loop that reads & processes incoming command packets
// see protocol documentation
// 
// if invoked "./p3app" - ADCs selected as normal
// if invoked "./p3app 1900000" - ADC1 and ADC2 inputs set to DDS test source at 1900000Hz
//
int main(int argc, char *argv[])
{
  int i, size;
//
// part written discovery reply packet
//
  uint8_t DiscoveryReply[VDISCOVERYREPLYSIZE] = 
  {
    0,0,0,0,                                      // sequence bytes
    2,                                            // 2 if not active; 3 if active
    0,0,0,0,0,0,                                  // SDR (raspberry i) MAC address
    10,                                           // board type. changed from "orion mk2" to "saturn"
    43,                                           // protocol version 4.3
    20,                                           // this SDR firmware version. >17 to enable QSK
    0,0,0,0,0,0,                                  // Mercury, Metis, Penny version numbers
    4,                                            // 4DDC
    1,                                            // phase word
    0,                                            // endian mode
    0,0,                                          // beta version, reserved byte (total 25 useful bytes)
    0,0,0,0,0,0,0,0,0,0,                          // 10 bytes padding
    0,0,0,0,0,0,0,0,0,0,                          // 10 bytes padding
    0,0,0,0,0,0,0,0,0,0,0,0,0,0                   // 15 bytes padding
  };

  uint8_t CmdByte;                                                  // command word from PC app
  struct ifreq hwaddr;                                              // holds this device MAC address
  struct sockaddr_in addr_from;                                     // holds MAC address of source of incoming messages
  uint8_t UDPInBuffer[VDDCPACKETSIZE];                              // outgoing buffer
  struct iovec iovecinst;                                           // iovcnt buffer - 1 for each outgoing buffer
  struct msghdr datagram;                                           // multiple incoming message header

  uint32_t TestFrequency;                                           // -f test source DDS freq
  int CmdOption;                                                    // command line option
  char BuildDate[]=GIT_DATE;
	ESoftwareID ID;
	unsigned int Version = 0;
  unsigned int MajorVersion = 0;
  bool IncompatibleFirmware = false;                                // becomes set if firmware is not compatible with this version
  unsigned int PCBVersion;

  //
  // initialise register access semaphores
  //
  sem_init(&DDCInSelMutex, 0, 1);                                   // for DDC input select register
  sem_init(&DDCResetFIFOMutex, 0, 1);                               // for FIFO reset register
  sem_init(&RFGPIOMutex, 0, 1);                                     // for RF GPIO register
  sem_init(&CodecRegMutex, 0, 1);                                   // for codec accesss
  sem_init(&MicWBDMAMutex, 0, 1);                                   // for mic and WB DMA
  P23PerfTelemetryInit("p3", GetP3appVersion());
  for(i = 0; i < VPORTTABLESIZE; i++)
    P23PerfTelemetrySetPort((unsigned int)i, atomic_load(&SocketData[i].Portid));
    
//
// setup Saturn hardware
//
  printf("SATURN P3 App (Protocol 2 compatible). press 'x <enter>' in console to close\n");

  if(!OpenXDMADriver(false))
  {
    printf("unable to continue without XDMA register access\n");
    return EXIT_FAILURE;
  }
  PrintVersionInfo();
  PCBVersion = GetPCBVersionNumber();
  printf("p3app client app software Version:%d Build Date:%s\n", P3APPVERSION, BuildDate);
  PrintAuxADCInfo();
  if (IsFallbackConfig())
      printf("FPGA load is a fallback - you should re-flash the primary FPGA image!\n");
  
  SetSpkrMute(true);                                                // mute speaker before initialising codec
  usleep(10000);
  CodecInitialise(PCBVersion);
  InitialiseDACAttenROMs();
//  InitialiseCWKeyerRamp(true, 5000);                              // create initial default 5 ms ramp, P2
  InitialiseCWKeyerRamp(true, 9000);                                // create initial default 9ms DL1YCF amp, P2
  SetCWSidetoneEnabled(true);
  SetTXProtocol(true);                                              // set to protocol 2
  SetTXModulationSource(eIQData);                                   // disable debug options
  HandlerSetEERMode(false);                                         // no EER
  SetByteSwapping(true);                                            // h/w to generate network byte order
  SetSpkrMute(false);

  Version = GetFirmwareVersion(&ID);                                // TX scaling changed at FW V13
  MajorVersion = GetFirmwareMajorVersion();

  if(PCBVersion <= 2)
  {
    if(Version < 13)
      SetTXAmplitudeScaling(VCONSTTXAMPLSCALEFACTOR);
    else if (Version < 17)
      SetTXAmplitudeScaling(VCONSTTXAMPLSCALEFACTOR_13);
    else
      SetTXAmplitudeScaling(VCONSTTXAMPLSCALEFACTOR_17);
  }
  else
  {
    SetTXAmplitudeScaling(VCONSTTXAMPLSCALEFACTOR_PCBV3);
  }  


  if (MajorVersion != FWREQUIREDMAJORVERSION)
  {
    printf("\n***************************************************************************\n");
    printf("***************************************************************************\n");
    printf("Incompatible Saturn FPGA firmware v%d; major version%d\n",
             Version,  MajorVersion);
    printf("This version of p3app requires major version = %d\n", FWREQUIREDMAJORVERSION);
    printf("You must update your copy of p3app to use that firmware version - see User manual\n");
    printf("p3app will refuse a connection request until this is resolved!\n");
    printf("\n\n\n***************************************************************************\n");
    IncompatibleFirmware = true;
  }

  // SetTXEnable(true);                                             // now only enabled if SDR active
  EnableAlexManualFilterSelect(true);
  SetBalancedMicInput(false);
  InitCATHandler();

  struct sigaction sa;
  memset(&sa, 0, sizeof(sa));
  sa.sa_handler = sig_handler;
  sigemptyset(&sa.sa_mask);
  sa.sa_flags = 0;
  if (sigaction(SIGINT, &sa, NULL) == -1)
    printf("\ncan't catch SIGINT\n");

//
// start up thread to check for no longer getting messages, to set back to inactive
//
  if(pthread_create(&CheckForNoActivityThread, NULL, CheckForActivity, NULL) != 0)
  {
    perror("pthread_create check for exit");
    return EXIT_FAILURE;
  }
  CheckForNoActivityThreadStarted = true;

//
// option string needs a colon after each option letter that has a parameter after it
// and it has a leading colon to suppress error messages
//
  while((CmdOption = getopt(argc, argv, ":a:i:f:x:m:sdphg")) != -1)
  {
    switch(CmdOption)
    {
      case 'h':
        printf("usage: ./p3app <optional arguments>\n");
        printf("optional arguments:\n");
        printf("-a LDG        control TUNE for LDG ATU\n");
        printf("-a Aries      control TUNE for Aries ATU\n");
        printf("-f <frequency in Hz> turns on test source for all DDCs\n");
        printf("-g            enables PA protection (G2-1k only)\n");
        printf("-i saturn     board responds as board id = Saturn\n");
        printf("-i orionmk2   board responds as board id = Orion mk 2\n");
        printf("-m xlr        selects balanced XLR microphone input\n");
        printf("-m jack       selects unbalanced 3.5mm microphone input\n");
        printf("-s            skip checking for exit keys, run as service\n");
        printf("-d            print additional debug\n");
        printf("-p            drive G2 control panel\n");
        printf("-x            dubin mode to allow interleaved DDC on different frquencies\n");
        return EXIT_SUCCESS;
        break;

      case 'a':
        if(strcmp(optarg,"LDG") == 0)
        {
          printf("TUNE command for LDG ATU via CAT\n");
          UseLDGATU = true;
        }
        else if(strcmp(optarg,"Aries") == 0)
        {
          printf("Interface for Aries ATU vRequested\n");
          UseAriesATU = true;
        }
        else
        {
          printf("error parsing ATU type. Command is case sensitive\n");
          printf("-a LDG    selects LDG ATU\n");
          printf("-a Aries      control TUNE for Aries ATU\n");
          return EXIT_SUCCESS;
        }
        break;

      case 'g':
        printf ("Ganymede PA control enabled\n");
        UseGanymede = true;
        break;

      case 'i':
        if(strcmp(optarg,"saturn") == 0)
        {
          printf("Discovery will respond as Saturn\n");
          DiscoveryReply[11] = 10;
        }
        else if(strcmp(optarg,"orionmk2") == 0)
        {
          printf("Discovery will respond as Orion mk 2\n");
          DiscoveryReply[11] = 5;
        }
        else
        {
          printf("error parsing board id. Values must be lower case\n");
          printf("-i saturn     board responds as board id = Saturn\n");
          printf("-i orionmk2   board responds as board id = Orion mk 2\n");
          return EXIT_SUCCESS;
        }
        break;


      case 'm':
        if(strcmp(optarg,"xlr") == 0)
        {
          printf("XLR mic input selected\n");
          SetBalancedMicInput(true);
        }
        else if(strcmp(optarg,"jack") == 0)
        {
          printf("unbalanced mic input selected\n");
          SetBalancedMicInput(false);
        }
        else
        {
          printf("error parsing microphone type. Values must be lower case\n");
          printf("-m xlr    selects balanced XLR microphone input\n\n");
          printf("-m jack   selects unbalanced 3.5mm microphone input\n");
          return EXIT_SUCCESS;
        }
        break;

      case 'f':
        TestFrequency = (atoi(optarg));
        SetTestDDSFrequency(TestFrequency, false);   
        UseTestDDSSource();         
        printf ("Test source selected, frequency = %dHz\n", TestFrequency);                  
        break;

      case 's':
        printf ("Skipping check for exit keys\n");                  
        SkipExitCheck = true;
        break;

      case 'd':
        printf ("Enhanced debug enabled\n");                  
        UseDebug = true;
        break;

      case 'p':
        printf ("Control panel enabled\n");                  
        UseControlPanel = true;
        break;

      case 'x':
        printf ("DEBUG ONLY interleaved DDC separate LO enabled\n");                  
        InterleavedDDCDebugMode = true;
        LODebugDDC1Frequency = (atoi(optarg));
        printf ("Fixed DDC1 selected, frequency = %dHz\n", LODebugDDC1Frequency);                  
        break;

    }
  }
  printf("\n");
  P23PerfTelemetrySetFeatureFlags(UseControlPanel, UseGanymede, UseLDGATU, UseAriesATU);


//
// startup ATU handler if needed
//
  if(UseLDGATU)
    InitialiseLDGHandler();

//
// startup ATU handler if needed
//
  if(UseAriesATU)
    InitialiseAriesHandler();
  if(UseGanymede)
    InitialiseGanymedeHandler();

//
// startup G2 front panel handler if needed
//
  if(UseControlPanel)
    InitialiseFrontPanelHandler();

//
// set paramter for interleaved DDC debug mode
//
  if(InterleavedDDCDebugMode)
    EnableInterleavedDDCLODebug(InterleavedDDCDebugMode);

//
// start up thread for exit command checking
//
  if (SkipExitCheck == false)
  {
    if(pthread_create(&CheckForExitThread, NULL, CheckForExitCommand, NULL) != 0)
    {
      perror("pthread_create check for exit");
      return EXIT_FAILURE;
    }
    CheckForExitThreadStarted = true;
  }

  //
  // create socket for incoming data on the command port
  //
  if(MakeSocket(SocketData, 0) != 0)
  {
    printf("failed to create command socket\n");
    return EXIT_FAILURE;
  }

  

  //
  // get this device MAC address
  // original code joust picked up interface eth0, but that doesn't work with Radxa CM5
  // revised code enumerated the interfaces, but had a startup race condition
  //

  
#if 0 // original p2app code
  memset(&hwaddr, 0, sizeof(hwaddr));
  strncpy(hwaddr.ifr_name, "eth0", IFNAMSIZ - 1);
  ioctl(SocketData[VPORTCOMMAND].Socketid, SIOCGIFHWADDR, &hwaddr);
  for(i = 0; i < 6; ++i) DiscoveryReply[i + 5] = hwaddr.ifr_addr.sa_data[i];         // copy MAC to reply message

#else // newer way
  DIR *dp;
  struct dirent *ep;
  char *posp;
  int ch = 'e';                                    // start character ethernet
  char InterfaceName[IFNAMSIZ] = {0};
  bool FoundInterface = false;

    dp = opendir("/sys/class/net");
    if (dp != NULL)
    {
      while ((ep = readdir(dp)) != NULL)
      {
        if ( !strcmp(ep->d_name, ".") || !strcmp(ep->d_name, "..") || !strcmp(ep->d_name, "lo") )
        {
          continue;
        }
        posp = strchr(ep->d_name, ch);
        if ( posp == ep->d_name ) {
          strncpy(InterfaceName, ep->d_name, IFNAMSIZ - 1);
          InterfaceName[IFNAMSIZ - 1] = '\0';
          printf("%s: interface name: %s\n", __FUNCTION__, InterfaceName);
          FoundInterface = true;
          break;
        }
      }
      (void) closedir(dp);
    }
    else
    {
      printf("%s: Couldn't open the directory\n", __FUNCTION__);
      return EXIT_FAILURE;
    }
    if(!FoundInterface)
    {
      printf("%s: No ethernet interface found\n", __FUNCTION__);
      return EXIT_FAILURE;
    }
    memset(&hwaddr, 0, sizeof(hwaddr));
    snprintf(hwaddr.ifr_name, sizeof(hwaddr.ifr_name), "%s", InterfaceName);
    if(ioctl(atomic_load(&SocketData[VPORTCOMMAND].Socketid), SIOCGIFHWADDR, &hwaddr) != 0)
    {
      perror("ioctl SIOCGIFHWADDR");
      return EXIT_FAILURE;
    }
    for(i = 0; i < 6; ++i) DiscoveryReply[i + 5] = hwaddr.ifr_addr.sa_data[i];         // copy MAC to reply message
#endif
  DiscoveryReply[13] = (uint8_t)Version;
  DiscoveryReply[23] = (uint8_t)P3APPVERSION;
  


  if(MakeSocket(SocketData+VPORTDDCSPECIFIC, 0) != 0)            // create and bind a socket
  {
    printf("failed to create DDC specific socket\n");
    return EXIT_FAILURE;
  }
  if(pthread_create(&DDCSpecificThread, NULL, IncomingDDCSpecific, (void*)&SocketData[VPORTDDCSPECIFIC]) != 0)
  {
    perror("pthread_create DDC specific");
    return EXIT_FAILURE;
  }
  DDCSpecificThreadStarted = true;

  if(MakeSocket(SocketData+VPORTDUCSPECIFIC, 0) != 0)            // create and bind a socket
  {
    printf("failed to create DUC specific socket\n");
    return EXIT_FAILURE;
  }
  if(pthread_create(&DUCSpecificThread, NULL, IncomingDUCSpecific, (void*)&SocketData[VPORTDUCSPECIFIC]) != 0)
  {
    perror("pthread_create DUC specific");
    return EXIT_FAILURE;
  }
  DUCSpecificThreadStarted = true;

  if(MakeSocket(SocketData+VPORTHIGHPRIORITYTOSDR, 0) != 0)            // create and bind a socket
  {
    printf("failed to create incoming high priority socket\n");
    return EXIT_FAILURE;
  }
  if(pthread_create(&HighPriorityToSDRThread, NULL, IncomingHighPriority, (void*)&SocketData[VPORTHIGHPRIORITYTOSDR]) != 0)
  {
    perror("pthread_create High priority to SDR");
    return EXIT_FAILURE;
  }
  HighPriorityToSDRThreadStarted = true;

  if(MakeSocket(SocketData+VPORTSPKRAUDIO, 0) != 0)            // create and bind a socket
  {
    printf("failed to create speaker audio socket\n");
    return EXIT_FAILURE;
  }
  if(pthread_create(&SpkrAudioThread, NULL, IncomingSpkrAudio, (void*)&SocketData[VPORTSPKRAUDIO]) != 0)
  {
    perror("pthread_create speaker audio");
    return EXIT_FAILURE;
  }
  SpkrAudioThreadStarted = true;

  if(MakeSocket(SocketData+VPORTDUCIQ, 0) != 0)            // create and bind a socket
  {
    printf("failed to create DUC I/Q socket\n");
    return EXIT_FAILURE;
  }
  if(pthread_create(&DUCIQThread, NULL, IncomingDUCIQ, (void*)&SocketData[VPORTDUCIQ]) != 0)
  {
    perror("pthread_create DUC I/Q");
    return EXIT_FAILURE;
  }
  DUCIQThreadStarted = true;

//
// create outgoing mic data thread
// default behavior shares port/socket with incoming DUC specific (1026).
// if general packet assigns a different mic port, the mic thread will
// create and use its own socket when it handles VBITCHANGEPORT.
//
  if(pthread_create(&MicThread, NULL, OutgoingMicSamples, (void*)&SocketData[VPORTMICAUDIO]) != 0)
  {
    perror("pthread_create Mic");
    return EXIT_FAILURE;
  }
  MicThreadStarted = true;


//
// create outgoing high priority data thread
// default behavior shares port/socket with incoming DDC specific (1025).
// if general packet assigns a different outgoing high-priority port, this
// thread will create/use its own socket when VBITCHANGEPORT is processed.
//
  if(pthread_create(&HighPriorityFromSDRThread, NULL, OutgoingHighPriority, (void*)&SocketData[VPORTHIGHPRIORITYFROMSDR]) != 0)
  {
    perror("pthread_create outgoing hi priority");
    return EXIT_FAILURE;
  }
  HighPriorityFromSDRThreadStarted = true;


//
// and for now create just one outgoing DDC data thread for DDC 0
// create all the sockets though!
//
  if(MakeSocket(SocketData + VPORTDDCIQ0, 0) != 0)
  {
    printf("failed to create DDC I/Q socket 0\n");
    return EXIT_FAILURE;
  }
  if(MakeSocket(SocketData + VPORTDDCIQ1, 0) != 0)
  {
    printf("failed to create DDC I/Q socket 1\n");
    return EXIT_FAILURE;
  }
  if(MakeSocket(SocketData + VPORTDDCIQ2, 0) != 0)
  {
    printf("failed to create DDC I/Q socket 2\n");
    return EXIT_FAILURE;
  }
  if(MakeSocket(SocketData + VPORTDDCIQ3, 0) != 0)
  {
    printf("failed to create DDC I/Q socket 3\n");
    return EXIT_FAILURE;
  }
  if(MakeSocket(SocketData + VPORTDDCIQ4, 0) != 0)
  {
    printf("failed to create DDC I/Q socket 4\n");
    return EXIT_FAILURE;
  }
  if(MakeSocket(SocketData + VPORTDDCIQ5, 0) != 0)
  {
    printf("failed to create DDC I/Q socket 5\n");
    return EXIT_FAILURE;
  }
  if(MakeSocket(SocketData + VPORTDDCIQ6, 0) != 0)
  {
    printf("failed to create DDC I/Q socket 6\n");
    return EXIT_FAILURE;
  }
  if(MakeSocket(SocketData + VPORTDDCIQ7, 0) != 0)
  {
    printf("failed to create DDC I/Q socket 7\n");
    return EXIT_FAILURE;
  }
  if(MakeSocket(SocketData + VPORTDDCIQ8, 0) != 0)
  {
    printf("failed to create DDC I/Q socket 8\n");
    return EXIT_FAILURE;
  }
  if(MakeSocket(SocketData + VPORTDDCIQ9, 0) != 0)
  {
    printf("failed to create DDC I/Q socket 9\n");
    return EXIT_FAILURE;
  }
  if(pthread_create(&DDCIQThread[0], NULL, OutgoingDDCIQ, (void*)&SocketData[VPORTDDCIQ0]) != 0)
  {
    perror("pthread_create DUC I/Q");
    return EXIT_FAILURE;
  }
  DDCIQThreadStarted[0] = true;

  if(Version >= 18)
  {
//
// create outgoing wideband data thread which services bothe wideband0 and wideband1
// default behavior shares sockets with incoming threads:
// wideband0->high priority in (1027), wideband1->speaker in (1028).
// if general packet assigns different wideband ports, this thread opens
// independent sockets when handling VBITCHANGEPORT.
//
    if(pthread_create(&WidebandDataThread, NULL, OutgoingWidebandSamples, (void*)&SocketData[VPORTWIDEBAND0]) != 0)
    {
      perror("pthread_create outgoing wideband data");
      return EXIT_FAILURE;
    }
    WidebandDataThreadStarted = true;
  }





  //
  // now main processing loop. Process received Command packets arriving at port 1024
  // these are identified by the command byte (byte 4)
  // cmd=00: general packet
  // cmd=02: discovery
  // cmd=03: set IP address (not supported)
  // cmd=04: erase (not supported)
  // cmd=05: program (not supported)
  //
  while(1)
  {
    SyncSignalExitRequest();
    memset(&iovecinst, 0, sizeof(struct iovec));
    memset(&datagram, 0, sizeof(datagram));
    iovecinst.iov_base = &UDPInBuffer;                  // set buffer for incoming message number i
    iovecinst.iov_len = VDDCPACKETSIZE;
    datagram.msg_iov = &iovecinst;
    datagram.msg_iovlen = 1;
    datagram.msg_name = &addr_from;
    datagram.msg_namelen = sizeof(addr_from);
    size = recvmsg(atomic_load(&SocketData[0].Socketid), &datagram, 0);  // get one message. If it times out, gets size=-1
    if(size < 0 && errno != EAGAIN)
    {
      perror("recvfrom, port 1024");
      return EXIT_FAILURE;
    }
    SyncSignalExitRequest();
    if(atomic_load(&ExitRequested))
      break;
    if(atomic_load(&ThreadError))
      break;


//
// only process packets of length 60 bytes on this port, to exclude protocol 1 discovery for example.
// (that means we can't handle the programming packet but we don't use that anyway)
//
    CmdByte = UDPInBuffer[4];
    if(size==VDISCOVERYSIZE)  
    {
      atomic_store(&NewMessageReceived, true);
      switch(CmdByte)
      {
        //
        // general packet. Get the port numbers and establish listener threads
        //
        case 0:
          //
          // get "from" MAC address and port; this is where the data goes back to
          //
          pthread_mutex_lock(&g_reply_addr_mutex);
          memset(&reply_addr, 0, sizeof(reply_addr));
          reply_addr.sin_family = AF_INET;
          reply_addr.sin_addr.s_addr = addr_from.sin_addr.s_addr;
          reply_addr.sin_port = addr_from.sin_port;                       // (but each outgoing thread needs to set its own sin_port)
          pthread_mutex_unlock(&g_reply_addr_mutex);
          if(QueueGeneralPacketForApply(UDPInBuffer, (size_t)size))
            MaybeLogStartupEvent(&g_startup_general_rx_logged, "General packet received");
          break;

        //
        // discovery packet
        //
        case 2:
          printf("P2 Discovery packet\n");
          MaybeLogStartupEvent(&g_startup_discovery_logged, "Discovery packet received");
          if(atomic_load(&SDRActive) || IncompatibleFirmware)
            DiscoveryReply[4] = 3;                             // response 2 if not active, 3 if running
          else
            DiscoveryReply[4] = 2;                             // response 2 if not active, 3 if running

          memset(&UDPInBuffer, 0, VDISCOVERYREPLYSIZE);
          memcpy(&UDPInBuffer, DiscoveryReply, VDISCOVERYREPLYSIZE);
          sendto(atomic_load(&SocketData[0].Socketid), &UDPInBuffer, VDISCOVERYREPLYSIZE, 0, (struct sockaddr *)&addr_from, sizeof(addr_from));
          break;

        case 3:
        case 4:
        case 5:
          printf("Unsupported packet\n");
          break;

        default:
          break;

      }// end switch (packet type)
    }
//
// now do any "post packet" processing
//
    (void)ApplyQueuedGeneralPacketIfStable();
    if(!atomic_load(&SDRActive))
    {
      if(ApplyQueuedOutgoingPortRebinds() != 0)
        break;
    }
    MaybeActivateFromStartupHandshake();
  } //while(1)
  atomic_store(&ExitRequested, true);
  if(atomic_load(&ThreadError))
    printf("Thread error reported - exiting\n");
  //
  // clean exit
  //
  printf("Exiting\n");
  Shutdown();
  return EXIT_SUCCESS;
}
