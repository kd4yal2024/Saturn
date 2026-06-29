# Makefile for p1app
# *****************************************************
# Variables to control Makefile operation
 
CC = gcc
LD = gcc
CFLAGS = -Wall -Wextra -Wno-unused-function -O2 -g -D_GNU_SOURCE \
         -Wformat-security -fstack-protector-strong -D_FORTIFY_SOURCE=2
LDFLAGS = -lm -lpthread
TARGET = p1app
VPATH=.:../common
 
# ****************************************************
# Targets needed to bring the executable up to date

OBJS=    $(TARGET).o hwaccess.o saturnregisters.o codecwrite.o version.o

all: $(OBJS)
	$(LD) -o $(TARGET) $(OBJS) $(LDFLAGS)
 
 
%.o: %.c
	$(CC) -c -o $(@F) $(CFLAGS) $<

clean:
	rm -rf $(TARGET) *.o *.bin
