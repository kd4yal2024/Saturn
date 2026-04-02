//////////////////////////////////////////////////////////////
//
// Saturn project: Artix7 FPGA + Raspberry Pi4 Compute Module
// PCI Express interface from linux on Raspberry pi
// this application uses C code to emulate HPSDR protocol 1 
//
// copyright Laurence Barker November 2021
// licenced under GNU GPL3
//
// version.h:
// print version information from FPGA registers
//
//////////////////////////////////////////////////////////////

#ifndef __version_h
#define __version_h

#include <stdbool.h>
#include <stdint.h>

#define VPRODUCT_NAME_MAX_LEN 32U
#define VFIRMWARE_NAME_MAX_LEN 64U

typedef struct
{
    uint16_t ProductId;
    uint16_t ProductVersion;
    uint16_t FirmwareVersion;
    uint8_t FirmwareId;
    uint8_t FirmwareMajorVersion;
    uint32_t DateCode;
    uint8_t ClockMask;
    bool AllClocksPresent;
    bool FallbackConfig;
    char ProductName[VPRODUCT_NAME_MAX_LEN];
    char FirmwareName[VFIRMWARE_NAME_MAX_LEN];
} TVersionInfoSnapshot;


//
// define types for product responses
//
typedef enum 
{
    eInvalidProduct,                // productid = 1
    eSaturn                         // productid=Saturn
} EProductId;

typedef enum 
{
    ePrototype1,                // productid = 1
    eProductionV1                         // productid=Saturn
} EProductVersion;

typedef enum
{
    eInvalidSWID,
    e1stProtoFirmware,
    e2ndProtofirmware,
    eFallback,
    eFullFunction
} ESoftwareID;


//
// function call to get firmware ID and version
//
unsigned int GetFirmwareVersion(ESoftwareID* ID);

//
// read all version-related FPGA register fields into a structured snapshot
//
void GetVersionInfoSnapshot(TVersionInfoSnapshot* Snapshot);


//
// function call to get firmware major version
//
unsigned int GetFirmwareMajorVersion(void);


//
// prints version information from the registers
//
void PrintVersionInfo(void);

//
// Check for a fallback configuration
// returns true if FPGA is a fallback load
//
bool IsFallbackConfig(void);

//
// check that the board is a Saturn one
// return true if SATURN
//
bool IsSaturnPCB(void);

//
// get PCB version number
// 1: 1st prototype; 2: production V1; 3: production V2
// (used to select the correct device drivers etc)
//
uint16_t GetPCBVersionNumber(void);

#endif
