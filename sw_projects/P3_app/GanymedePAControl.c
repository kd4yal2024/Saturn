/////////////////////////////////////////////////////////////
//
// Saturn project: Artix7 FPGA + Raspberry Pi4 Compute Module
// PCI Express interface from linux on Raspberry pi
// this application uses C code to emulate HPSDR protocol 2
//
// copyright Laurence Barker November 2021
// licenced under GNU GPL3
//
// GanymedePAControl.c:
//
// interface the Ganymede PA controller
//
//////////////////////////////////////////////////////////////

#include "threaddata.h"
#include <stdint.h>
#include "../common/saturntypes.h"
#include <errno.h>
#include <stdlib.h>
#include <stddef.h>
#include <unistd.h>
#include <stdio.h>
#include <string.h>
#include <fcntl.h>
#include <sys/time.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <pthread.h>
#include <syscall.h>
#include <stdatomic.h>

#include "../common/saturnregisters.h"
#include "../common/saturndrivers.h"
#include "../common/hwaccess.h"
#include "../common/debugaids.h"
#include "cathandler.h"
#include "serialport.h"
#include "GanymedePAControl.h"
#include "../common/version.h"


atomic_bool GanymedeActive = false;                  // true if Ganymede is operating
static atomic_bool GanymedeDetected = false;         // true if Ganymede detected from CAT message
static atomic_bool GanymedeCATDetected = false;      // true if Ganymede ZZZS ID message has been sent
ESoftwareID FirmwareID;

static atomic_uchar GanymedeSWID = 0;
static atomic_uchar GanymedeHWVersion = 0;
static atomic_uchar GanymedeProductID = 0;

static atomic_uint_fast32_t MostRecentAmplifierState = 0;   // reported amplifier state


TSerialThreadData GanymedeData;                       // data for Ganymede read thread
pthread_t GanymedeSerialThread;                       // thread for serial read from Ganymede
pthread_t GanymedeTickThread;                         // thread with periodic tick
static bool GanymedeSerialThreadStarted = false;
static bool GanymedeTickThreadStarted = false;


#define GANYMEDEPATH "/dev/serial/by-path/g2-ganymede-9600"           // ganymede controller (note needs udev rule to map name)

#define P2APPVERSIONID 6
#define G2FIRMWAREVERSIONID 7

#define ID_GANYMEDE "c6e1f9a4-53b2-47d8-8c0e-2a7b5d14f963"


//
// helper to encode ZZZS numeric CAT payload
//
static uint32_t MakeVersionParam(uint8_t ProductID, uint8_t HWVersion, uint8_t SWVersion)
{
    return ((uint32_t)ProductID * 100000U) + ((uint32_t)HWVersion * 1000U) + (uint32_t)SWVersion;
}


//
// Ganymede periodic timestep
// this runs as a thread, created at startup if Ganymede is detected.
//
static void* GanymedeTick(__attribute__((unused)) void *arg)
{
    printf("opened Ganymede periodic tick thread, pid=%ld\n", syscall(SYS_gettid));
    while(atomic_load(&GanymedeActive) && !atomic_load(&ExitRequested))
    {
        if(atomic_load(&CATPortAssigned))      // see if CAT has become available for the 1st time
        {
            if(!atomic_load(&GanymedeCATDetected))
            {
                atomic_store(&GanymedeCATDetected, true);
                MakeCATMessageNumeric(DESTTCPCATPORT, eZZZS,
                    (long)MakeVersionParam(atomic_load(&GanymedeProductID),
                                           atomic_load(&GanymedeHWVersion),
                                           atomic_load(&GanymedeSWID)));
                MakeCATMessageString(DESTTCPCATPORT, eZZGA, ID_GANYMEDE);
                if(atomic_load(&MostRecentAmplifierState) != 0U)
                {
                    MakeCATMessageNumeric(DESTTCPCATPORT, eZZZA,
                        (long)atomic_load(&MostRecentAmplifierState));        // forward message to TCP/IP port if amp is tripped
                }
            }
        }
        else
            atomic_store(&GanymedeCATDetected, false);

        usleep(20000);                    // 20ms period
    }
    printf("Closing Ganymede tick thread\n");
    return NULL;
}


//
// function to initialise a connection to the PA controller; call at startup if selected as a command line option
// create serial handler, and ask it to send a ZZZS. Then wait to see if a response provided.
// if a response from an Ganymede is received, set up periodic tick handler.
//
void InitialiseGanymedeHandler(void)
{
    int DeviceHandle;

    printf("checking for Ganymede PA controller\n");
    atomic_store(&GanymedeDetected, false);
    atomic_store(&GanymedeCATDetected, false);
    atomic_store(&GanymedeActive, false);
    atomic_store(&MostRecentAmplifierState, 0U);

    //
    // launch serial handler for Ganymede
    //
    strcpy(GanymedeData.PathName, GANYMEDEPATH);
    atomic_store(&GanymedeData.DeviceHandle, -1);
    atomic_store(&GanymedeData.IsOpen, false);
    atomic_store(&GanymedeData.DeviceActive, true);
    atomic_store(&GanymedeData.RequestID, true);
    GanymedeData.Device = eGanymedePAController;
    GanymedeData.Baud = B9600;
    GanymedeSerialThreadStarted = false;
    GanymedeTickThreadStarted = false;

    if(pthread_create(&GanymedeSerialThread, NULL, CATSerial, (void *)&GanymedeData) != 0)
    {
        perror("pthread_create Ganymede PA Controller thread");
        atomic_store(&GanymedeData.DeviceActive, false);
    }
    else
        GanymedeSerialThreadStarted = true;

    for(int WaitCntr = 0; WaitCntr < 20; WaitCntr++)
    {
        if(atomic_load(&GanymedeDetected))
            break;
        usleep(100000);
    }

    //
    // now see if anything came back from CAT handler
    // disable devices if not used - this will cause it to close the file
    // if detected, create periodic tick thread
    // and send CAT commands for p2app, firmware versions
    //
    if(atomic_load(&GanymedeDetected))
    {
        printf("Ganymede PA Controller selected and Active\n");
        atomic_store(&GanymedeActive, true);
        if(pthread_create(&GanymedeTickThread, NULL, GanymedeTick, NULL) != 0)
        {
            perror("pthread_create Ganymede tick");
            atomic_store(&GanymedeActive, false);
        }
        else
            GanymedeTickThreadStarted = true;

        DeviceHandle = atomic_load(&GanymedeData.DeviceHandle);
        if((DeviceHandle != -1) && atomic_load(&GanymedeData.IsOpen))
        {
            MakeCATMessageNumeric(DeviceHandle, eZZZS,
                (long)MakeVersionParam(P2APPVERSIONID, 1U, (uint8_t)GetP2appVersion()));
            MakeCATMessageNumeric(DeviceHandle, eZZZS,
                (long)MakeVersionParam(G2FIRMWAREVERSIONID, (uint8_t)GetPCBVersionNumber(),
                                       (uint8_t)GetFirmwareVersion(&FirmwareID)));
        }
    }
    else
    {
        atomic_store(&GanymedeData.DeviceActive, false);
        if(GanymedeSerialThreadStarted)
        {
            pthread_join(GanymedeSerialThread, NULL);
            GanymedeSerialThreadStarted = false;
        }
    }
}


//
// function to shutdown a connection to the PA Controller; call if selected as a command line option
//
void ShutdownGanymedeHandler(void)
{
    atomic_store(&GanymedeActive, false);          // shut down tick thread
    if(GanymedeTickThreadStarted)
    {
        pthread_join(GanymedeTickThread, NULL);
        GanymedeTickThreadStarted = false;
    }

    atomic_store(&GanymedeData.DeviceActive, false);   // shut down serial thread
    if(GanymedeSerialThreadStarted)
    {
        pthread_join(GanymedeSerialThread, NULL);
        GanymedeSerialThreadStarted = false;
    }
}


//
// receive ZZZS state
// this has already been decoded by the CAT handler
// store the ID values, so we can send out a message to TCP/IP when it connects
//
void SetGanymedeZZZSState(uint8_t ProductID, uint8_t HWVersion, uint8_t SWID)
{
    if(ProductID == 3U)
    {
        printf("found Ganymede PA Controller, product ID=%d", ProductID);
        printf("; H/W verson = %d", HWVersion);
        printf("; S/W verson = %d\n", SWID);
        atomic_store(&GanymedeDetected, true);
        atomic_store(&GanymedeProductID, ProductID);
        atomic_store(&GanymedeHWVersion, HWVersion);
        atomic_store(&GanymedeSWID, SWID);
    }
}


//
// see if serial device belongs to a Ganymede open serial port
// return true if this handle belongs to Ganymede PA Controller
//
bool IsGanymedeSerial(int Handle)
{
    bool Result = false;
    if((Handle == atomic_load(&GanymedeData.DeviceHandle)) && atomic_load(&GanymedeData.IsOpen))
        Result = true;
    return Result;
}


//
// receive a ZZZA message from Ganymede
// SourceDevice identifies where the message came from
//
void HandleGanymedeZZZAMessage(uint32_t Param, int SourceDevice, bool IsRequest)
{
    int DeviceHandle = atomic_load(&GanymedeData.DeviceHandle);

    if(SourceDevice != DESTTCPCATPORT)                  // source was Ganymede itself
    {
        atomic_store(&MostRecentAmplifierState, Param);
        MakeCATMessageNumeric(DESTTCPCATPORT, eZZZA, (long)Param);        // forward message to TCP/IP port
    }
    else if((!IsRequest) && (DeviceHandle != -1) && atomic_load(&GanymedeData.IsOpen))
    {
        MakeCATMessageNumeric(DeviceHandle, eZZZA, (long)Param);        // forward message to Ganymede
    }
    else if(IsRequest && (DeviceHandle != -1) && atomic_load(&GanymedeData.IsOpen))
    {
        MakeCATMessageNoParam(DeviceHandle, eZZZA);        // forward message to Ganymede
    }
}
