/////////////////////////////////////////////////////////////
//
// Saturn project: Artix7 FPGA + Raspberry Pi4 Compute Module
// PCI Express interface from linux on Raspberry pi
// this application uses C code to emulate HPSDR protocol 2 
//
// copyright Laurence Barker November 2021
// licenced under GNU GPL3
//
// frontpanelhandler.h:
//
// handle interface to front panel controls
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
#include <linux/i2c-dev.h>

#include "../common/saturnregisters.h"
#include "../common/saturndrivers.h"
#include "../common/hwaccess.h"
#include "../common/debugaids.h"
#include "frontpanelhandler.h"
#include "g2panel.h"
#include "g2v2panel.h"
#include "i2cdriver.h"

bool FoundG2Panel = false;
bool FoundG2V2Panel = false;

static const char* GetFrontPanelModeOverride(void)
{
    const char* Mode = getenv("SATURN_FRONT_PANEL_MODE");
    if((Mode == NULL) || (Mode[0] == '\0'))
        return "auto";
    return Mode;
}

static void InitialiseFrontPanelAuto(void)
{
    if(CheckG2V2PanelPresent())
    {
        printf("Found serial device interfaced panel\n");
        FoundG2V2Panel = true;
        InitialiseG2V2PanelHandler();
    }
    else if(CheckG2PanelPresent())
    {
        printf("Found G2 front panel\n");
        FoundG2Panel = true;
        InitialiseG2PanelHandler();
    }
    else
        printf("No front panel found\n");
}


//
// function to initialise a connection to the front panel; call if selected as a command line option
// establish which if any front panel is attached, and get it set up.
//
void InitialiseFrontPanelHandler(void)
{
    const char* PanelMode;

    FoundG2Panel = false;
    FoundG2V2Panel = false;
    PanelMode = GetFrontPanelModeOverride();

    if((strcmp(PanelMode, "off") == 0) || (strcmp(PanelMode, "none") == 0))
    {
        printf("Front panel disabled by SATURN_FRONT_PANEL_MODE=%s\n", PanelMode);
        return;
    }

    if(strcmp(PanelMode, "g2v2") == 0)
    {
        if(CheckG2V2PanelPresent())
        {
            printf("Forced front panel mode: g2v2\n");
            printf("Found serial device interfaced panel\n");
            FoundG2V2Panel = true;
            InitialiseG2V2PanelHandler();
        }
        else
            printf("SATURN_FRONT_PANEL_MODE=g2v2 but no G2V2 panel detected\n");
        return;
    }

    if(strcmp(PanelMode, "g2") == 0)
    {
        if(CheckG2PanelPresent())
        {
            printf("Forced front panel mode: g2\n");
            printf("Found G2 front panel\n");
            FoundG2Panel = true;
            InitialiseG2PanelHandler();
        }
        else
            printf("SATURN_FRONT_PANEL_MODE=g2 but no G2 panel detected\n");
        return;
    }

    if((strcmp(PanelMode, "prefer-g2") == 0) || (strcmp(PanelMode, "prefer_g2") == 0))
    {
        printf("Front panel detection mode: prefer-g2\n");
        if(CheckG2PanelPresent())
        {
            printf("Found G2 front panel\n");
            FoundG2Panel = true;
            InitialiseG2PanelHandler();
        }
        else if(CheckG2V2PanelPresent())
        {
            printf("Found serial device interfaced panel\n");
            FoundG2V2Panel = true;
            InitialiseG2V2PanelHandler();
        }
        else
            printf("No front panel found\n");
        return;
    }

    if((strcmp(PanelMode, "prefer-g2v2") == 0) || (strcmp(PanelMode, "prefer_g2v2") == 0))
    {
        printf("Front panel detection mode: prefer-g2v2\n");
        InitialiseFrontPanelAuto();
        return;
    }

    if(strcmp(PanelMode, "auto") != 0)
        printf("Unknown SATURN_FRONT_PANEL_MODE=%s, using auto detect\n", PanelMode);
    InitialiseFrontPanelAuto();
}


//
// function to shutdown a connection to the front panel; call if selected as a command line option
// establish which if any front panel is attached, and close it down.
//
void ShutdownFrontPanelHandler(void)
{
    if(FoundG2Panel)
    {
        ShutdownG2PanelHandler();
    }
    else if (FoundG2V2Panel)
    {
        ShutdownG2V2PanelHandler();
    }
}

