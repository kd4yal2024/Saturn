
/////////////////////////////////////////////////////////////
//
// Saturn project: Artix7 FPGA + Raspberry Pi4 Compute Module
// PCI Express interface from linux on Raspberry pi
// this application uses C code to emulate HPSDR protocol 2 
//
// copyright Laurence Barker November 2021
// licenced under GNU GPL3
//
// serialport.c:
//
// handle simple CAT access to serial port
// CAT messages are forwarded to CAT handler
// port is opened and read performed by creating a thread
//
//////////////////////////////////////////////////////////////

#include <termios.h>
#include <stdlib.h>
#include <fcntl.h>
#include <sys/ioctl.h>
#include <unistd.h>
#include <string.h>
#include <stdio.h>
#include <pthread.h>
#include <syscall.h>


#include "serialport.h"
#include "cathandler.h"


//
// names of devices that thread has been opened for
//
char* DeviceNames[] =
{
  "G2V2 Panel",
  "G2V1 Panel Adapter",
  "AriesATU",
  "Ganymede PA Control"
};



//
// open and set up a serial port for read/write access
//
int OpenSerialPort(char* DeviceName, unsigned int Baud)
{
    int Device;
    struct termios Ser;

    Device = open(DeviceName, O_RDWR);
    if(Device == -1)
    {
        printf("serial open failed on device %s\n", DeviceName);
    }
    else
    {
        memset(&Ser, 0, sizeof(Ser));
        Ser.c_iflag = IGNBRK | IGNPAR;
        Ser.c_cflag = CS8 | CREAD | HUPCL | CLOCAL;
        cfsetospeed(&Ser, Baud);
        cfsetispeed(&Ser, Baud);
        Ser.c_cc[VTIME] = 5;                // 0.5s timeout on read
        Ser.c_cc[VMIN] = 0;                 // read can return wioth no characters

        if (tcsetattr(Device, TCSANOW, &Ser) < 0) 
        {
            perror("tcsetattr()");
            close(Device);
            return -1;
        }
        tcflush(Device, TCIFLUSH);
    }
    return Device;
}



//
// send a string to the serial port
//
void SendStringToSerial(int Device, const char* Message)
{
    int Length;                                     // message length in charactera

    Length = strlen(Message);
    { ssize_t _r = write(Device, Message, Length); (void)_r; }
}




#define VESERINSIZE 120                 // large enough to hold a whole CAT message
//
// serial read thread
// the paramter passed is a pointer to a struct with the required settings
//
void* CATSerial(void *arg)
{
    char SerialInputBuffer[VESERINSIZE];
    char CATMessageBuffer[VESERINSIZE];
    int DeviceHandle;
    int ReadCnt;
    int Cntr;
    int CATWritePtr = 0;
    char ch;                                    // individual read character
    bool StartsWithZZZS;
    bool StartsWithZZZP;

    TSerialThreadData *DeviceData;

    DeviceData = (TSerialThreadData *) arg;
    DeviceHandle = OpenSerialPort(DeviceData -> PathName, DeviceData -> Baud);
    atomic_store(&DeviceData->DeviceHandle, DeviceHandle);
    if(DeviceHandle != -1)
    {
        printf("Setting up CAT Serial read handler thread for device %s, pid=%ld\n", DeviceNames[(int)DeviceData->Device], syscall(SYS_gettid));
        atomic_store(&DeviceData->IsOpen, true);
        if(atomic_load(&DeviceData->DeviceActive))
        {
            sleep(1);                                   // allow serial to start (particularly for USB)

            if(atomic_load(&DeviceData->RequestID) && atomic_load(&DeviceData->DeviceActive))
                { ssize_t _r = write(DeviceHandle, "ZZZS;", 5); (void)_r; }
        }

    //
    // now loop waiting for characters, then form them into CAT messages terminated by semicolon
    // read() will return after timoeut with no characters, so do check the count!
    //
        while(atomic_load(&DeviceData->DeviceActive))
        {
            ReadCnt = read(DeviceHandle, &SerialInputBuffer, VESERINSIZE);
            if (ReadCnt > 0)
            {
    //
    // we have input data available, so read it one char at a time and write to buffer
    // if we find a terminating semicolon, process the command
    // if we get a control character, abandon the line so far and start again
    //
                for(Cntr=0; Cntr < ReadCnt; Cntr++)
                {
                    ch=SerialInputBuffer[Cntr];
                    if(((unsigned char)ch < 0x20) && (ch != ';'))
                    {
                        CATWritePtr = 0;                                // control chars abandon current message
                        continue;
                    }
                    if(CATWritePtr >= (VESERINSIZE - 2))
                        CATWritePtr = 0;                                // prevent overwrite; restart buffer
                    CATMessageBuffer[CATWritePtr++] = ch;
                    if (ch == ';')                                      // found end of a complete CAT message
                    {
                        CATMessageBuffer[CATWritePtr] = 0;              // terminate the string
                        StartsWithZZZS = (strncmp(CATMessageBuffer, "ZZZS", 4) == 0);
                        StartsWithZZZP = (strncmp(CATMessageBuffer, "ZZZP", 4) == 0);
                        if((DeviceData -> Device == eG2V2Panel) ||(DeviceData -> Device == eG2V1PanelAdapter))
                        {
                        //
                        // if ZZZS or ZZZP, send to local handler; else send to SDR client app
                        //
                            if(StartsWithZZZS || StartsWithZZZP)
                                ParseCATCmd(CATMessageBuffer, DeviceHandle);              // if ZZZS, process locally; else send to TCPIP CAT port
                            else
                                SendCATMessage(CATMessageBuffer);           // send unprocessed to SDR client app via TCP/IP
                        }
                        else
                        {
                        //
                        // for non front panel devices, process CAT commands locally
                        //
                            ParseCATCmd(CATMessageBuffer, DeviceHandle);
                        }
                        CATWritePtr = 0;                                // reset for next CAT message
                    }
                }
            }
        }
        printf("Closing CAT Serial read handler thread for device %s\n", DeviceNames[(int)DeviceData->Device]);
        close(DeviceHandle);
        atomic_store(&DeviceData->IsOpen, false);
        atomic_store(&DeviceData->DeviceHandle, -1);
    }
    return NULL;
}
