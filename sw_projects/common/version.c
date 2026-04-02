//////////////////////////////////////////////////////////////
//
// Saturn project: Artix7 FPGA + Raspberry Pi4 Compute Module
// PCI Express interface from linux on Raspberry pi
// this application uses C code to emulate HPSDR protocol 1 
//
// copyright Laurence Barker November 2021
// licenced under GNU GPL3
//
// version.c:
// print version information from FPGA registers

//
//////////////////////////////////////////////////////////////

#define _DEFAULT_SOURCE
#define _XOPEN_SOURCE 500
#include <assert.h>
#include <fcntl.h>
#include <getopt.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <errno.h>

#include <sys/types.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

#include "../common/saturntypes.h"
#include "../common/version.h"
#include "../common/saturnregisters.h"
#include "../common/hwaccess.h"



#define VADDRUSERVERSIONREG 0x4004              // user defined version register
#define VADDRSWVERSIONREG 0XC000                // user defined s/w version register
#define VADDRPRODVERSIONREG 0XC004              // user defined product version register


//
// the identification scheme leaves open the possibility of other products with similar s/w & FPGA architecture
//
#define VMAXPRODUCTID 1							// product ID index limit
#define VMAXSWID 4								// software ID index limit

const char* ProductIDStrings[] =
{
	"invalid product ID",
	"Saturn"
};

//
// these are relevant to Saturn only!
//
const char* SWIDStrings[] =
{
	"invalid software ID",
	"Saturn prototype, board test code",
	"Saturn prototype, with DSP",
	"Fallback Golden image",
	"Saturn, full function"
};

const char* ClockStrings[] =
{
	"122.88MHz main clock",
	"10MHz Reference clock",
	"EMC config clock",
	"122.88MHz main clock"
};

#define SATURNPRODUCTID 1					// Saturn, any version
#define SATURNGOLDENCONFIGID 3				// "golden" configuration id

static void FillVersionInfoSnapshot(TVersionInfoSnapshot* Snapshot)
{
	uint32_t SoftwareInformation;
	uint32_t ProductInformation;
	uint32_t DateCode;
	uint32_t SWID;
	uint32_t ProdID;

	if (Snapshot == NULL)
		return;

	memset(Snapshot, 0, sizeof(*Snapshot));

	SoftwareInformation = RegisterRead(VADDRSWVERSIONREG);
	ProductInformation = RegisterRead(VADDRPRODVERSIONREG);
	DateCode = RegisterRead(VADDRUSERVERSIONREG);

	Snapshot->DateCode = DateCode;
	Snapshot->ClockMask = (uint8_t)(SoftwareInformation & 0x0FU);
	Snapshot->FirmwareVersion = (uint16_t)((SoftwareInformation >> 4) & 0xFFFFU);
	SWID = (SoftwareInformation >> 20) & 0x1FU;
	Snapshot->FirmwareId = (uint8_t)SWID;
	Snapshot->FirmwareMajorVersion = (uint8_t)(SoftwareInformation >> 25);
	Snapshot->ProductVersion = (uint16_t)(ProductInformation & 0xFFFFU);
	ProdID = ProductInformation >> 16;
	Snapshot->ProductId = (uint16_t)ProdID;
	Snapshot->AllClocksPresent = (Snapshot->ClockMask == 0x0FU);
	Snapshot->FallbackConfig = ((ProdID == SATURNPRODUCTID) && (SWID == SATURNGOLDENCONFIGID));

	if (ProdID > VMAXPRODUCTID)
		snprintf(Snapshot->ProductName, sizeof(Snapshot->ProductName), "%s", ProductIDStrings[0]);
	else
		snprintf(Snapshot->ProductName, sizeof(Snapshot->ProductName), "%s", ProductIDStrings[ProdID]);

	if (SWID > VMAXSWID)
		snprintf(Snapshot->FirmwareName, sizeof(Snapshot->FirmwareName), "%s", SWIDStrings[0]);
	else
		snprintf(Snapshot->FirmwareName, sizeof(Snapshot->FirmwareName), "%s", SWIDStrings[SWID]);
}

void GetVersionInfoSnapshot(TVersionInfoSnapshot* Snapshot)
{
	FillVersionInfoSnapshot(Snapshot);
}


//
// Check for a fallback configuration
// returns true if FPGA is a fallback load
//
bool IsFallbackConfig(void)
{
	TVersionInfoSnapshot Snapshot;
	FillVersionInfoSnapshot(&Snapshot);
	return Snapshot.FallbackConfig;
}

//
// prints version information from the registers
//
void PrintVersionInfo(void)
{
	TVersionInfoSnapshot Snapshot;
	uint32_t Cntr;

	FillVersionInfoSnapshot(&Snapshot);
	printf("FPGA BIT file data code = %08x\n", Snapshot.DateCode);
	printf(" Product: %s; Version = %d\n", Snapshot.ProductName, Snapshot.ProductVersion);
	printf(" FPGA Firmware loaded: %s; FW Version = %d, major version = %d\n",
	       Snapshot.FirmwareName, Snapshot.FirmwareVersion, Snapshot.FirmwareMajorVersion);

	if (Snapshot.AllClocksPresent)
		printf("All clocks present\n");
	else
	{
		for (Cntr = 0; Cntr < 4; Cntr++)
		{
			if (Snapshot.ClockMask & (1U << Cntr))
				printf("%s present\n", ClockStrings[Cntr]);
			else
				printf("%s not present\n", ClockStrings[Cntr]);
		}
	}
}



//
// function call to get firmware ID and version
//
unsigned int GetFirmwareVersion(ESoftwareID* ID)
{
	TVersionInfoSnapshot Snapshot;
	FillVersionInfoSnapshot(&Snapshot);
	*ID = (ESoftwareID)Snapshot.FirmwareId;
	return Snapshot.FirmwareVersion;
}



//
// function call to get firmware major version
//
unsigned int GetFirmwareMajorVersion(void)
{
	TVersionInfoSnapshot Snapshot;
	FillVersionInfoSnapshot(&Snapshot);
	return Snapshot.FirmwareMajorVersion;
}


//
// check that the board is a Saturn one
// return true if SATURN
//
bool IsSaturnPCB(void)
{
	TVersionInfoSnapshot Snapshot;
	FillVersionInfoSnapshot(&Snapshot);
	return (Snapshot.ProductId == SATURNPRODUCTID);

}

//
// get PCB version number
// 1: 1st prototype; 2: production V1; 3: production V2
// (used to select the correct device drivers etc)
//
uint16_t GetPCBVersionNumber(void)
{
	TVersionInfoSnapshot Snapshot;
	FillVersionInfoSnapshot(&Snapshot);
	return Snapshot.ProductVersion;
}
