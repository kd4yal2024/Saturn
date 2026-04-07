/////////////////////////////////////////////////////////////
//
// Saturn project: Artix7 FPGA + Raspberry Pi4 Compute Module
// PCI Express interface from linux on Raspberry pi
// this application uses C code to emulate HPSDR protocol 2 
//
// copyright Laurence Barker November 2021
// licenced under GNU GPL3
//
// g2panel.c:
//
// interface G2V2 front panel using asynchronous serial
// also interfaces a G2V1 panel if it has an RP2040 serial adapter
//
//////////////////////////////////////////////////////////////

#include "g2v2panel.h"
#include "threaddata.h"
#include <stdbool.h>
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
#include "serialport.h"

#include "../common/saturnregisters.h"
#include "../common/saturndrivers.h"
#include "../common/hwaccess.h"
#include "../common/debugaids.h"
#include "cathandler.h"

#include <linux/i2c-dev.h>
#include "i2cdriver.h"
#include "andromedacatmessages.h"
#include "AriesATU.h"

#define ID_G2V2_PANEL "9f2b6c5a-4d7e-4c3b-9a21-3f8d0e6b12c4"


bool G2V2PanelControlled = false;
atomic_bool G2V2PanelActive = false;               // true while panel active and threads should run
atomic_bool G2V2CATDetected = false;               // true if panel ID message has been sent
atomic_bool G2V2Detected = false;                  // true if G2V2 panel detected from ZZZS response
atomic_bool G2V1AdapterDetected = false;           // true if G2V1 adapter detected from ZZZS response
atomic_bool GZZZIReceived = false;                 // true if a ZZZI message received (so halt polling)

extern int i2c_fd;                                  // file reference
char* gpio_dev = NULL;
pthread_t G2V2PanelTickThread;                      // thread with periodic tick
pthread_t G2V2PanelSerialThread;                    // thread wfor serial read from panel
pthread_t G2V1AdapterSerialThread;                // thread wfor serial read from panel
static bool G2V2PanelTickThreadStarted = false;
static bool G2V2PanelSerialThreadStarted = false;
atomic_uchar G2V2PanelSWID;
atomic_uchar G2V2PanelHWVersion;
atomic_uchar G2V2PanelProductID;
atomic_bool G2ToneState = false;                   // true if 2 tone test in progress
atomic_bool GVFOBSelected = false;                 // true if VFO B selected
atomic_uint_fast32_t GCombinedVFOState = 0;        // reported VFO state bits
TSerialThreadData G2V2Data;                         // data for G2V2 read thread
atomic_bool ATURedLED = false;
atomic_bool ATUGreenLED = false;                   // LED states


#define VKEEPALIVECOUNT 150                         // 15s period between keepalive requests (based on 100ms tick)


#define G2ARDUINOPATH "/dev/ttyAMA1"                // G2 panel, Raspberry pi serial port
#define G2V1ADAPTERPATH "/dev/ttyACM0"              // G2V1 adapter, USB serial




static SaturnSerialPort SaturnSerialPortsList[] = 
{
  {"/dev/serial/by-id/g2-front-115200", B115200},
  {"/dev/serial/by-id/g2-front-9600", B9600},
  {NULL, 0}
};



 
//
// function to check if panel is present. This is called before panel initialise.
//
// change to open the thread, which opens file and sends ZZZS;
// then wait for response to come back via CAT handler. Making a proper "closed loop" identification. 
//
bool CheckG2V2PanelPresent(void)
{
    bool Result = false;
    char* Name;
    int Cntr;
    int Found = 0;

    printf("checking for G2V2 or G2V1 adapter\n");
    atomic_store(&G2V2Detected, false);
    atomic_store(&G2V1AdapterDetected, false);

//
// try to find a device that exists
//
    Cntr=0;
    while(1)
    {
        Name = SaturnSerialPortsList[Cntr++].port;

        if(Name == NULL)                                // if last entry
            break;
        else
        {
            if(access(Name, W_OK)==0)
            {
                printf("table access identifies device %s\n", Name);
                Found = Cntr;
                break;
            }
        }
    }

    if(Found != 0)                      // we found a serial port
    {
        Found--;                           // get back to 0 base
    //
    // launch handler for G2V2
    //
        strcpy(G2V2Data.PathName, SaturnSerialPortsList[Found].port);
        G2V2Data.Baud = SaturnSerialPortsList[Found].baud;
        atomic_store(&G2V2Data.DeviceHandle, -1);
        atomic_store(&G2V2Data.IsOpen, false);
        atomic_store(&G2V2Data.DeviceActive, true);
        atomic_store(&G2V2Data.RequestID, true);
        G2V2Data.Device = eG2V2Panel;
        G2V2PanelSerialThreadStarted = false;

        if(pthread_create(&G2V2PanelSerialThread, NULL, CATSerial, (void *)&G2V2Data) != 0)
        {
            perror("pthread_create G2V2 serial thread");
            atomic_store(&G2V2Data.DeviceActive, false);
        }
        else
            G2V2PanelSerialThreadStarted = true;
        for(int WaitCntr = 0; WaitCntr < 20; WaitCntr++)
        {
            if(atomic_load(&G2V1AdapterDetected) || atomic_load(&G2V2Detected))
                break;
            usleep(100000);
        }
    //
    // now see if anything came back from CAT handler
    // disable devices not to be used - this will cause them to close their files
    //
        if(atomic_load(&G2V1AdapterDetected) || atomic_load(&G2V2Detected))
        {
            Result = true;
        }
        else
        {
            atomic_store(&G2V2Data.DeviceActive, false);
            if(G2V2PanelSerialThreadStarted)
            {
                pthread_join(G2V2PanelSerialThread, NULL);
                G2V2PanelSerialThreadStarted = false;
            }
        }
    }

    return Result;
}


#define VNUMG2V2INDICATORS 9



//
// periodic timestep
//
void* G2V2PanelTick(__attribute__((unused)) void *arg)
{
    uint32_t NewLEDStates = 0;
    uint8_t CATPollCntr = 0;                        // owned by tick thread
    uint16_t GLEDState = 0;                         // owned by tick thread

    while(atomic_load(&G2V2PanelActive))
    {
        if(atomic_load(&CATPortAssigned))      // see if CAT has become available for the 1st time
        {
            if(atomic_load(&G2V2CATDetected) == false)
            {
                atomic_store(&G2V2CATDetected, true);
                MakeProductVersionCAT(atomic_load(&G2V2PanelProductID), atomic_load(&G2V2PanelHWVersion), atomic_load(&G2V2PanelSWID));
                MakeCATMessageString(DESTTCPCATPORT, eZZGA, ID_G2V2_PANEL);
            }
        }
        else
            atomic_store(&G2V2CATDetected, false);
//
// poll CAT, if we haven't been sent an indicator message
//
        if(atomic_load(&GZZZIReceived) == false)
            switch(CATPollCntr++)
            {
                case 0:
                    MakeCATMessageNoParam(DESTTCPCATPORT, eZZXV);
                    break;

                case 1:
                    MakeCATMessageNoParam(DESTTCPCATPORT, eZZUT);
                    break;

                case 2:
                    MakeCATMessageNoParam(DESTTCPCATPORT, eZZYR);
                    break;

                default:
                    CATPollCntr = 0;
                    break;
            }
//
// Set LEDs from values reported by CAT messages
// store into NewLEDStates; then set to I2C create ZZZI if different from what we had before
// ATU tune LEDs are internal to P2app, not Thetis
//
        if(atomic_load(&GZZZIReceived) == false)
        {
            uint32_t CombinedVFOState = atomic_load(&GCombinedVFOState);
            bool ToneState = atomic_load(&G2ToneState);
            bool VFOBSelected = atomic_load(&GVFOBSelected);
            bool LocalATURedLED = atomic_load(&ATURedLED);
            bool LocalATUGreenLED = atomic_load(&ATUGreenLED);

            NewLEDStates = 0;
            if((CombinedVFOState & (1<<6)) != 0)
                NewLEDStates |= 1;                          // MOX bit
            if((CombinedVFOState & (1<<7)) != 0)
                NewLEDStates |= (1 << 1);                   // TUNE bit
            if(ToneState)
                NewLEDStates |= (1 << 2);                   // 2 tone bit
            if(LocalATURedLED)
                NewLEDStates |= (1 << 3);                   // red ATU bit
            if(LocalATUGreenLED)
                NewLEDStates |= (1 << 4);                   // green ATU bit
            if((CombinedVFOState & (1<<8)) != 0)
                NewLEDStates |= (1 << 6);                   // XIT bit
            if((CombinedVFOState & (1<<0)) != 0)
                NewLEDStates |= (1 << 5);                   // RIT bit
            if(!VFOBSelected)
                NewLEDStates |= (1 << 7);                   // led lit if VFO A selected

            if((((CombinedVFOState & (1<<2)) != 0) && VFOBSelected) ||
            (((CombinedVFOState & (1<<1)) != 0) && !VFOBSelected))
                NewLEDStates |= (1 << 8);                   // VFO Lock bit

//
// now loop through to find differences
// do bitwise compares; if differences found, send a ZZZI message
// only send to G2V2, not to G2V1 adapter because it has no LEDs
//
            int Cntr;
            int Mask = 1;
            int NewState;
            int Param;

            for(Cntr=0; Cntr < VNUMG2V2INDICATORS; Cntr++)
            {
                if((NewLEDStates & Mask) != (GLEDState & Mask))
                {
                    NewState = (NewLEDStates & Mask) >> Cntr;
                    Param = ((Cntr +1)* 10) + NewState;
                    {
                        int DeviceHandle = atomic_load(&G2V2Data.DeviceHandle);
                        if(atomic_load(&G2V2Data.IsOpen) && (DeviceHandle != -1))
                            MakeCATMessageNumeric(DeviceHandle, eZZZI, Param);
                    }

                }
                Mask = Mask << 1;                               // bitmask for next bit
            }
            GLEDState = NewLEDStates;
        }

        usleep(100000);                                                  // 100ms period

    }
    return NULL;
}



//
// function to initialise a connection to the G2 V2 front panel; call if selected as a command line option
// this is called *after* the G2V2 panel has been discovered.
// create threads for tick
//
void InitialiseG2V2PanelHandler(void)
{
    G2V2PanelControlled = true;
    printf("Initialising G2V2 panel handler\n");
    atomic_store(&G2V2PanelActive, true);
    G2V2PanelTickThreadStarted = false;

    if(pthread_create(&G2V2PanelTickThread, NULL, G2V2PanelTick, NULL) != 0)
        perror("pthread_create G2 panel tick");
    else
        G2V2PanelTickThreadStarted = true;
}


//
// function to shutdown a connection to the G2 front panel; call if selected as a command line option
// serial files closed by setting DeviceActive to false; the thread then closes the file. 
//
void ShutdownG2V2PanelHandler(void)
{
    atomic_store(&G2V2PanelActive, false);
    if(G2V2PanelTickThreadStarted)
    {
        pthread_join(G2V2PanelTickThread, NULL);
        G2V2PanelTickThreadStarted = false;
    }
    atomic_store(&G2V2Data.DeviceActive, false);
    if(G2V2PanelSerialThreadStarted)
    {
        pthread_join(G2V2PanelSerialThread, NULL);
        G2V2PanelSerialThreadStarted = false;
    }
}


//
// receive ZZUT state
//
void SetG2V2ZZUTState(bool NewState)
{
    atomic_store(&G2ToneState, NewState);
}


//
// receive ZZYR state
//
void SetG2V2ZZYRState(bool NewState)
{
    atomic_store(&GVFOBSelected, NewState);
}



//
// receive ZZXV state
//
void SetG2V2ZZXVState(uint32_t NewState)
{
    atomic_store(&GCombinedVFOState, NewState);
}



//
// receive ZZZS state
// this has already been decoded by the CAT handler
//
void SetG2V2ZZZSState(uint8_t ProductID, uint8_t HWVersion, uint8_t SWID)
{
    if(ProductID == 4)
    {
        printf("found G2V1 adapter, product ID=%d", ProductID);
        atomic_store(&G2V1AdapterDetected, true);
    }
    else if(ProductID == 5)
    {
        printf("found G2V2 panel, product ID=%d", ProductID);
        atomic_store(&G2V2Detected, true);
    }
    printf("; H/W verson = %d", HWVersion);
    printf("; S/W verson = %d\n", SWID);
    atomic_store(&G2V2PanelProductID, ProductID);
    atomic_store(&G2V2PanelHWVersion, HWVersion);
    atomic_store(&G2V2PanelSWID, SWID);
}



//
// receive ZZZI state
// set that it has been seen, and make an outgoing message for the panel
//
void SetG2V2ZZZIState(uint32_t Param)
{
    int DeviceHandle = atomic_load(&G2V2Data.DeviceHandle);

    atomic_store(&GZZZIReceived, true);
    if((DeviceHandle != -1) && atomic_load(&G2V2Data.IsOpen))
        MakeCATMessageNumeric(DeviceHandle, eZZZI, Param);

}

#define VATUBUTTONSCANCODE 4
//
// receive a ZZZP message from front panel
// for now, send straight to client SDR app via TCP/IP
//
void HandleG2V2ZZZPMessage(uint32_t Param)
{
    uint8_t ScanCode;
    uint8_t State;

    ScanCode = Param / 10;
    State = Param % 10;
    if(ScanCode == 4)
        HandleATUButtonPress(State);
    else
        MakeCATMessageNumeric(DESTTCPCATPORT, eZZZP, Param);
}



//
// see if serial device belongs to a front panel open serial port
// return true if this handle belongs to a front panel
//
bool IsFrontPanelSerial(int32_t Handle)
{
    bool Result = false;
    if((Handle == atomic_load(&G2V2Data.DeviceHandle)) && atomic_load(&G2V2Data.IsOpen))
        Result = true;
    return Result;
}


//
// set ATU LED states
// bool true if lit
//
void SetATULEDs(bool GreenLED, bool RedLED)
{
    atomic_store(&ATURedLED, RedLED);
    atomic_store(&ATUGreenLED, GreenLED);
}
